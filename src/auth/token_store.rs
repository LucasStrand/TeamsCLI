//! Persistent storage for OAuth tokens.
//!
//! The refresh token is the sensitive long-lived secret; it is written to a
//! user-only (0600) file under the config directory. (A future enhancement can
//! move this into the OS keychain via the `keyring` crate.)

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Absolute time at which the access token expires.
    pub expires_at: DateTime<Utc>,
}

impl TokenSet {
    /// Build a token set from a raw OAuth token response, computing the absolute
    /// expiry from `expires_in` (seconds) with a small safety margin.
    pub fn from_response(
        access_token: String,
        refresh_token: Option<String>,
        expires_in_secs: i64,
    ) -> Self {
        // Subtract a 60s margin so we refresh slightly before actual expiry.
        let ttl = (expires_in_secs - 60).max(0);
        Self {
            access_token,
            refresh_token,
            expires_at: Utc::now() + Duration::seconds(ttl),
        }
    }

    pub fn is_access_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    pub fn load(path: &Path) -> Option<Self> {
        let data = fs::read_to_string(path).ok()?;
        match serde_json::from_str(&data) {
            Ok(tokens) => Some(tokens),
            Err(e) => {
                tracing::warn!("ignoring unreadable token cache at {path:?}: {e}");
                None
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating config dir {parent:?}"))?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json).with_context(|| format!("writing token cache {path:?}"))?;
        restrict_permissions(path);
        Ok(())
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
        tracing::warn!("could not set 0600 permissions on {path:?}: {e}");
    }
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}
