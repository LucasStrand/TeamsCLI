//! Starting new conversations: resolve a person by email, and create a 1:1 or
//! group thread.

use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::config;

use super::TeamsClient;

/// A person who can be added to a new chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub mri: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct FetchProfile {
    #[serde(default)]
    mri: Option<String>,
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
}

// The fetch endpoint returns either a bare array or `{ "value": [...] }`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FetchResponse {
    Wrapped { value: Vec<FetchProfile> },
    Bare(Vec<FetchProfile>),
}

impl FetchResponse {
    fn into_profiles(self) -> Vec<FetchProfile> {
        match self {
            FetchResponse::Wrapped { value } => value,
            FetchResponse::Bare(v) => v,
        }
    }
}

impl TeamsClient {
    /// Resolve an email address to a person via the middle-tier `users/fetch`.
    pub async fn resolve_email(&self, email: &str) -> Result<Option<Person>> {
        let region = self.region().await?;
        let url = format!(
            "{}/{region}/beta/users/fetch\
             ?isMailAddress=true&canBeSmtpAddress=true&enableGuest=true\
             &includeIBBarredUsers=true&skypeTeamsInfo=true",
            config::MT_BASE
        );
        let body = serde_json::to_vec(&[email])?;
        let bytes = self.post_bytes(&url, body, config::SKYPE_RESOURCE).await?;
        tracing::debug!(
            "users/fetch raw response: {}",
            String::from_utf8_lossy(&bytes)
        );
        let resp: FetchResponse = serde_json::from_slice(&bytes)?;
        let person = resp.into_profiles().into_iter().find_map(|p| {
            let mri = p.mri?;
            let name = p.display_name.unwrap_or_else(|| email.to_string());
            Some(Person { mri, name })
        });
        // Learn the name for future labeling.
        if let Some(p) = &person {
            self.merge_names([(p.mri.clone(), p.name.clone())]).await;
        }
        Ok(person)
    }

    /// Create a 1:1 (one other member) or group (2+ members, with `topic`) chat
    /// and return its conversation id. The signed-in user is added automatically.
    pub async fn create_chat(&self, members: &[String], topic: Option<&str>) -> Result<String> {
        let me = self
            .me_mri()
            .ok_or_else(|| anyhow!("cannot create a chat without knowing your own identity"))?;

        let mut roster: Vec<ThreadMember> = Vec::with_capacity(members.len() + 1);
        roster.push(ThreadMember::admin(me));
        for m in members {
            if m != me {
                roster.push(ThreadMember::admin(m));
            }
        }

        let body = CreateThread {
            members: roster,
            properties: ThreadProperties {
                thread_type: "chat",
                topic: topic.filter(|t| !t.trim().is_empty()),
            },
        };

        let host = self.messaging_host().await?;
        let url = format!("{host}/v1/threads");
        let location = self
            .post_for_location(&url, serde_json::to_vec(&body)?)
            .await?
            .ok_or_else(|| anyhow!("thread created but no id was returned"))?;
        thread_id_from_location(&location)
            .ok_or_else(|| anyhow!("could not parse thread id from '{location}'"))
    }
}

#[derive(serde::Serialize)]
struct CreateThread<'a> {
    members: Vec<ThreadMember<'a>>,
    properties: ThreadProperties<'a>,
}

#[derive(serde::Serialize)]
struct ThreadMember<'a> {
    id: &'a str,
    role: &'a str,
}

impl<'a> ThreadMember<'a> {
    fn admin(id: &'a str) -> Self {
        Self { id, role: "Admin" }
    }
}

#[derive(serde::Serialize)]
struct ThreadProperties<'a> {
    #[serde(rename = "threadType")]
    thread_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    topic: Option<&'a str>,
}

/// Extract the thread id (final path segment) from a `Location` header.
fn thread_id_from_location(location: &str) -> Option<String> {
    let seg = location.rsplit('/').next()?.split('?').next()?;
    if seg.is_empty() {
        None
    } else {
        Some(seg.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_thread_id_from_location() {
        assert_eq!(
            thread_id_from_location(
                "https://emea.ng.msg.teams.microsoft.com/v1/threads/19:abc@thread.v2"
            )
            .as_deref(),
            Some("19:abc@thread.v2")
        );
    }

    #[test]
    fn fetch_response_accepts_both_shapes() {
        let bare: FetchResponse =
            serde_json::from_str(r#"[{"mri":"8:orgid:x","displayName":"X"}]"#).unwrap();
        assert_eq!(bare.into_profiles().len(), 1);
        let wrapped: FetchResponse =
            serde_json::from_str(r#"{"value":[{"mri":"8:orgid:y","displayName":"Y"}]}"#).unwrap();
        assert_eq!(wrapped.into_profiles().len(), 1);
    }

    #[test]
    fn create_thread_body_includes_topic_for_group() {
        let body = CreateThread {
            members: vec![
                ThreadMember::admin("8:orgid:me"),
                ThreadMember::admin("8:orgid:a"),
            ],
            properties: ThreadProperties {
                thread_type: "chat",
                topic: Some("Project X"),
            },
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"topic\":\"Project X\""));
        assert!(json.contains("\"role\":\"Admin\""));
    }

    #[test]
    fn create_thread_body_omits_empty_topic() {
        let body = CreateThread {
            members: vec![ThreadMember::admin("8:orgid:me")],
            properties: ThreadProperties {
                thread_type: "chat",
                topic: None,
            },
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(!json.contains("topic"));
    }
}
