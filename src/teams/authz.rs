//! Exchange a Skype-resource bearer token for a skypetoken via the Teams authz
//! endpoint, and determine the region-specific messaging host.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

use crate::config;

/// Result of an authz exchange: the skypetoken and where to send messaging calls.
#[derive(Debug, Clone)]
pub struct SkypeAuth {
    pub skype_token: String,
    pub expires_at: DateTime<Utc>,
    /// Region-specific messaging host, e.g. `https://emea.ng.msg.teams.microsoft.com`.
    pub messaging_host: String,
    /// Region code (e.g. `emea`) used in middle-tier endpoint paths.
    pub region: String,
}

impl SkypeAuth {
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }
}

#[derive(Debug, Deserialize)]
struct AuthzResponse {
    tokens: AuthzTokens,
    #[serde(rename = "regionGtms", default)]
    region_gtms: Option<serde_json::Value>,
    #[serde(default)]
    region: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthzTokens {
    #[serde(rename = "skypeToken")]
    skype_token: String,
    #[serde(rename = "expiresIn", default)]
    expires_in: i64,
}

/// POST the bearer token to the authz endpoint and parse the skypetoken.
pub async fn fetch_skype_auth(http: &reqwest::Client, bearer: &str) -> Result<SkypeAuth> {
    let resp = http
        .post(config::AUTHZ_URL)
        .bearer_auth(bearer)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body("{}")
        .send()
        .await
        .context("calling Teams authz endpoint")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("skypetoken exchange failed ({status}): {body}"));
    }

    let parsed: AuthzResponse = resp.json().await.context("parsing authz response")?;

    let ttl = if parsed.tokens.expires_in > 0 {
        parsed.tokens.expires_in - 60
    } else {
        // Skypetokens are typically valid ~24h; be conservative if unspecified.
        60 * 60 * 12
    };

    let messaging_host = messaging_host_from(&parsed);
    let region = region_from(&parsed, &messaging_host);

    Ok(SkypeAuth {
        skype_token: parsed.tokens.skype_token,
        expires_at: Utc::now() + Duration::seconds(ttl.max(0)),
        messaging_host,
        region,
    })
}

/// The region code (lowercase), preferring the explicit `region` field and
/// falling back to the messaging host's subdomain (e.g. `emea` from
/// `https://emea.ng.msg.teams.microsoft.com`).
fn region_from(resp: &AuthzResponse, messaging_host: &str) -> String {
    if let Some(region) = &resp.region {
        let r = region.trim().to_lowercase();
        if !r.is_empty() {
            return r;
        }
    }
    messaging_host
        .strip_prefix("https://")
        .and_then(|h| h.split('.').next())
        .unwrap_or("emea")
        .to_string()
}

/// Determine the messaging host. Prefer scanning `regionGtms` for the actual
/// `*.ng.msg.teams.microsoft.com` URL (robust to key-name changes); otherwise
/// build it from the `region` string; otherwise fall back to a default.
fn messaging_host_from(resp: &AuthzResponse) -> String {
    if let Some(gtms) = &resp.region_gtms {
        if let Some(host) = find_messaging_host(&gtms.to_string()) {
            return host;
        }
    }
    if let Some(region) = &resp.region {
        let r = region.trim().to_lowercase();
        if !r.is_empty() {
            return format!("https://{r}.ng.msg.teams.microsoft.com");
        }
    }
    config::DEFAULT_MESSAGING_HOST.to_string()
}

/// Find the first `https://<sub>.ng.msg.teams.microsoft.com` URL inside a blob
/// of JSON text and return its scheme+host.
fn find_messaging_host(blob: &str) -> Option<String> {
    const MARKER: &str = ".ng.msg.teams.microsoft.com";
    let marker_idx = blob.find(MARKER)?;
    // Walk back to the start of the URL ("https://").
    let prefix = "https://";
    let start = blob[..marker_idx].rfind(prefix)?;
    let host_end = marker_idx + MARKER.len();
    Some(blob[start..host_end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_messaging_host_from_gtms_blob() {
        let blob = r#"{"chatService":"https://teams.microsoft.com/api/csa","messaging":"https://emea.ng.msg.teams.microsoft.com/v1"}"#;
        assert_eq!(
            find_messaging_host(blob).as_deref(),
            Some("https://emea.ng.msg.teams.microsoft.com")
        );
    }

    #[test]
    fn no_host_when_absent() {
        assert_eq!(find_messaging_host("{\"x\":\"y\"}"), None);
    }
}
