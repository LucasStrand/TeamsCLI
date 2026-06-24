//! Public, UI-facing types plus the raw JSON shapes returned by the internal
//! Teams APIs. The raw shapes are intentionally permissive (everything optional)
//! because these endpoints are undocumented and fields vary between accounts.

use std::collections::HashMap;

use serde::Deserialize;

use crate::util::{render_message_html, strip_html};

/// What kind of conversation a sidebar row represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatKind {
    /// 1:1 chat.
    Dm,
    /// Group chat (3+ people).
    Group,
    /// Chat with only yourself ("Notes to self").
    SelfChat,
    /// A team channel.
    Channel,
}

impl ChatKind {
    pub fn is_channel(self) -> bool {
        matches!(self, ChatKind::Channel)
    }
}

/// A chat row shown in the sidebar.
#[derive(Debug, Clone)]
pub struct ChatSummary {
    pub id: String,
    pub label: String,
    pub kind: ChatKind,
    /// True when the conversation has unread messages.
    pub unread: bool,
    /// ISO-8601 timestamp of the last message, for recency sorting.
    pub last_activity: Option<String>,
    /// Short preview of the last message (used in notifications).
    pub last_preview: Option<String>,
    /// True when the last message was sent by the signed-in user.
    pub last_from_me: bool,
    /// Parent team name, for channels.
    pub team: Option<String>,
}

/// A single rendered message.
#[derive(Debug, Clone)]
pub struct Message {
    pub id: String,
    pub sender: String,
    pub text: String,
    /// ISO-8601 timestamp, used for ordering and display.
    pub created: Option<String>,
    /// True when the signed-in user sent this message.
    pub from_me: bool,
    /// Files, cards, and hosted images carried by the message.
    pub attachments: Vec<Attachment>,
}

/// What kind of non-text content a message carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    Image,
    File,
    Card,
}

/// A non-text item shown as a chip beneath the message (and, for images,
/// rendered inline in a later phase).
#[derive(Debug, Clone)]
pub struct Attachment {
    pub kind: AttachmentKind,
    /// Display label for the chip (file name / image name / card title).
    pub label: String,
    /// For images: the AMS object URL to fetch with the skypetoken. Captured now
    /// but only consumed in Phase 7 (inline image rendering).
    #[allow(dead_code)]
    pub url: Option<String>,
}

fn default_true() -> bool {
    true
}

// ----------------------------------------------------------------------------
// Raw conversation-list shapes (CSA `/teams/users/me`).
// ----------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct ConversationsResponse {
    #[serde(default)]
    pub chats: Vec<RawChat>,
    #[serde(default)]
    pub teams: Vec<RawTeam>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawChat {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub members: Vec<RawMember>,
    #[serde(rename = "isOneOnOne", default)]
    pub is_one_on_one: bool,
    // Absent → assume read, so we don't flag everything unread on first load.
    #[serde(rename = "isRead", default = "default_true")]
    pub is_read: bool,
    #[serde(rename = "lastMessage", default)]
    pub last_message: Option<RawLastMessage>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawMember {
    #[serde(default)]
    pub mri: Option<String>,
}

/// The conversation list embeds the last message; it carries the sender's
/// display name (camelCase here, unlike the messages endpoint), MRI, content,
/// and timestamps.
#[derive(Debug, Default, Deserialize)]
pub struct RawLastMessage {
    #[serde(rename = "imDisplayName", default)]
    pub im_display_name: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(rename = "originalArrivalTime", default)]
    pub original_arrival_time: Option<String>,
    #[serde(rename = "composeTime", default)]
    pub compose_time: Option<String>,
}

impl RawLastMessage {
    fn timestamp(&self) -> Option<String> {
        self.original_arrival_time
            .clone()
            .or_else(|| self.compose_time.clone())
    }

    fn preview(&self) -> Option<String> {
        self.content
            .as_deref()
            .map(strip_html)
            .filter(|s| !s.is_empty())
    }

