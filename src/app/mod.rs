//! Application state and input handling (UI-agnostic).

pub mod poller;

use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::teams::models::{ChatSummary, Message};

/// The keyboard interaction mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Vim-style navigation across the focused pane.
    Normal,
    /// Typing a message into the composer.
    Insert,
    /// Typing a filter to narrow the chat list.
    Search,
}

/// Which pane the keyboard acts on in Normal mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The chat list (sidebar).
    Chats,
    /// The message view.
    Messages,
}

fn toggle_focus(focus: Focus) -> Focus {
    match focus {
        Focus::Chats => Focus::Messages,
        Focus::Messages => Focus::Chats,
    }
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
    /// How many lines the message view is scrolled up from the bottom. 0 = follow
    /// the newest messages. Clamped to the real maximum by the renderer.
    pub msg_scroll: u16,

    pub mode: Mode,
    pub focus: Focus,
    /// True after a lone `g`, awaiting a second `g` for "go to top".
    pending_g: bool,
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
            msg_scroll: 0,
            mode: Mode::Normal,
            focus: Focus::Chats,
            pending_g: false,
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
        if initial {
            self.status = if self.messages.is_empty() {
                "No messages yet".to_string()
            } else {
                format!("{} messages", self.messages.len())
            };
        } else {
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
        self.msg_scroll = 0;
        self.status = "Loading messages…".to_string();
    }

    /// Scroll the message view up (towards older messages).
    pub fn scroll_up(&mut self, lines: u16) {
        self.msg_scroll = self.msg_scroll.saturating_add(lines);
    }

    /// Scroll the message view down (towards newer messages).
    pub fn scroll_down(&mut self, lines: u16) {
        self.msg_scroll = self.msg_scroll.saturating_sub(lines);
    }

    /// Jump to the oldest messages. The renderer clamps to the real maximum.
    pub fn scroll_to_top(&mut self) {
        self.msg_scroll = u16::MAX;
    }

    /// Jump back to the newest messages (resume following).
    pub fn scroll_to_bottom(&mut self) {
        self.msg_scroll = 0;
    }

    /// Mouse-wheel scroll, applied to whichever pane has focus.
    pub fn wheel(&mut self, up: bool) {
        // Wheel up = move backwards (older messages / earlier chats).
        self.pane_step(3, !up);
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

    /// Move the chat-list selection down by `n` (clamped), opening as it goes.
    fn select_down(&mut self, n: usize) {
        let count = self.visible_indices().len();
        if count == 0 {
            return;
        }
        self.selected = (self.selected + n).min(count - 1);
        self.sync_selection();
    }

    fn select_up(&mut self, n: usize) {
        self.selected = self.selected.saturating_sub(n);
        self.sync_selection();
    }

    fn select_to(&mut self, idx: usize) {
        let count = self.visible_indices().len();
        if count == 0 {
            return;
        }
        self.selected = idx.min(count - 1);
        self.sync_selection();
    }

    fn on_key_normal(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // `gg` = go to top: consume the pending `g` if this is the second one.
        let after_g = self.pending_g;
        self.pending_g = false;

        match key.code {
            KeyCode::Char('q') => return Action::Quit,
            KeyCode::Char('?') => {
                self.show_help = true;
            }
            // --- focus switching ---
            KeyCode::Tab => self.focus = toggle_focus(self.focus),
            KeyCode::Char('l') | KeyCode::Right => self.focus = Focus::Messages,
            KeyCode::Char('h') | KeyCode::Left => self.focus = Focus::Chats,
            KeyCode::Enter => self.focus = Focus::Messages,
            KeyCode::Esc if self.focus == Focus::Messages => self.focus = Focus::Chats,

            // --- modes ---
            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                self.focus = Focus::Chats;
                self.status = "Filter chats — type to narrow, Esc to clear".to_string();
            }
            KeyCode::Char('i') => {
                if self.active_chat.is_some() {
                    self.mode = Mode::Insert;
                    self.status = "-- INSERT -- (Enter to send, Esc to cancel)".to_string();
                }
            }

            // --- `g` then `g` = top ---
            KeyCode::Char('g') => {
                if after_g {
                    self.goto_top();
                } else {
                    self.pending_g = true;
                }
            }
            KeyCode::Char('G') => self.goto_bottom(),

            // --- vim motion, dispatched to the focused pane ---
            KeyCode::Char('j') | KeyCode::Down => self.pane_step(1, true),
            KeyCode::Char('k') | KeyCode::Up => self.pane_step(1, false),
            KeyCode::Char('d') if ctrl => self.pane_step(10, true),
            KeyCode::Char('u') if ctrl => self.pane_step(10, false),
            KeyCode::PageDown => self.pane_step(10, true),
            KeyCode::PageUp => self.pane_step(10, false),
            KeyCode::Home => self.goto_top(),
            KeyCode::End => self.goto_bottom(),

            _ => {}
        }
        Action::None
    }

    /// A down (`forward`) or up motion of `n` units applied to the focused pane:
    /// the chat selection when on the sidebar, the message scroll when on the
    /// message view.
    fn pane_step(&mut self, n: u16, forward: bool) {
        match self.focus {
            Focus::Chats => {
                if forward {
                    self.select_down(n as usize);
                } else {
                    self.select_up(n as usize);
                }
            }
            Focus::Messages => {
                // In the message view, down = towards newer = less scroll-back.
                if forward {
                    self.scroll_down(n);
                } else {
                    self.scroll_up(n);
                }
            }
        }
    }

    fn goto_top(&mut self) {
        match self.focus {
            Focus::Chats => self.select_to(0),
            Focus::Messages => self.scroll_to_top(),
        }
    }

    fn goto_bottom(&mut self) {
        match self.focus {
            Focus::Chats => self.select_to(usize::MAX),
            Focus::Messages => self.scroll_to_bottom(),
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
            KeyCode::Down => self.select_down(1),
            KeyCode::Up => self.select_up(1),
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

    #[test]
    fn capital_g_jumps_to_last_chat() {
        let mut app = App::new("Me".into());
        app.set_chats(vec![chat("a", "Ann"), chat("b", "Bob"), chat("c", "Cara")]);
        let _ = app.take_pending_open();
        app.on_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::empty()));
        assert_eq!(app.selected, 2);
        assert_eq!(app.take_pending_open().as_deref(), Some("c"));
    }

    #[test]
    fn gg_jumps_to_first_chat() {
        let mut app = App::new("Me".into());
        app.set_chats(vec![chat("a", "Ann"), chat("b", "Bob"), chat("c", "Cara")]);
        let _ = app.take_pending_open();
        app.on_key(key('G'));
        app.on_key(key('g'));
        app.on_key(key('g')); // second g triggers "go to top"
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn focus_switches_motion_target() {
        let mut app = App::new("Me".into());
        app.set_chats(vec![chat("a", "Ann"), chat("b", "Bob")]);
        let _ = app.take_pending_open();
        // Focus the message view: j should scroll messages, not move selection.
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));
        assert_eq!(app.focus, Focus::Messages);
        app.on_key(key('k')); // scroll up in messages
        assert_eq!(app.selected, 0); // selection unchanged
        assert!(app.msg_scroll > 0);
    }
}
