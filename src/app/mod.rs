//! Application state and input handling (UI-agnostic).

pub mod poller;

use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::teams::models::{ChatSummary, Message};

/// Which pane currently has focus / what the keyboard does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Navigate the chat list and read messages.
    Normal,
    /// Typing a message into the composer.
    Insert,
}

/// An action the main loop should perform after handling a key. Keeps the App
/// free of async/IO concerns.
#[derive(Debug, Clone)]
pub enum Action {
    None,
    Quit,
    /// Load the initial messages for this chat.
    OpenChat(String),
    /// Send the composed text to this chat.
    SendMessage {
        chat_id: String,
        text: String,
    },
}

pub struct App {
    /// Display name of the signed-in user (for the status bar).
    pub display_name: String,

    pub chats: Vec<ChatSummary>,
    pub selected: usize,

    /// The currently opened chat, if any.
    pub active_chat: Option<String>,
    pub messages: Vec<Message>,
    seen_message_ids: HashSet<String>,

    pub mode: Mode,
    pub composer: String,
    pub status: String,
    pub show_help: bool,
    pub loading: bool,
    /// True while a background poll for the active chat is outstanding.
    pub poll_in_flight: bool,
    pub should_quit: bool,
}

impl App {
    pub fn new(display_name: String) -> Self {
        Self {
            display_name,
            chats: Vec::new(),
            selected: 0,
            active_chat: None,
            messages: Vec::new(),
            seen_message_ids: HashSet::new(),
            mode: Mode::Normal,
            composer: String::new(),
            status: "Loading chats…".to_string(),
            show_help: false,
            loading: true,
            poll_in_flight: false,
            should_quit: false,
        }
    }

    pub fn set_chats(&mut self, chats: Vec<ChatSummary>) {
        self.loading = false;
        self.chats = chats;
        if self.selected >= self.chats.len() {
            self.selected = self.chats.len().saturating_sub(1);
        }
        self.status = format!("{} chats", self.chats.len());
    }

    pub fn selected_chat_id(&self) -> Option<String> {
        self.chats.get(self.selected).map(|c| c.id.clone())
    }

    /// Apply a batch of messages for `chat_id`. Ignored if the user has since
    /// switched to a different chat. Dedupes by id and keeps oldest-first order.
    pub fn apply_messages(&mut self, chat_id: &str, msgs: Vec<Message>, initial: bool) {
        if self.active_chat.as_deref() != Some(chat_id) {
            return;
        }
        if initial {
            self.messages.clear();
            self.seen_message_ids.clear();
        }
        let mut added = 0;
        for msg in msgs {
            if self.seen_message_ids.insert(msg.id.clone()) {
                self.messages.push(msg);
                added += 1;
            }
        }
        self.messages.sort_by(|a, b| a.created.cmp(&b.created));
        self.loading = false;
        if !initial {
            self.poll_in_flight = false;
            if added > 0 {
                self.status = format!("{added} new message(s)");
            }
        }
    }

    pub fn open_chat(&mut self, chat_id: String) {
        self.active_chat = Some(chat_id);
        self.messages.clear();
        self.seen_message_ids.clear();
        self.loading = true;
        self.poll_in_flight = false;
        self.status = "Loading messages…".to_string();
    }

    /// Handle a key press, returning an Action for the main loop to execute.
    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        // Global: Ctrl-C always quits.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Action::Quit;
        }
        if self.show_help {
            self.show_help = false;
            return Action::None;
        }
        match self.mode {
            Mode::Normal => self.on_key_normal(key),
            Mode::Insert => self.on_key_insert(key),
        }
    }

    fn on_key_normal(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Char('?') => {
                self.show_help = true;
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.selected + 1 < self.chats.len() {
                    self.selected += 1;
                }
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                Action::None
            }
            KeyCode::Enter => {
                if let Some(id) = self.selected_chat_id() {
                    self.open_chat(id.clone());
                    Action::OpenChat(id)
                } else {
                    Action::None
                }
            }
            KeyCode::Char('i') => {
                if self.active_chat.is_some() {
                    self.mode = Mode::Insert;
                    self.status = "-- INSERT -- (Enter to send, Esc to cancel)".to_string();
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    fn on_key_insert(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.composer.clear();
                self.status = "Ready".to_string();
                Action::None
            }
            KeyCode::Enter => {
                let text = self.composer.trim().to_string();
                if text.is_empty() {
                    return Action::None;
                }
                self.composer.clear();
                self.mode = Mode::Normal;
                match self.active_chat.clone() {
                    Some(chat_id) => {
                        self.status = "Sending…".to_string();
                        Action::SendMessage { chat_id, text }
                    }
                    None => Action::None,
                }
            }
            KeyCode::Backspace => {
                self.composer.pop();
                Action::None
            }
            KeyCode::Char(c) => {
                self.composer.push(c);
                Action::None
            }
            _ => Action::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teams::models::Message;

    fn msg(id: &str, created: &str) -> Message {
        Message {
            id: id.into(),
            sender: "X".into(),
            text: "hi".into(),
            created: Some(created.into()),
        }
    }

    fn test_app() -> App {
        App::new("Me".into())
    }

    #[test]
    fn dedupes_and_sorts_messages() {
        let mut app = test_app();
        app.open_chat("chat1".into());
        app.apply_messages(
            "chat1",
            vec![
                msg("b", "2024-01-01T10:05:00Z"),
                msg("a", "2024-01-01T10:00:00Z"),
            ],
            true,
        );
        // Poll returns an overlap ("b") plus a new message ("c").
        app.apply_messages(
            "chat1",
            vec![
                msg("b", "2024-01-01T10:05:00Z"),
                msg("c", "2024-01-01T10:10:00Z"),
            ],
            false,
        );
        let ids: Vec<&str> = app.messages.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn messages_for_other_chat_are_ignored() {
        let mut app = test_app();
        app.open_chat("chat1".into());
        app.apply_messages("chat2", vec![msg("x", "2024-01-01T10:00:00Z")], true);
        assert!(app.messages.is_empty());
    }
}
