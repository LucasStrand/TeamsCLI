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
        let resp: ConversationsResponse = self.get_json(&url).await?;
        Ok(resp.into_summaries())
    }
}
