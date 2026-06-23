//! Public, UI-facing types plus the raw JSON shapes returned by the internal
//! Teams APIs. The raw shapes are intentionally permissive (everything optional)
//! because these endpoints are undocumented and fields vary between accounts.

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
    #[serde(rename = "lastMessage", default)]
    pub last_message: Option<RawLastMessage>,
}

#[derive(Debug, Deserialize)]
pub struct RawMember {
    #[serde(default)]
    pub mri: Option<String>,
}

/// The conversation list embeds the last message; it carries the sender's
/// display name (camelCase here, unlike the messages endpoint) and MRI.
#[derive(Debug, Deserialize)]
pub struct RawLastMessage {
    #[serde(rename = "imDisplayName", default)]
    pub im_display_name: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
}

impl ConversationsResponse {
    /// Reduce the raw response to sidebar rows. `me_mri` identifies the signed-in
    /// user so 1:1 chats can be named after the *other* party and self-chats
    /// detected.
    pub fn into_summaries(self, me_mri: Option<&str>) -> Vec<ChatSummary> {
        self.chats
            .into_iter()
            .map(|c| {
                let label = chat_label(&c, me_mri);
                ChatSummary { id: c.id, label }
            })
            .collect()
    }
}

fn chat_label(chat: &RawChat, me_mri: Option<&str>) -> String {
    // A named group/topic chat: use the title.
    if let Some(title) = &chat.title {
        if !title.trim().is_empty() {
            return title.clone();
        }
    }

    // Self-chat: every member is me (or the only member is me).
    let has_other = chat
        .members
        .iter()
        .filter_map(|m| m.mri.as_deref())
        .any(|mri| Some(mri) != me_mri);
    if !chat.members.is_empty() && !has_other {
        return "Notes to self".to_string();
    }

    // Best available name: the last message's sender, when it isn't me — for a
    // 1:1 that is the other person. (Full per-member name resolution needs the
    // profile endpoint; see roadmap.)
    if let Some(lm) = &chat.last_message {
        if let Some(name) = &lm.im_display_name {
            if !name.trim().is_empty() && lm.from.as_deref() != me_mri {
                return name.clone();
            }
        }
    }

    if chat.is_one_on_one || chat.members.len() == 2 {
        "Direct message".to_string()
    } else {
        "Group chat".to_string()
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

    fn member(mri: &str) -> RawMember {
        RawMember {
            mri: Some(mri.into()),
        }
    }

    const ME: &str = "8:orgid:me";

    #[test]
    fn chat_label_prefers_title() {
        let c = RawChat {
            id: "c1".into(),
            title: Some("Project X".into()),
            members: vec![member(ME), member("8:orgid:x")],
            is_one_on_one: false,
            last_message: None,
        };
        assert_eq!(chat_label(&c, Some(ME)), "Project X");
    }

    #[test]
    fn chat_label_uses_other_party_from_last_message() {
        let c = RawChat {
            id: "c2".into(),
            title: None,
            members: vec![member(ME), member("8:orgid:x")],
            is_one_on_one: true,
            last_message: Some(RawLastMessage {
                im_display_name: Some("Carol".into()),
                from: Some("8:orgid:x".into()),
            }),
        };
        assert_eq!(chat_label(&c, Some(ME)), "Carol");
    }

    #[test]
    fn chat_label_ignores_my_own_last_message() {
        let c = RawChat {
            id: "c3".into(),
            title: None,
            members: vec![member(ME), member("8:orgid:x")],
            is_one_on_one: true,
            last_message: Some(RawLastMessage {
                im_display_name: Some("Me".into()),
                from: Some(ME.into()),
            }),
        };
        assert_eq!(chat_label(&c, Some(ME)), "Direct message");
    }

    #[test]
    fn chat_label_detects_self_chat() {
        let c = RawChat {
            id: "c4".into(),
            title: None,
            members: vec![member(ME)],
            is_one_on_one: false,
            last_message: None,
        };
        assert_eq!(chat_label(&c, Some(ME)), "Notes to self");
    }
}
