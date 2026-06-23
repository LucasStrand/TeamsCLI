//! OAuth2 device authorization grant (v1 endpoint) against the Microsoft
//! identity platform, plus refresh-token exchange. Uses the resource-based v1
//! flow because the Teams internal APIs expect a token whose audience is the
//! Skype/Spaces resource.

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::config::{self, Config};

use super::token_store::TokenSet;

/// Accepts a JSON number whether it is encoded as an integer or a string (the
/// v1 endpoints return some numeric fields as strings).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum FlexNum {
    Int(i64),
    Str(String),
}

impl FlexNum {
    fn as_i64(&self) -> i64 {
        match self {
            FlexNum::Int(i) => *i,
            FlexNum::Str(s) => s.parse().unwrap_or(0),
        }
    }
}

/// Response from the v1 `/devicecode` endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    /// v1 returns `verification_url` (note: not `verification_uri`).
    pub verification_url: String,
    expires_in: FlexNum,
    interval: FlexNum,
    #[allow(dead_code)]
    pub message: String,
}

/// v1 token endpoint success payload.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: FlexNum,
}

/// v1 token endpoint error payload.
#[derive(Debug, Deserialize)]
struct TokenError {
    error: String,
    #[serde(default)]
    error_description: String,
}

/// Run the full device-code flow: request a code, surface it to the user via
/// `prompt`, then poll until the user authorizes (or the request expires).
pub async fn device_code_flow(
    cfg: &Config,
    http: &reqwest::Client,
    prompt: impl Fn(&DeviceCodeResponse),
) -> Result<TokenSet> {
    let dc: DeviceCodeResponse = http
        .post(config::DEVICECODE_URL)
        .form(&[
            ("client_id", cfg.client_id.as_str()),
            ("resource", config::SKYPE_RESOURCE),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    prompt(&dc);

    let mut interval = Duration::from_secs(dc.interval.as_i64().max(1) as u64);
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(dc.expires_in.as_i64().max(0) as u64);

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "device code expired before authorization completed"
            ));
        }
        tokio::time::sleep(interval).await;

        let resp = http
            .post(config::TOKEN_URL)
            .form(&[
                ("grant_type", "device_code"),
                ("client_id", cfg.client_id.as_str()),
                ("resource", config::SKYPE_RESOURCE),
                ("code", dc.device_code.as_str()),
            ])
            .send()
            .await?;

        if resp.status().is_success() {
            let tr: TokenResponse = resp.json().await?;
            return Ok(TokenSet::from_response(
                tr.access_token,
                tr.refresh_token,
                tr.expires_in.as_i64(),
            ));
        }

        let err: TokenError = resp.json().await?;
        match err.error.as_str() {
            "authorization_pending" => continue,
            "slow_down" => {
                interval += Duration::from_secs(5);
                continue;
            }
            "code_expired" | "expired_token" => {
                return Err(anyhow!("device code expired; please try signing in again"))
            }
            "authorization_declined" => return Err(anyhow!("sign-in was declined")),
            other => {
                return Err(anyhow!(
                    "token request failed ({other}): {}",
                    err.error_description
                ))
            }
        }
    }
}

/// Exchange a refresh token for a fresh access (and refresh) token for the given
/// resource. v1 refresh tokens are multi-resource, so one refresh token can be
/// redeemed for different resources (Skype, chatsvcagg, …).
pub async fn refresh(
    cfg: &Config,
    http: &reqwest::Client,
    refresh_token: &str,
    resource: &str,
) -> Result<TokenSet> {
    let resp = http
        .post(config::TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", cfg.client_id.as_str()),
            ("resource", resource),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await?;

    if resp.status().is_success() {
        let tr: TokenResponse = resp.json().await?;
        let refresh = tr.refresh_token.or_else(|| Some(refresh_token.to_string()));
        return Ok(TokenSet::from_response(
            tr.access_token,
            refresh,
            tr.expires_in.as_i64(),
        ));
    }

    let err: TokenError = resp.json().await?;
    Err(anyhow!(
        "refresh failed ({}): {}",
        err.error,
        err.error_description
    ))
}