    /// (sender MRI, display name) if both are present — feeds the name cache.
    fn name_pair(&self) -> Option<(String, String)> {
        let mri = self.from.clone()?;
        let name = self.im_display_name.clone()?;
        if name.trim().is_empty() {
            None
        } else {
            Some((mri, name))
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct RawTeam {
    #[serde(rename = "displayName", default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub channels: Vec<RawChannel>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawChannel {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "displayName", default)]
    pub display_name: Option<String>,
    #[serde(rename = "isMessageRead", default = "default_true")]
    pub is_message_read: bool,
    #[serde(rename = "lastMessage", default)]
    pub last_message: Option<RawLastMessage>,
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

    /// (MRI, name) pairs gleaned from every last message — recovers names of
    /// people the profile lookup doesn't return (e.g. former employees).
    pub fn last_message_name_pairs(&self) -> Vec<(String, String)> {
        let chat_lms = self.chats.iter().filter_map(|c| c.last_message.as_ref());
        let chan_lms = self
            .teams
            .iter()
            .flat_map(|t| t.channels.iter())
            .filter_map(|c| c.last_message.as_ref());
        chat_lms
            .chain(chan_lms)
            .filter_map(|lm| lm.name_pair())
            .collect()
    }

    /// Reduce the raw response to sidebar rows (chats + channels). `me_mri`
    /// identifies the signed-in user; `names` maps MRIs to display names.
    pub fn into_summaries(
        self,
        me_mri: Option<&str>,
        names: &HashMap<String, String>,
    ) -> Vec<ChatSummary> {
        let mut out = Vec::with_capacity(self.chats.len() + 8);
        for c in self.chats {
            out.push(chat_summary(c, me_mri, names));
        }
        for t in self.teams {
            let team = t.display_name.clone();
            for ch in t.channels {
                if let Some(s) = channel_summary(ch, team.clone(), me_mri) {
                    out.push(s);
                }
            }
        }
        out
    }
}

fn chat_kind(chat: &RawChat, me_mri: Option<&str>) -> ChatKind {
    let has_other = chat
        .members
        .iter()
        .filter_map(|m| m.mri.as_deref())
        .any(|mri| Some(mri) != me_mri);
    if !chat.members.is_empty() && !has_other {
        ChatKind::SelfChat
    } else if chat.is_one_on_one || chat.members.len() == 2 {
        ChatKind::Dm
    } else {
        ChatKind::Group
    }
}

fn chat_summary(c: RawChat, me_mri: Option<&str>, names: &HashMap<String, String>) -> ChatSummary {
    let kind = chat_kind(&c, me_mri);
    let label = chat_label(&c, kind, me_mri, names);
    let (last_activity, last_preview, last_from_me) = match &c.last_message {
        Some(lm) => (
            lm.timestamp(),
            lm.preview(),
            lm.from
                .as_deref()
                .map(|f| Some(f) == me_mri)
                .unwrap_or(false),
        ),
        None => (None, None, false),
    };
    ChatSummary {
        id: c.id,
        label,
        kind,
        unread: !c.is_read,
        last_activity,
        last_preview,
        last_from_me,
        team: None,
    }
}

fn channel_summary(
    ch: RawChannel,
    team: Option<String>,
    me_mri: Option<&str>,
) -> Option<ChatSummary> {
    if ch.id.trim().is_empty() {
        return None;
    }
    let label = ch
        .display_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Channel".to_string());
    let (last_activity, last_preview, last_from_me) = match &ch.last_message {
        Some(lm) => (
            lm.timestamp(),
            lm.preview(),
            lm.from
                .as_deref()
                .map(|f| Some(f) == me_mri)
                .unwrap_or(false),
        ),
        None => (None, None, false),
    };
    Some(ChatSummary {
        id: ch.id,
        label,
        kind: ChatKind::Channel,
        unread: !ch.is_message_read,
        last_activity,
        last_preview,
        last_from_me,
        team,
    })
}

fn chat_label(
    chat: &RawChat,
    kind: ChatKind,
    me_mri: Option<&str>,
    names: &HashMap<String, String>,
) -> String {
    // A named group/topic chat: the custom title wins.
    if let Some(title) = &chat.title {
        if !title.trim().is_empty() {
            return title.clone();
        }
    }

    if kind == ChatKind::SelfChat {
        return "Notes to self".to_string();
    }

    // Members other than me.
    let others: Vec<&str> = chat
        .members
        .iter()
        .filter_map(|m| m.mri.as_deref())
        .filter(|mri| Some(*mri) != me_mri)
        .collect();

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

    match kind {
        ChatKind::Group => "Group chat".to_string(),
        _ => "Direct message".to_string(),
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

#[derive(Debug, Default, Deserialize)]
pub struct RawMessage {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "messagetype", default)]
    pub message_type: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(rename = "imdisplayname", default)]
    pub im_display_name: Option<String>,
    /// Sender, as a contacts URL ending in the MRI (e.g. `…/contacts/8:orgid:…`).
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub composetime: Option<String>,
    #[serde(default)]
    pub originalarrivaltime: Option<String>,
    /// Free-form properties bag; carries `files` / `cards` as JSON strings.
    /// Kept as a `Value` because shapes vary by account/region.
    #[serde(default)]
    pub properties: Option<serde_json::Value>,
}

impl MessagesResponse {
    /// Keep only human chat messages (drop system/thread-activity events) and
    /// project them into the UI `Message` type. `me_mri` marks own messages.
    pub fn into_messages(self, me_mri: Option<&str>) -> Vec<Message> {
        self.messages
            .into_iter()
            .filter_map(|m| m.into_message(me_mri))
            .collect()
    }

    /// (MRI, name) pairs from message senders — feeds the name cache so people
    /// whose profiles don't resolve still get named once you open their chat.
    pub fn name_pairs(&self) -> Vec<(String, String)> {
        self.messages
            .iter()
            .filter_map(|m| {
                let name = m.im_display_name.as_ref()?;
                let mri = mri_from_contact(m.from.as_deref()?)?;
                if name.trim().is_empty() {
                    None
                } else {
                    Some((mri, name.clone()))
                }
            })
            .collect()
    }
}

/// Extract the MRI (final path segment) from a sender contacts URL.
fn mri_from_contact(from: &str) -> Option<String> {
    let seg = from.rsplit('/').next()?.split('?').next()?;
    if seg.is_empty() {
        None
    } else {
        Some(seg.to_string())
    }
}

impl RawMessage {
    fn into_message(self, me_mri: Option<&str>) -> Option<Message> {
        let id = self.id?;
        let mtype = self.message_type.unwrap_or_default();
        // Only render text/rich-text chat messages.
        if !(mtype.starts_with("RichText") || mtype == "Text") {
            return None;
        }
        let raw = self.content.unwrap_or_default();
        let (text, images) = render_message_html(&raw);

        // Hosted images first, then files and cards from the properties bag.
        let mut attachments: Vec<Attachment> = images
            .into_iter()
            .map(|img| Attachment {
                kind: AttachmentKind::Image,
                label: img.name,
                url: Some(img.url),
            })
            .collect();
        if let Some(props) = &self.properties {
            for f in attachment_list(props, "files") {
                if let Some(label) = pick_label(&f, &["title", "fileName", "name"]) {
                    attachments.push(Attachment {
                        kind: AttachmentKind::File,
                        label,
                        url: None,
                    });
                }
            }
            for c in attachment_list(props, "cards") {
                let label = pick_label(&c, &["title", "name"]).unwrap_or_else(|| "card".into());
                attachments.push(Attachment {
                    kind: AttachmentKind::Card,
                    label,
                    url: None,
                });
            }
        }

        // Drop only genuinely empty messages (no text and nothing attached).
        if text.is_empty() && attachments.is_empty() {
            return None;
        }
        let from_me = self
            .from
            .as_deref()
            .and_then(mri_from_contact)
            .map(|mri| Some(mri.as_str()) == me_mri)
            .unwrap_or(false);
        Some(Message {
            id,
            sender: self
                .im_display_name
                .unwrap_or_else(|| "Unknown".to_string()),
            text,
            created: self.originalarrivaltime.or(self.composetime),
            from_me,
            attachments,
        })
    }
}

/// Pull an attachment list out of the properties bag. The value is usually a
/// JSON-encoded string (`"[{…}]"`) but may already be an array, so handle both.
fn attachment_list(props: &serde_json::Value, key: &str) -> Vec<serde_json::Value> {
    match props.get(key) {
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {
            serde_json::from_str(s).unwrap_or_default()
        }
        Some(serde_json::Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    }
}

/// First non-empty string field from `keys` on a JSON object.
fn pick_label(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|k| value.get(k).and_then(|v| v.as_str()))
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(str::to_string)
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
        let msgs = resp.into_messages(None);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].sender, "Ann");
        assert_eq!(msgs[0].text, "hi");
        assert_eq!(msgs[1].text, "yo");
    }

    #[test]
    fn parses_image_and_file_attachments() {
        let json = r#"{"messages":[
            {"id":"1","messagetype":"RichText/Html","imdisplayname":"Ann",
             "content":"<div><img itemtype=\"http://schema.skype.com/AMSImage\" src=\"https://eu-api.asm.skype.com/v1/objects/abc/views/imgo\" alt=\"pic.png\"></div>"},
            {"id":"2","messagetype":"RichText/Html","imdisplayname":"Bob",
             "content":"<div>see file</div>",
             "properties":{"files":"[{\"title\":\"Deck.pptx\",\"fileType\":\"pptx\"}]"}}
        ]}"#;
        let resp: MessagesResponse = serde_json::from_str(json).unwrap();
        let msgs = resp.into_messages(None);
        // The image-only message is kept even though its text is empty.
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].text.is_empty());
        assert_eq!(msgs[0].attachments.len(), 1);
        assert_eq!(msgs[0].attachments[0].kind, AttachmentKind::Image);
        assert_eq!(msgs[0].attachments[0].label, "pic.png");
        assert!(msgs[0].attachments[0]
            .url
            .as_deref()
            .unwrap()
            .contains("/v1/objects/abc"));
        assert_eq!(msgs[1].text, "see file");
        assert_eq!(msgs[1].attachments.len(), 1);
        assert_eq!(msgs[1].attachments[0].kind, AttachmentKind::File);
        assert_eq!(msgs[1].attachments[0].label, "Deck.pptx");
    }

    #[test]
    fn marks_own_messages() {
        let json = r#"{"messages":[
            {"id":"1","messagetype":"Text","content":"hi","imdisplayname":"Me",
             "from":"https://x/v1/users/ME/contacts/8:orgid:me"},
            {"id":"2","messagetype":"Text","content":"yo","imdisplayname":"Ann",
             "from":"https://x/v1/users/ME/contacts/8:orgid:ann"}
        ]}"#;
        let resp: MessagesResponse = serde_json::from_str(json).unwrap();
        let msgs = resp.into_messages(Some("8:orgid:me"));
        assert!(msgs[0].from_me);
        assert!(!msgs[1].from_me);
    }

    #[test]
    fn harvests_names_from_message_senders() {
        let json = r#"{
            "messages": [
                {"id":"1","messagetype":"Text","content":"hi","imdisplayname":"Ann",
                 "from":"https://emea.ng.msg.teams.microsoft.com/v1/users/ME/contacts/8:orgid:abc"}
            ]
        }"#;
        let resp: MessagesResponse = serde_json::from_str(json).unwrap();
        let pairs = resp.name_pairs();
        assert_eq!(pairs, vec![("8:orgid:abc".to_string(), "Ann".to_string())]);
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

    fn label_of(c: &RawChat, names: &HashMap<String, String>) -> String {
        chat_label(c, chat_kind(c, Some(ME)), Some(ME), names)
    }

    #[test]
    fn chat_label_prefers_title() {
        let c = RawChat {
            id: "c1".into(),
            title: Some("Project X".into()),
            members: vec![member(ME), member("8:orgid:x")],
            ..Default::default()
        };
        assert_eq!(label_of(&c, &names(&[])), "Project X");
    }

    #[test]
    fn chat_label_one_on_one_uses_other_member_name() {
        let c = RawChat {
            id: "c2".into(),
            members: vec![member(ME), member("8:orgid:x")],
            is_one_on_one: true,
            ..Default::default()
        };
        let map = names(&[("8:orgid:x", "Carol Smith")]);
        assert_eq!(label_of(&c, &map), "Carol Smith");
    }

    #[test]
    fn chat_label_group_joins_member_names() {
        let c = RawChat {
            id: "c3".into(),
            members: vec![member(ME), member("8:orgid:a"), member("8:orgid:b")],
            ..Default::default()
        };
        let map = names(&[("8:orgid:a", "Ann"), ("8:orgid:b", "Bob")]);
        assert_eq!(label_of(&c, &map), "Ann, Bob");
    }

    #[test]
    fn chat_label_falls_back_to_last_sender() {
        let c = RawChat {
            id: "c4".into(),
            members: vec![member(ME), member("8:orgid:x")],
            is_one_on_one: true,
            last_message: Some(RawLastMessage {
                im_display_name: Some("Carol".into()),
                from: Some("8:orgid:x".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        // No resolved name, but the other party sent the last message.
        assert_eq!(label_of(&c, &names(&[])), "Carol");
    }

    #[test]
    fn chat_label_detects_self_chat() {
        let c = RawChat {
            id: "c5".into(),
            members: vec![member(ME)],
            ..Default::default()
        };
        assert_eq!(label_of(&c, &names(&[])), "Notes to self");
    }

    #[test]
    fn summary_carries_unread_and_recency() {
        let resp: ConversationsResponse = serde_json::from_str(
            r#"{"chats":[
                {"id":"c1","isOneOnOne":true,"isRead":false,
                 "members":[{"mri":"8:orgid:me"},{"mri":"8:orgid:x"}],
                 "lastMessage":{"imDisplayName":"Carol","from":"8:orgid:x",
                   "content":"<div>hey</div>","originalArrivalTime":"2026-01-01T10:00:00Z"}}
            ],"teams":[
                {"displayName":"Team A","channels":[
                   {"id":"19:ch@thread.skype","displayName":"General","isMessageRead":true,
                    "lastMessage":{"originalArrivalTime":"2026-01-02T10:00:00Z"}}]}
            ]}"#,
        )
        .unwrap();
        let s = resp.into_summaries(Some(ME), &names(&[]));
        assert_eq!(s.len(), 2);
        let dm = &s[0];
        assert_eq!(dm.label, "Carol");
        assert_eq!(dm.kind, ChatKind::Dm);
        assert!(dm.unread);
        assert_eq!(dm.last_preview.as_deref(), Some("hey"));
        let ch = &s[1];
        assert_eq!(ch.kind, ChatKind::Channel);
        assert_eq!(ch.label, "General");
        assert_eq!(ch.team.as_deref(), Some("Team A"));
        assert!(!ch.unread);
    }
}
