//! Listing the user's chats via the chat-service aggregator (CSA).

use anyhow::Result;

use crate::config;

use super::models::{ChatSummary, ConversationsResponse};
use super::TeamsClient;

impl TeamsClient {
    /// List the user's chats (1:1 and group). Teams/channels are returned by the
    /// same endpoint but are handled in a later phase.
    pub async fn list_chats(&self) -> Result<Vec<ChatSummary>> {
        let url = format!(
            "{}/teams/users/me?isPrefetch=false&enableMembershipSummary=true",
            config::CSA_BASE
        );
        let bytes = self.send(reqwest::Method::GET, &url, None).await?;
        // Log the raw payload (debug only) so chat-label field mapping can be
        // verified/tuned against real responses. Enable with TEAMSCLI_LOG=debug.
        tracing::debug!(
            "conversations raw response: {}",
            String::from_utf8_lossy(&bytes)
        );
        let resp: ConversationsResponse = serde_json::from_slice(&bytes)?;
        Ok(resp.into_summaries(self.me_mri()))
    }
}
