//! Public, UI-facing types plus the raw JSON shapes returned by the internal
//! Teams APIs. The raw shapes are intentionally permissive (everything optional)
//! because these endpoints are undocumented and fields vary between accounts.

use std::collections::HashMap;

use serde::Deserialize;

use crate::util::strip_html;

/// A chat row shown in the sidebar.
#[derive(Debug, Clone)]
pub struct ChatSummary {
    pub id: String,
    pub label: String,
}

/// A single rendered message.
#[derive(Debug, Clone)]
pub struct Message {
    pub id: String,
    pub sender: String,
    pub text: String,
    /// ISO-8601 timestamp, used for ordering and display.
    pub created: Option<String>,
}

// ----------------------------------------------------------------------------
// Raw conversation-list shapes (CSA `/teams/users/me`).
// ----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ConversationsResponse {
    #[serde(default)]
    pub chats: Vec<RawChat>,
    #[serde(default)]
    pub users: Vec<RawUser>,
}

#[derive(Debug, Deserialize)]
pub struct RawChat {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub members: Vec<RawMember>,
    #[serde(rename = "isOneOnOne", default)]
    pub is_one_on_one: bool,
    #[serde(rename = "lastMessagePreview", default)]
    pub last_message_preview: Option<RawLastMessage>,
}

#[derive(Debug, Deserialize)]
pub struct RawMember {
    #[serde(default)]
    pub mri: Option<String>,
    #[serde(rename = "friendlyName", default)]
    pub friendly_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RawUser {
    #[serde(default)]
    pub mri: Option<String>,
    #[serde(rename = "displayName", default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RawLastMessage {
    #[serde(rename = "imdisplayname", default)]
    pub im_display_name: Option<String>,
}

impl ConversationsResponse {
    /// Reduce the raw response to sidebar rows, resolving member names from the
    /// `users` table where possible.
    pub fn into_summaries(self) -> Vec<ChatSummary> {
        let names: HashMap<String, String> = self
            .users
            .into_iter()
            .filter_map(|u| Some((u.mri?, u.display_name?)))
            .collect();

        self.chats
            .into_iter()
            .map(|c| {
                let label = chat_label(&c, &names);
                ChatSummary { id: c.id, label }
            })
            .collect()
    }
}

fn chat_label(chat: &RawChat, names: &HashMap<String, String>) -> String {
    if let Some(title) = &chat.title {
        if !title.trim().is_empty() {
            return title.clone();
        }
    }
    // Derive from member names (resolved via the users table, else friendlyName).
    let mut member_names: Vec<String> = chat
        .members
        .iter()
        .filter_map(|m| {
            let mri = m.mri.as_deref();
            mri.and_then(|mri| names.get(mri).cloned())
                .or_else(|| m.friendly_name.clone())
        })
        .filter(|s| !s.trim().is_empty())
        .collect();
    member_names.dedup();
    if !member_names.is_empty() {
        return member_names.join(", ");
    }
    if let Some(preview) = &chat.last_message_preview {
        if let Some(name) = &preview.im_display_name {
            if !name.trim().is_empty() {
                return name.clone();
            }
        }
    }
    if chat.is_one_on_one {
        "Direct message".to_string()
    } else {
        "Chat".to_string()
    }
}

// ----------------------------------------------------------------------------
// Raw message shapes (messaging host `/conversations/{id}/messages`).
// ----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct MessagesResponse {
    #[serde(default)]
    pub messages: Vec<RawMessage>,
}

#[derive(Debug, Deserialize)]
pub struct RawMessage {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "messagetype", default)]
    pub message_type: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(rename = "imdisplayname", default)]
    pub im_display_name: Option<String>,
    #[serde(default)]
    pub composetime: Option<String>,
    #[serde(default)]
    pub originalarrivaltime: Option<String>,
}

impl MessagesResponse {
    /// Keep only human chat messages (drop system/thread-activity events) and
    /// project them into the UI `Message` type.
    pub fn into_messages(self) -> Vec<Message> {
        self.messages
            .into_iter()
            .filter_map(|m| m.into_message())
            .collect()
    }
}

impl RawMessage {
    fn into_message(self) -> Option<Message> {
        let id = self.id?;
        let mtype = self.message_type.unwrap_or_default();
        // Only render text/rich-text chat messages.
        if !(mtype.starts_with("RichText") || mtype == "Text") {
            return None;
        }
        let raw = self.content.unwrap_or_default();
        let text = strip_html(&raw);
        if text.is_empty() {
            return None;
        }
        Some(Message {
            id,
            sender: self
                .im_display_name
                .unwrap_or_else(|| "Unknown".to_string()),
            text,
            created: self.originalarrivaltime.or(self.composetime),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_messages_and_filters_system_events() {
        let json = r#"{
            "messages": [
                {"id":"1","messagetype":"RichText/Html","content":"<div>hi</div>","imdisplayname":"Ann","originalarrivaltime":"2021-04-11T09:31:49Z"},
                {"id":"2","messagetype":"ThreadActivity/AddMember","content":"<systemmessage/>"},
                {"id":"3","messagetype":"Text","content":"yo","imdisplayname":"Bob"}
            ]
        }"#;
        let resp: MessagesResponse = serde_json::from_str(json).unwrap();
        let msgs = resp.into_messages();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].sender, "Ann");
        assert_eq!(msgs[0].text, "hi");
        assert_eq!(msgs[1].text, "yo");
    }

    #[test]
    fn chat_label_prefers_title_then_members() {
        let names: HashMap<String, String> = [("8:orgid:x".to_string(), "Carol".to_string())]
            .into_iter()
            .collect();

        let titled = RawChat {
            id: "c1".into(),
            title: Some("Project X".into()),
            members: vec![],
            is_one_on_one: false,
            last_message_preview: None,
        };
        assert_eq!(chat_label(&titled, &names), "Project X");

        let by_member = RawChat {
            id: "c2".into(),
            title: None,
            members: vec![RawMember {
                mri: Some("8:orgid:x".into()),
                friendly_name: None,
            }],
            is_one_on_one: true,
            last_message_preview: None,
        };
        assert_eq!(chat_label(&by_member, &names), "Carol");
    }
}
