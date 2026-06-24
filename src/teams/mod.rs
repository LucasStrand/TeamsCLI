//! Client for Microsoft Teams' internal (undocumented) API.
//!
//! Auth model: an AAD access token for the Skype/Spaces resource is exchanged
//! for a *skypetoken* (see [`authz`]). Requests to the chat-service aggregator
//! (CSA) carry the bearer token plus the skypetoken; requests to the
//! region-specific messaging host carry the skypetoken as the Authorization
//! header. Header routing is decided per-host in [`TeamsClient::send`].

mod authz;
pub mod conversations;
pub mod messages;
pub mod models;
pub mod people;
pub mod profiles;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

use crate::auth::device_code;
use crate::auth::token_store::TokenSet;
use crate::config::{self, Config};

use authz::SkypeAuth;

const MAX_RETRIES: u32 = 4;

#[derive(Clone)]
pub struct TeamsClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    cfg: Config,
    /// The multi-resource refresh token (redeemed per-resource on demand).
    refresh_token: Mutex<String>,
    /// Cached access tokens keyed by resource/audience.
    tokens: Mutex<HashMap<String, TokenSet>>,
    skype: Mutex<Option<SkypeAuth>>,
    /// The signed-in user's MRI, for naming 1:1 chats / detecting self-chats.
    me_mri: Option<String>,
    /// Accumulating MRI → display-name cache, grown from profile lookups,
    /// last-message senders, and opened message history.
    names: Mutex<HashMap<String, String>>,
}

