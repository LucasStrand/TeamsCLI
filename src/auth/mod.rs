//! Authentication: OAuth2 device-code flow with a persisted refresh token for
//! silent re-login. Authenticates as the first-party Teams client, so no Azure
//! app registration or admin consent is needed.

pub mod device_code;
pub mod token_store;

use anyhow::Result;

use crate::config::Config;
use token_store::TokenSet;

/// Obtain a valid access token, preferring a silent refresh and falling back to
/// the interactive device-code flow.
///
/// The `prompt` callback is invoked with the human-readable instructions
/// (user code + verification URL) when interactive sign-in is required.
pub async fn authenticate(
    cfg: &Config,
    http: &reqwest::Client,
    prompt: impl Fn(&device_code::DeviceCodeResponse),
) -> Result<TokenSet> {
    // 1. Try the cached token set.
    if let Some(tokens) = TokenSet::load(&cfg.token_cache_path) {
        if !tokens.is_access_expired() {
            return Ok(tokens);
        }
        // 2. Access token expired — try a silent refresh.
        if let Some(refresh) = tokens.refresh_token.clone() {
            match device_code::refresh(cfg, http, &refresh, crate::config::SKYPE_RESOURCE).await {
                Ok(refreshed) => {
                    refreshed.save(&cfg.token_cache_path)?;
                    return Ok(refreshed);
                }
                Err(e) => {
                    tracing::warn!("silent refresh failed, falling back to device code: {e}");
                }
            }
        }
    }

    // 3. Interactive device-code flow.
    let tokens = device_code::device_code_flow(cfg, http, prompt).await?;
    tokens.save(&cfg.token_cache_path)?;
    Ok(tokens)
}
