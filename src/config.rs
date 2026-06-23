//! Application configuration: Microsoft Teams internal API endpoints, the
//! first-party OAuth client used for sign-in, and on-disk paths.
//!
//! This client talks to Teams' **internal** (undocumented) API rather than
//! Microsoft Graph. It authenticates as the official Microsoft Teams first-party
//! application, which is pre-consented in every tenant — so no Azure app
//! registration and no admin consent are required. See the README for the
//! trade-offs (these endpoints are unofficial and can change without notice).

use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;

/// OAuth v1 device-code endpoint (multi-tenant + personal via `common`).
pub const DEVICECODE_URL: &str = "https://login.microsoftonline.com/common/oauth2/devicecode";
/// OAuth v1 token endpoint.
pub const TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/token";

/// Official Microsoft Teams first-party client ID (public client). Pre-consented
/// in all tenants, so signing in with it does not require admin approval.
pub const TEAMS_CLIENT_ID: &str = "1fec8e78-bce4-4aaf-ab1b-5451cc387264";

/// Resource (audience) for the initial access token. The token for this resource
/// is what the Teams authz endpoint exchanges for a skypetoken.
pub const SKYPE_RESOURCE: &str = "https://api.spaces.skype.com";

/// Resource (audience) required by the chat-service aggregator (CSA) host.
/// CSA requests must carry a bearer token for *this* resource, not the Skype one.
pub const CHATSVCAGG_RESOURCE: &str = "https://chatsvcagg.teams.microsoft.com";

/// Exchanges a Skype-resource bearer token for a skypetoken + region info.
pub const AUTHZ_URL: &str = "https://teams.microsoft.com/api/authsvc/v1.0/authz";

/// Chat service aggregator base — lists the user's chats and teams.
pub const CSA_BASE: &str = "https://teams.microsoft.com/api/csa/api/v1";

/// Middle-tier base — used to resolve user MRIs to display names. The region
/// segment is appended at runtime: `{MT_BASE}/{region}/beta/...`.
pub const MT_BASE: &str = "https://teams.microsoft.com/api/mt";

/// Fallback messaging host if the region cannot be determined from the authz
/// response. The real host is region-specific (e.g. emea/amer/apac).
pub const DEFAULT_MESSAGING_HOST: &str = "https://emea.ng.msg.teams.microsoft.com";

/// How often the background poller refreshes the active chat's messages.
pub const POLL_INTERVAL_SECS: u64 = 4;

#[derive(Debug, Clone)]
pub struct Config {
    pub client_id: String,
    pub token_cache_path: PathBuf,
    pub log_dir: PathBuf,
}

impl Config {
    pub fn load() -> Result<Self> {
        let dirs = ProjectDirs::from("dev", "teamscli", "teamscli")
            .context("could not determine application directories")?;

        let config_dir = dirs.config_dir().to_path_buf();
        let log_dir = dirs.data_dir().join("logs");

        // The first-party Teams client is the default; an override is supported
        // only for experimentation.
        let client_id =
            std::env::var("TEAMSCLI_CLIENT_ID").unwrap_or_else(|_| TEAMS_CLIENT_ID.to_string());

        Ok(Self {
            client_id,
            token_cache_path: config_dir.join("token.json"),
            log_dir,
        })
    }
}
