//! Async fetch helpers. Each spawns work on the runtime and reports the result
//! back through the event channel, keeping `App` synchronous.

use tokio::sync::mpsc::UnboundedSender;

use crate::event::Event;
use crate::teams::TeamsClient;

/// Load the chat list (initial).
pub fn load_chats(teams: TeamsClient, tx: UnboundedSender<Event>) {
    tokio::spawn(async move {
        let result = teams.list_chats().await.map_err(|e| e.to_string());
        let _ = tx.send(Event::Chats(result));
    });
}

/// Refresh the chat list in the background (recency + unread + notifications).
pub fn refresh_chats(teams: TeamsClient, tx: UnboundedSender<Event>) {
    tokio::spawn(async move {
        let result = teams.list_chats().await.map_err(|e| e.to_string());
        let _ = tx.send(Event::ChatsRefresh(result));
    });
}

/// Load messages for a chat (initial open).
pub fn open_chat(teams: TeamsClient, tx: UnboundedSender<Event>, chat_id: String) {
    fetch_messages(teams, tx, chat_id, true);
}

/// Re-fetch messages for a chat (poll).
pub fn poll_messages(teams: TeamsClient, tx: UnboundedSender<Event>, chat_id: String) {
    fetch_messages(teams, tx, chat_id, false);
}

fn fetch_messages(teams: TeamsClient, tx: UnboundedSender<Event>, chat_id: String, initial: bool) {
    tokio::spawn(async move {
        let result = teams
            .get_messages(&chat_id)
            .await
            .map_err(|e| e.to_string());
        let _ = tx.send(Event::Messages {
            chat_id,
            result,
            initial,
        });
    });
}

/// Send a message, then report success/failure.
pub fn send_message(teams: TeamsClient, tx: UnboundedSender<Event>, chat_id: String, text: String) {
    tokio::spawn(async move {
        let result = teams
            .send_message(&chat_id, &text)
            .await
            .map_err(|e| e.to_string());
        let _ = tx.send(Event::Sent(result));
    });
}

/// Snapshot known contacts (name cache) for the new-chat picker.
pub fn load_people(teams: TeamsClient, tx: UnboundedSender<Event>) {
    tokio::spawn(async move {
        let mut people: Vec<crate::teams::people::Person> = teams
            .names_snapshot()
            .await
            .into_iter()
            .map(|(mri, name)| crate::teams::people::Person { mri, name })
            .collect();
        people.sort_by_key(|p| p.name.to_lowercase());
        let _ = tx.send(Event::People(people));
    });
}

/// Resolve an email address to a person.
pub fn resolve_person(teams: TeamsClient, tx: UnboundedSender<Event>, email: String) {
    tokio::spawn(async move {
        let result = teams.resolve_email(&email).await.map_err(|e| e.to_string());
        let _ = tx.send(Event::PersonResolved(result));
    });
}

/// Create a 1:1 or group chat, then report the new conversation id.
pub fn create_chat(
    teams: TeamsClient,
    tx: UnboundedSender<Event>,
    members: Vec<String>,
    topic: Option<String>,
) {
    tokio::spawn(async move {
        let result = teams
            .create_chat(&members, topic.as_deref())
            .await
            .map_err(|e| e.to_string());
        let _ = tx.send(Event::ChatCreated(result));
    });
}
