//! OAuth tokens.
//!
//! Only the **refresh token** is persisted to disk (user-only `0600` file). It
//! is multi-resource, so every per-resource access token is derived from it at
//! runtime. Access tokens are never written to disk — persisting them caused
//! audience confusion (a `chatsvcagg` token being reused as the Skype token).

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// In-memory access token for a single resource, plus the refresh token.
#[derive(Debug, Clone)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Absolute time at which the access token expires.
    pub expires_at: DateTime<Utc>,
}

/// On-disk shape: just the refresh token.
#[derive(Debug, Serialize, Deserialize)]
struct RefreshStore {
    refresh_token: String,
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

    /// Persist this token set's refresh token (if any) for silent re-login.
    pub fn save_refresh(&self, path: &Path) -> Result<()> {
        if let Some(rt) = &self.refresh_token {
            save_refresh(path, rt)?;
        }
        Ok(())
    }
}

/// Read the cached refresh token, if present and readable.
pub fn load_refresh(path: &Path) -> Option<String> {
    let data = fs::read_to_string(path).ok()?;
    match serde_json::from_str::<RefreshStore>(&data) {
        Ok(store) if !store.refresh_token.is_empty() => Some(store.refresh_token),
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("ignoring unreadable token cache at {path:?}: {e}");
            None
        }
    }
}

/// Write the refresh token to a user-only file.
pub fn save_refresh(path: &Path, refresh_token: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating config dir {parent:?}"))?;
    }
    let json = serde_json::to_string_pretty(&RefreshStore {
        refresh_token: refresh_token.to_string(),
    })?;
    fs::write(path, json).with_context(|| format!("writing token cache {path:?}"))?;
    restrict_permissions(path);
    Ok(())
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
