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
    /// Unique member MRIs across all chats, excluding the signed-in user, for a
    /// batched name lookup.
    pub fn member_mris(&self, me_mri: Option<&str>) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for chat in &self.chats {
            for m in &chat.members {
                if let Some(mri) = &m.mri {
                    if Some(mri.as_str()) != me_mri && seen.insert(mri.clone()) {
                        out.push(mri.clone());
                    }
                }
            }
        }
        out
    }

    /// Reduce the raw response to sidebar rows. `me_mri` identifies the signed-in
    /// user; `names` maps member MRIs to display names (from the profile lookup).
    pub fn into_summaries(
        self,
        me_mri: Option<&str>,
        names: &HashMap<String, String>,
    ) -> Vec<ChatSummary> {
        self.chats
            .into_iter()
            .map(|c| {
                let label = chat_label(&c, me_mri, names);
                ChatSummary { id: c.id, label }
            })
            .collect()
    }
}

fn chat_label(chat: &RawChat, me_mri: Option<&str>, names: &HashMap<String, String>) -> String {
    // A named group/topic chat: the custom title wins.
    if let Some(title) = &chat.title {
        if !title.trim().is_empty() {
            return title.clone();
        }
    }

    // Members other than me.
    let others: Vec<&str> = chat
        .members
        .iter()
        .filter_map(|m| m.mri.as_deref())
        .filter(|mri| Some(*mri) != me_mri)
        .collect();

    // Self-chat: only me in the roster.
    if !chat.members.is_empty() && others.is_empty() {
        return "Notes to self".to_string();
    }

    // Name the chat after the resolved member display names: for a 1:1 this is
    // the single other person; for a group it's "name1, name2, …".
    let resolved: Vec<String> = others
        .iter()
        .filter_map(|mri| names.get(*mri).cloned())
        .collect();
    if !resolved.is_empty() {
        return join_names(&resolved);
    }

    // Fallback when the lookup returned nothing: the last sender, if not me.
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

/// Join member names, capping long group rosters as "a, b, c +N".
fn join_names(names: &[String]) -> String {
    const CAP: usize = 4;
    if names.len() <= CAP {
        return names.join(", ");
    }
    let shown = names[..CAP].join(", ");
    format!("{shown} +{}", names.len() - CAP)
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

    fn names(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn chat_label_prefers_title() {
        let c = RawChat {
            id: "c1".into(),
            title: Some("Project X".into()),
            members: vec![member(ME), member("8:orgid:x")],
            is_one_on_one: false,
            last_message: None,
        };
        assert_eq!(chat_label(&c, Some(ME), &names(&[])), "Project X");
    }

    #[test]
    fn chat_label_one_on_one_uses_other_member_name() {
        let c = RawChat {
            id: "c2".into(),
            title: None,
            members: vec![member(ME), member("8:orgid:x")],
            is_one_on_one: true,
            last_message: None,
        };
        let map = names(&[("8:orgid:x", "Carol Smith")]);
        assert_eq!(chat_label(&c, Some(ME), &map), "Carol Smith");
    }

    #[test]
    fn chat_label_group_joins_member_names() {
        let c = RawChat {
            id: "c3".into(),
            title: None,
            members: vec![member(ME), member("8:orgid:a"), member("8:orgid:b")],
            is_one_on_one: false,
            last_message: None,
        };
        let map = names(&[("8:orgid:a", "Ann"), ("8:orgid:b", "Bob")]);
        assert_eq!(chat_label(&c, Some(ME), &map), "Ann, Bob");
    }

    #[test]
    fn chat_label_falls_back_to_last_sender() {
        let c = RawChat {
            id: "c4".into(),
            title: None,
            members: vec![member(ME), member("8:orgid:x")],
            is_one_on_one: true,
            last_message: Some(RawLastMessage {
                im_display_name: Some("Carol".into()),
                from: Some("8:orgid:x".into()),
            }),
        };
        // No resolved name, but the other party sent the last message.
        assert_eq!(chat_label(&c, Some(ME), &names(&[])), "Carol");
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
        assert_eq!(chat_label(&c, Some(ME), &names(&[])), "Notes to self");
    }
}
