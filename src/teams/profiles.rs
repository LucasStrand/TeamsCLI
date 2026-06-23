//! Resolve user MRIs to display names via the middle-tier `fetchShortProfile`
//! endpoint. Used to name 1:1 and group chats whose roster carries only MRIs.

use std::collections::HashMap;

use anyhow::Result;
use serde::Deserialize;

use crate::config;

use super::TeamsClient;

/// Max MRIs per fetchShortProfile request (the endpoint accepts a batch).
const BATCH: usize = 200;

#[derive(Debug, Deserialize)]
struct ShortProfileResponse {
    #[serde(default)]
    value: Vec<ShortProfile>,
}

#[derive(Debug, Deserialize)]
struct ShortProfile {
    #[serde(default)]
    mri: Option<String>,
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
}

impl TeamsClient {
    /// Resolve the given MRIs to display names. Missing/unknown MRIs are simply
    /// absent from the returned map; lookup failures are logged and yield an
    /// empty map so chat labels can still fall back gracefully.
    pub async fn resolve_names(&self, mris: &[String]) -> HashMap<String, String> {
        let mut names = HashMap::new();
        if mris.is_empty() {
            return names;
        }
        let region = match self.region().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("could not determine region for profile lookup: {e}");
                return names;
            }
        };
        let url = format!(
            "{}/{region}/beta/users/fetchShortProfile\
             ?isMailAddress=false&enableGuest=true&includeIBBarredUsers=false&skypeTeamsInfo=true",
            config::MT_BASE
        );

        for chunk in mris.chunks(BATCH) {
            match self.fetch_short_profiles(&url, chunk).await {
                Ok(profiles) => {
                    for p in profiles {
                        if let (Some(mri), Some(name)) = (p.mri, p.display_name) {
                            if !name.trim().is_empty() {
                                names.insert(mri, name);
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!("profile lookup failed: {e}"),
            }
        }
        names
    }

    async fn fetch_short_profiles(&self, url: &str, mris: &[String]) -> Result<Vec<ShortProfile>> {
        let body = serde_json::to_vec(mris)?;
        // The middle tier expects a token for the Skype/Spaces resource.
        let bytes = self.post_bytes(url, body, config::SKYPE_RESOURCE).await?;
        tracing::debug!(
            "fetchShortProfile raw response: {}",
            String::from_utf8_lossy(&bytes)
        );
        let resp: ShortProfileResponse = serde_json::from_slice(&bytes)?;
        Ok(resp.value)
    }
}
