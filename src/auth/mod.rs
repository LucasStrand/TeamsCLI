//! Authentication: OAuth2 device-code flow with a persisted refresh token for
//! silent re-login. Authenticates as the first-party Teams client, so no Azure
//! app registration or admin consent is needed.

pub mod device_code;
pub mod token_store;

use anyhow::Result;

use crate::config::{self, Config};
use token_store::TokenSet;

/// Obtain a valid access token for the Skype/Spaces resource, preferring a
/// silent refresh (from the cached refresh token) and falling back to the
/// interactive device-code flow.
///
/// The `prompt` callback is invoked with the human-readable instructions
/// (user code + verification URL) when interactive sign-in is required.
pub async fn authenticate(
    cfg: &Config,
    http: &reqwest::Client,
    prompt: impl Fn(&device_code::DeviceCodeResponse),
) -> Result<TokenSet> {
    // 1. Try a silent refresh from the cached refresh token.
    if let Some(refresh) = token_store::load_refresh(&cfg.token_cache_path) {
        match device_code::refresh(cfg, http, &refresh, config::SKYPE_RESOURCE).await {
            Ok(tokens) => {
                tokens.save_refresh(&cfg.token_cache_path)?;
                return Ok(tokens);
            }
            Err(e) => {
                tracing::warn!("silent refresh failed, falling back to device code: {e}");
            }
        }
    }

    // 2. Interactive device-code flow.
    let tokens = device_code::device_code_flow(cfg, http, prompt).await?;
    tokens.save_refresh(&cfg.token_cache_path)?;
    Ok(tokens)
}
