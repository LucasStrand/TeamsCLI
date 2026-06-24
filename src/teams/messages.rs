//! Reading and sending messages via the region-specific messaging host.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::Serialize;

use crate::config;

use super::models::{Message, MessagesResponse};
use super::TeamsClient;

impl TeamsClient {
    /// Fetch recent messages for a conversation, oldest-first after filtering.
    pub async fn get_messages(&self, conversation_id: &str) -> Result<Vec<Message>> {
        let host = self.messaging_host().await?;
        let id = urlencoding::encode(conversation_id);
        let page_size = self.message_limit();
        let url = format!(
            "{host}/v1/users/ME/conversations/{id}/messages\
             ?view=msnp24Equivalent%7CsupportsMessageProperties&pageSize={page_size}&startTime=1"
        );
        // Messaging host uses the skypetoken; the bearer resource is unused.
        let resp: MessagesResponse = self.get_json(&url, config::SKYPE_RESOURCE).await?;
        // Learn sender names from history so chats whose profiles don't resolve
        // still get named (the cache is reused when labeling the chat list).
        self.merge_names(resp.name_pairs()).await;
        let me = self.me_mri().map(|s| s.to_string());
        Ok(resp.into_messages(me.as_deref()))
    }

    /// Send a plain-text message to a conversation.
    pub async fn send_message(&self, conversation_id: &str, text: &str) -> Result<()> {
        let host = self.messaging_host().await?;
        let id = urlencoding::encode(conversation_id);
        let url = format!("{host}/v1/users/ME/conversations/{id}/messages");

        let body = OutgoingMessage {
            content: html_escape(text),
            messagetype: "RichText/Html",
            contenttype: "text",
            clientmessageid: new_client_message_id(),
        };
        let bytes = serde_json::to_vec(&body)?;
        self.post_bytes(&url, bytes, config::SKYPE_RESOURCE).await?;
        Ok(())
    }
}

#[derive(Serialize)]
struct OutgoingMessage {
    content: String,
    messagetype: &'static str,
    contenttype: &'static str,
    clientmessageid: String,
}

/// A client-generated message id (Teams expects a large numeric string).
fn new_client_message_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    nanos.to_string()
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
