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
        let bytes = self
            .send(
                reqwest::Method::GET,
                &url,
                None,
                config::CHATSVCAGG_RESOURCE,
            )
            .await?;
        // Log the raw payload (debug only) so chat-label field mapping can be
        // verified/tuned against real responses. Enable with TEAMSCLI_LOG=debug.
        tracing::debug!(
            "conversations raw response: {}",
            String::from_utf8_lossy(&bytes)
        );
        let resp: ConversationsResponse = serde_json::from_slice(&bytes)?;

        // Resolve member MRIs to display names so 1:1 and group chats are named
        // after their participants.
        let me = self.me_mri().map(|s| s.to_string());
        let names = self.resolve_names(&resp.member_mris(me.as_deref())).await;
        Ok(resp.into_summaries(me.as_deref(), &names))
    }
}