impl TeamsClient {
    pub fn new(http: reqwest::Client, cfg: Config, tokens: TokenSet) -> Self {
        let refresh = tokens.refresh_token.clone().unwrap_or_default();
        let me_mri = crate::util::my_mri_from_token(&tokens.access_token);
        let mut map = HashMap::new();
        // The initial token was issued for the Skype/Spaces resource.
        map.insert(config::SKYPE_RESOURCE.to_string(), tokens);
        Self {
            inner: Arc::new(Inner {
                http,
                cfg,
                refresh_token: Mutex::new(refresh),
                tokens: Mutex::new(map),
                skype: Mutex::new(None),
                me_mri,
                names: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// The signed-in user's MRI, if it could be derived from the token.
    pub fn me_mri(&self) -> Option<&str> {
        self.inner.me_mri.as_deref()
    }

    /// Messages to request per conversation (from the CLI `--msg` flag).
    pub fn message_limit(&self) -> usize {
        self.inner.cfg.message_limit
    }

    /// Merge newly-learned (MRI → name) pairs into the shared name cache.
    pub async fn merge_names(&self, pairs: impl IntoIterator<Item = (String, String)>) {
        let mut cache = self.inner.names.lock().await;
        for (mri, name) in pairs {
            if !name.trim().is_empty() {
                cache.insert(mri, name);
            }
        }
    }

    /// A snapshot of the current name cache for labeling.
    pub async fn names_snapshot(&self) -> HashMap<String, String> {
        self.inner.names.lock().await.clone()
    }

    /// Valid AAD bearer token for `resource`, redeeming the refresh token if the
    /// cached token is missing or expired.
    async fn bearer(&self, resource: &str) -> Result<String> {
        {
            let map = self.inner.tokens.lock().await;
            if let Some(ts) = map.get(resource) {
                if !ts.is_access_expired() {
                    return Ok(ts.access_token.clone());
                }
            }
        }
        let refresh = self.inner.refresh_token.lock().await.clone();
        if refresh.is_empty() {
            return Err(anyhow!("no refresh token available; please sign in again"));
        }
        let ts =
            device_code::refresh(&self.inner.cfg, &self.inner.http, &refresh, resource).await?;
        // A rotated refresh token must be kept and persisted for silent re-login.
        // Only the refresh token is persisted — never per-resource access tokens.
        if let Some(new_refresh) = &ts.refresh_token {
            *self.inner.refresh_token.lock().await = new_refresh.clone();
            let _ = ts.save_refresh(&self.inner.cfg.token_cache_path);
        }
        let token = ts.access_token.clone();
        self.inner
            .tokens
            .lock()
            .await
            .insert(resource.to_string(), ts);
        Ok(token)
    }

    /// Valid skypetoken + messaging host, fetching/refreshing as needed. The
    /// authz exchange requires a token for the Skype/Spaces resource.
    async fn skype_auth(&self) -> Result<SkypeAuth> {
        {
            let guard = self.inner.skype.lock().await;
            if let Some(sa) = guard.as_ref() {
                if !sa.is_expired() {
                    return Ok(sa.clone());
                }
            }
        }
        let bearer = self.bearer(config::SKYPE_RESOURCE).await?;
        let sa = authz::fetch_skype_auth(&self.inner.http, &bearer).await?;
        *self.inner.skype.lock().await = Some(sa.clone());
        Ok(sa)
    }

    /// The region code used in middle-tier endpoint paths.
    async fn region(&self) -> Result<String> {
        Ok(self.skype_auth().await?.region)
    }

    /// Issue a request with host-appropriate auth headers and retry on
    /// throttling / transient errors. For non-messaging hosts the bearer token
    /// is issued for `bearer_resource` (CSA and the middle tier need different
    /// audiences); the messaging host uses the skypetoken only.
    async fn send(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<Vec<u8>>,
        bearer_resource: &str,
    ) -> Result<Vec<u8>> {
        let mut attempt = 0;
        loop {
            let sa = self.skype_auth().await?;
            let to_messaging = url.starts_with(&sa.messaging_host);
            let skype_header = format!("skypetoken={}", sa.skype_token);

            let mut req = self.inner.http.request(method.clone(), url);
            if to_messaging {
                // Messaging host authenticates with the skypetoken only.
                req = req
                    .header(reqwest::header::AUTHORIZATION, &skype_header)
                    .header("Authentication", &skype_header);
            } else {
                // CSA / middle-tier hosts need a bearer for the right resource
                // plus the skypetoken side-channel.
                let bearer = self.bearer(bearer_resource).await?;
                req = req
                    .bearer_auth(&bearer)
                    .header("Authentication", &skype_header);
            }
            if let Some(ref b) = body {
                req = req
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(b.clone());
            }

            let resp = req.send().await?;
            let status = resp.status();
            if status.is_success() {
                return Ok(resp.bytes().await?.to_vec());
            }

            let retryable =
                status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            if retryable && attempt < MAX_RETRIES {
                let wait = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(Duration::from_secs)
                    .unwrap_or_else(|| Duration::from_secs(1u64 << attempt));
                tracing::warn!("teams {status} on {url}, retrying in {wait:?}");
                tokio::time::sleep(wait).await;
                attempt += 1;
                continue;
            }

            let detail = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Teams request failed ({status}): {detail}"));
        }
    }

    async fn get_json<T: DeserializeOwned>(&self, url: &str, bearer_resource: &str) -> Result<T> {
        let bytes = self
            .send(reqwest::Method::GET, url, None, bearer_resource)
            .await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn post_bytes(&self, url: &str, body: Vec<u8>, bearer_resource: &str) -> Result<Vec<u8>> {
        self.send(reqwest::Method::POST, url, Some(body), bearer_resource)
            .await
    }

    /// POST to the messaging host (skypetoken auth) and return the `Location`
    /// response header — used by thread creation, where the new thread id is
    /// returned in `Location` rather than the body.
    async fn post_for_location(&self, url: &str, body: Vec<u8>) -> Result<Option<String>> {
        let sa = self.skype_auth().await?;
        let skype_header = format!("skypetoken={}", sa.skype_token);
        let resp = self
            .inner
            .http
            .post(url)
            .header(reqwest::header::AUTHORIZATION, &skype_header)
            .header("Authentication", &skype_header)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().await.unwrap_or_default();
            return Err(anyhow!("thread creation failed ({status}): {detail}"));
        }
        Ok(resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string()))
    }

    /// The region-specific messaging host (for building message URLs).
    async fn messaging_host(&self) -> Result<String> {
        Ok(self.skype_auth().await?.messaging_host)
    }
}
