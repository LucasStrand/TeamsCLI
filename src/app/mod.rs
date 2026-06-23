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
    /// Typing a filter to narrow the chat list.
    Search,
}

/// An action the main loop should perform after handling a key.
#[derive(Debug, Clone)]
pub enum Action {
    None,
    Quit,
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
    /// Index into the *visible* (filtered) chat list.
    pub selected: usize,
    /// Case-insensitive substring filter over chat labels.
    pub filter: String,

    /// The currently opened chat, if any.
    pub active_chat: Option<String>,
    pub messages: Vec<Message>,
    seen_message_ids: HashSet<String>,
    /// A chat the main loop should open (load messages for) on its next pass.
    pending_open: Option<String>,

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
            filter: String::new(),
            active_chat: None,
            messages: Vec::new(),
            seen_message_ids: HashSet::new(),
            pending_open: None,
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
        self.selected = 0;
        self.status = format!("{} chats", self.chats.len());
        // Auto-open the first chat so the conversation shows without pressing Enter.
        self.sync_selection();
    }

    /// Indices into `chats` that match the current filter, in order.
    fn visible_indices(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.chats.len()).collect();
        }
        let needle = self.filter.to_lowercase();
        self.chats
            .iter()
            .enumerate()
            .filter(|(_, c)| c.label.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }

    /// The filtered chats to render, paired with their selection position.
    pub fn visible_chats(&self) -> Vec<&ChatSummary> {
        self.visible_indices()
            .into_iter()
            .filter_map(|i| self.chats.get(i))
            .collect()
    }

    pub fn selected_chat_id(&self) -> Option<String> {
        let visible = self.visible_indices();
        visible
            .get(self.selected)
            .and_then(|&i| self.chats.get(i))
            .map(|c| c.id.clone())
    }

    /// Clamp the selection to the visible list and, if it points at a chat other
    /// than the active one, queue it to be opened.
    fn sync_selection(&mut self) {
        let count = self.visible_indices().len();
        if count == 0 {
            self.selected = 0;
            return;
        }
        if self.selected >= count {
            self.selected = count - 1;
        }
        if let Some(id) = self.selected_chat_id() {
            if self.active_chat.as_deref() != Some(id.as_str()) {
                self.pending_open = Some(id);
            }
        }
    }

    /// Taken by the main loop to perform the async message load.
    pub fn take_pending_open(&mut self) -> Option<String> {
        self.pending_open.take()
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
            Mode::Search => self.on_key_search(key),
        }
    }

    fn move_down(&mut self) {
        let count = self.visible_indices().len();
        if self.selected + 1 < count {
            self.selected += 1;
            self.sync_selection();
        }
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.sync_selection();
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
                self.move_down();
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_up();
                Action::None
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                self.status = "Filter chats — type to narrow, Esc to clear".to_string();
                Action::None
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

    fn on_key_search(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.selected = 0;
                self.mode = Mode::Normal;
                self.status = "Ready".to_string();
                self.sync_selection();
            }
            KeyCode::Enter => {
                // Keep the filter, return to navigation.
                self.mode = Mode::Normal;
            }
            KeyCode::Down => self.move_down(),
            KeyCode::Up => self.move_up(),
            KeyCode::Backspace => {
                self.filter.pop();
                self.selected = 0;
                self.sync_selection();
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.selected = 0;
                self.sync_selection();
            }
            _ => {}
        }
        Action::None
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

    fn chat(id: &str, label: &str) -> ChatSummary {
        ChatSummary {
            id: id.into(),
            label: label.into(),
        }
    }

    fn msg(id: &str, created: &str) -> Message {
        Message {
            id: id.into(),
            sender: "X".into(),
            text: "hi".into(),
            created: Some(created.into()),
        }
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
    }

    #[test]
    fn dedupes_and_sorts_messages() {
        let mut app = App::new("Me".into());
        app.open_chat("chat1".into());
        app.apply_messages(
            "chat1",
            vec![
                msg("b", "2024-01-01T10:05:00Z"),
                msg("a", "2024-01-01T10:00:00Z"),
            ],
            true,
        );
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
    fn set_chats_auto_opens_first() {
        let mut app = App::new("Me".into());
        app.set_chats(vec![chat("a", "Ann"), chat("b", "Bob")]);
        assert_eq!(app.take_pending_open().as_deref(), Some("a"));
    }

    #[test]
    fn navigation_queues_open() {
        let mut app = App::new("Me".into());
        app.set_chats(vec![chat("a", "Ann"), chat("b", "Bob")]);
        let first = app.take_pending_open().unwrap();
        app.open_chat(first); // open "a"
        app.on_key(key('j'));
        assert_eq!(app.take_pending_open().as_deref(), Some("b"));
    }

    #[test]
    fn search_filters_and_selects() {
        let mut app = App::new("Me".into());
        app.set_chats(vec![chat("a", "Ann"), chat("b", "Bob"), chat("c", "Bobby")]);
        let _ = app.take_pending_open();
        app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::empty()));
        app.on_key(key('b'));
        let visible: Vec<&str> = app
            .visible_chats()
            .iter()
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(visible, vec!["Bob", "Bobby"]);
        assert_eq!(app.take_pending_open().as_deref(), Some("b"));
    }
}
