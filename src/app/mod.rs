//! Application state and input handling (UI-agnostic).

pub mod poller;

use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::teams::models::{ChatKind, ChatSummary, Message};
use crate::teams::people::Person;

/// The keyboard interaction mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Vim-style navigation across the focused pane.
    Normal,
    /// Typing a message into the composer.
    Insert,
    /// Typing a filter to narrow the chat list.
    Search,
    /// Picking people to start a new chat.
    NewChat,
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

/// Sidebar filter tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    All,
    Dms,
    Channels,
}

impl Tab {
    pub const ORDER: [Tab; 3] = [Tab::All, Tab::Dms, Tab::Channels];

    pub fn label(self) -> &'static str {
        match self {
            Tab::All => "All",
            Tab::Dms => "DMs",
            Tab::Channels => "Channels",
        }
    }

    fn accepts(self, kind: ChatKind) -> bool {
        match self {
            Tab::All => true,
            Tab::Dms => !kind.is_channel(),
            Tab::Channels => kind.is_channel(),
        }
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
    /// Load known contacts for the new-chat picker.
    LoadPeople,
    /// Resolve an email to a person in the new-chat picker.
    ResolvePerson(String),
    /// Create a new 1:1/group chat with these members (+ optional group topic).
    CreateChat {
        members: Vec<String>,
        topic: Option<String>,
    },
}

/// A new-message notification surfaced by a background refresh.
#[derive(Debug, Clone)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

pub struct App {
    /// Display name of the signed-in user (for the status bar).
    pub display_name: String,

    pub chats: Vec<ChatSummary>,
    /// Index into the *visible* (filtered) chat list.
    pub selected: usize,
    /// Case-insensitive substring filter over chat labels.
    pub filter: String,
    /// Active sidebar tab.
    pub tab: Tab,

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

    // --- new-chat picker state ---
    /// Query typed into the people picker.
    pub nc_query: String,
    /// All known contacts (from the name cache).
    pub nc_people: Vec<Person>,
    /// Cursor into the filtered results.
    pub nc_idx: usize,
    /// People chosen for the new chat (group = 2+).
    pub nc_chosen: Vec<Person>,

    pub status: String,
    pub show_help: bool,
    /// Whether the left chat-list pane is shown (toggle with Ctrl-B for
    /// full-width reading).
    pub show_sidebar: bool,
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
            tab: Tab::All,
            active_chat: None,
            messages: Vec::new(),
            seen_message_ids: HashSet::new(),
            pending_open: None,
            msg_scroll: 0,
            mode: Mode::Normal,
            focus: Focus::Chats,
            pending_g: false,
            composer: String::new(),
            nc_query: String::new(),
            nc_people: Vec::new(),
            nc_idx: 0,
            nc_chosen: Vec::new(),
            status: "Loading chats…".to_string(),
            show_help: false,
            show_sidebar: true,
            loading: true,
            poll_in_flight: false,
            should_quit: false,
        }
    }

    /// Initial chat load: sort by recency and auto-open the most recent.
    pub fn set_chats(&mut self, chats: Vec<ChatSummary>) {
        self.loading = false;
        self.chats = chats;
        self.sort_chats();
        self.selected = 0;
        self.status = format!("{} conversations", self.chats.len());
        self.sync_selection();
    }

    /// Background refresh: replace the list while preserving selection (by id),
    /// keeping the active chat marked read, and returning notifications for chats
    /// that gained a new incoming message.
    pub fn merge_chats(&mut self, mut chats: Vec<ChatSummary>) -> Vec<Notification> {
        let active = self.active_chat.clone();
        let prev_activity: HashMap<String, Option<String>> = self
            .chats
            .iter()
            .map(|c| (c.id.clone(), c.last_activity.clone()))
            .collect();

        let mut notifs = Vec::new();
        for c in &chats {
            let newer = match prev_activity.get(&c.id) {
                None => true,
                Some(prev) => c.last_activity.as_deref() > prev.as_deref(),
            };
            let is_active = active.as_deref() == Some(c.id.as_str());
            if c.unread && !c.last_from_me && !is_active && newer {
                notifs.push(Notification {
                    title: c.label.clone(),
                    body: c
                        .last_preview
                        .clone()
                        .unwrap_or_else(|| "New message".into()),
                });
            }
        }

        // The chat we're reading stays read locally even if the server lags.
        if let Some(active_id) = &active {
            for c in &mut chats {
                if &c.id == active_id {
                    c.unread = false;
                }
            }
        }

        let keep = self.selected_chat_id();
        self.chats = chats;
        self.sort_chats();
        self.reselect_by_id(keep.as_deref());
        notifs
    }

    /// Sort chats newest-first by last activity (undated chats sort last).
    fn sort_chats(&mut self) {
        self.chats
            .sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
    }

    /// Indices into `chats` that match the current tab and filter, in order.
    fn visible_indices(&self) -> Vec<usize> {
        let needle = self.filter.to_lowercase();
        self.chats
            .iter()
            .enumerate()
            .filter(|(_, c)| self.tab.accepts(c.kind))
            .filter(|(_, c)| needle.is_empty() || c.label.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }

    /// The filtered chats to render.
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

    /// Unread counts as (all, dms, channels) for the tab bar.
    pub fn unread_counts(&self) -> (usize, usize, usize) {
        let (mut all, mut dms, mut chans) = (0, 0, 0);
        for c in &self.chats {
            if c.unread {
                all += 1;
                if c.kind.is_channel() {
                    chans += 1;
                } else {
                    dms += 1;
                }
            }
        }
        (all, dms, chans)
    }

    /// Restore the selection to the chat with `id` (within the current view),
    /// clamping if it's no longer visible.
    fn reselect_by_id(&mut self, id: Option<&str>) {
        let visible = self.visible_indices();
        if let Some(id) = id {
            if let Some(pos) = visible.iter().position(|&i| self.chats[i].id == id) {
                self.selected = pos;
                return;
            }
        }
        if self.selected >= visible.len() {
            self.selected = visible.len().saturating_sub(1);
        }
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

    fn set_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.selected = 0;
            self.sync_selection();
        }
    }

    fn cycle_tab(&mut self, forward: bool) {
        let i = Tab::ORDER.iter().position(|t| *t == self.tab).unwrap_or(0);
        let n = if forward {
            (i + 1) % Tab::ORDER.len()
        } else {
            (i + Tab::ORDER.len() - 1) % Tab::ORDER.len()
        };
        self.set_tab(Tab::ORDER[n]);
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
        // Mark read locally so the unread badge clears immediately.
        if let Some(c) = self.chats.iter_mut().find(|c| c.id == chat_id) {
            c.unread = false;
        }
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
            Mode::NewChat => self.on_key_new_chat(key),
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
            // --- layout ---
            KeyCode::Char('b') if ctrl => self.toggle_sidebar(),

            // --- focus switching ---
            KeyCode::Tab => self.focus = toggle_focus(self.focus),
            KeyCode::Char('l') | KeyCode::Right => self.focus = Focus::Messages,
            KeyCode::Char('h') | KeyCode::Left => self.focus = Focus::Chats,
            KeyCode::Enter => self.focus = Focus::Messages,
            KeyCode::Esc if self.focus == Focus::Messages => self.focus = Focus::Chats,

            // --- tabs ---
            KeyCode::Char('1') => self.set_tab(Tab::All),
            KeyCode::Char('2') => self.set_tab(Tab::Dms),
            KeyCode::Char('3') => self.set_tab(Tab::Channels),
            KeyCode::Char(']') => self.cycle_tab(true),
            KeyCode::Char('[') => self.cycle_tab(false),

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
            KeyCode::Char('n') => {
                self.mode = Mode::NewChat;
                self.nc_query.clear();
                self.nc_chosen.clear();
                self.nc_idx = 0;
                self.status = "New chat — type a name, Space to add, Enter to start, Esc to cancel"
                    .to_string();
                return Action::LoadPeople;
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

    /// Show/hide the chat-list pane. Hiding it moves focus to the messages so
    /// keyboard navigation still has a target.
    fn toggle_sidebar(&mut self) {
        self.show_sidebar = !self.show_sidebar;
        if !self.show_sidebar {
            self.focus = Focus::Messages;
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

    // ---- new-chat picker ----

    /// Contacts matching the current query (case-insensitive substring on name).
    pub fn nc_results(&self) -> Vec<&Person> {
        let needle = self.nc_query.trim().to_lowercase();
        self.nc_people
            .iter()
            .filter(|p| needle.is_empty() || p.name.to_lowercase().contains(&needle))
            .collect()
    }

    fn nc_current(&self) -> Option<Person> {
        self.nc_results().get(self.nc_idx).map(|p| (*p).clone())
    }

    fn nc_move(&mut self, delta: isize) {
        let len = self.nc_results().len();
        if len == 0 {
            self.nc_idx = 0;
            return;
        }
        let cur = self.nc_idx as isize;
        self.nc_idx = (cur + delta).clamp(0, len as isize - 1) as usize;
    }

    fn nc_toggle_current(&mut self) {
        if let Some(p) = self.nc_current() {
            if let Some(pos) = self.nc_chosen.iter().position(|c| c.mri == p.mri) {
                self.nc_chosen.remove(pos);
            } else {
                self.nc_chosen.push(p);
            }
        }
    }

    pub fn nc_is_chosen(&self, mri: &str) -> bool {
        self.nc_chosen.iter().any(|c| c.mri == mri)
    }

    /// Replace the picker's contact list (from the name cache).
    pub fn set_people(&mut self, people: Vec<Person>) {
        self.nc_people = people;
        self.nc_idx = 0;
    }

    /// Add an email-resolved person and select them.
    pub fn add_resolved_person(&mut self, person: Option<Person>) {
        match person {
            Some(p) => {
                if !self.nc_people.iter().any(|x| x.mri == p.mri) {
                    self.nc_people.insert(0, p.clone());
                }
                if !self.nc_is_chosen(&p.mri) {
                    self.nc_chosen.push(p.clone());
                }
                self.nc_query.clear();
                self.nc_idx = 0;
                self.status = format!("Added {}", p.name);
            }
            None => self.status = "No match for that email".to_string(),
        }
    }

    pub fn cancel_new_chat(&mut self) {
        self.mode = Mode::Normal;
        self.nc_query.clear();
        self.nc_chosen.clear();
        self.nc_idx = 0;
        self.status = "Ready".to_string();
    }

    /// Called when a new chat was created: leave the picker (the main loop opens
    /// the conversation and refreshes the list).
    pub fn finish_new_chat(&mut self) {
        self.mode = Mode::Normal;
        self.nc_query.clear();
        self.nc_chosen.clear();
        self.nc_idx = 0;
        self.focus = Focus::Messages;
        self.status = "Started chat".to_string();
    }

    fn on_key_new_chat(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.cancel_new_chat(),
            KeyCode::Down => self.nc_move(1),
            KeyCode::Up => self.nc_move(-1),
            KeyCode::Char('n') if ctrl => self.nc_move(1),
            KeyCode::Char('p') if ctrl => self.nc_move(-1),
            // Tab adds/removes the highlighted person (Space is a name character).
            KeyCode::Tab => self.nc_toggle_current(),
            KeyCode::Char('r') if ctrl => {
                let q = self.nc_query.trim().to_string();
                if q.contains('@') {
                    self.status = format!("Looking up {q}…");
                    return Action::ResolvePerson(q);
                }
                self.status = "Type a full email, then Ctrl-R to look it up".to_string();
            }
            KeyCode::Enter => {
                let mut members: Vec<String> =
                    self.nc_chosen.iter().map(|p| p.mri.clone()).collect();
                if members.is_empty() {
                    if let Some(p) = self.nc_current() {
                        members.push(p.mri);
                    }
                }
                if members.is_empty() {
                    self.status = "Pick at least one person (Tab to add)".to_string();
                    return Action::None;
                }
                let topic = None; // group auto-named by members for now
                self.status = "Creating chat…".to_string();
                return Action::CreateChat { members, topic };
            }
            KeyCode::Backspace => {
                self.nc_query.pop();
                self.nc_idx = 0;
            }
            KeyCode::Char(c) => {
                self.nc_query.push(c);
                self.nc_idx = 0;
            }
            _ => {}
        }
        Action::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teams::models::{ChatKind, Message};

    fn chat_full(
        id: &str,
        label: &str,
        kind: ChatKind,
        ts: Option<&str>,
        unread: bool,
    ) -> ChatSummary {
        ChatSummary {
            id: id.into(),
            label: label.into(),
            kind,
            unread,
            last_activity: ts.map(|s| s.into()),
            last_preview: None,
            last_from_me: false,
            team: None,
        }
    }

    fn chat(id: &str, label: &str) -> ChatSummary {
        chat_full(id, label, ChatKind::Dm, None, false)
    }

    fn msg(id: &str, created: &str) -> Message {
        Message {
            id: id.into(),
            sender: "X".into(),
            text: "hi".into(),
            created: Some(created.into()),
            from_me: false,
            attachments: Vec::new(),
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
    fn sorts_chats_by_recency() {
        let mut app = App::new("Me".into());
        app.set_chats(vec![
            chat_full(
                "a",
                "Ann",
                ChatKind::Dm,
                Some("2026-01-01T09:00:00Z"),
                false,
            ),
            chat_full(
                "b",
                "Bob",
                ChatKind::Dm,
                Some("2026-01-03T09:00:00Z"),
                false,
            ),
            chat_full(
                "c",
                "Cara",
                ChatKind::Dm,
                Some("2026-01-02T09:00:00Z"),
                false,
            ),
        ]);
        let order: Vec<&str> = app.chats.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(order, vec!["b", "c", "a"]);
        // Auto-opens the most recent.
        assert_eq!(app.take_pending_open().as_deref(), Some("b"));
    }

    #[test]
    fn merge_preserves_selection_and_reports_notifications() {
        let mut app = App::new("Me".into());
        app.set_chats(vec![
            chat_full(
                "a",
                "Ann",
                ChatKind::Dm,
                Some("2026-01-02T09:00:00Z"),
                false,
            ),
            chat_full(
                "b",
                "Bob",
                ChatKind::Dm,
                Some("2026-01-01T09:00:00Z"),
                false,
            ),
        ]);
        let _ = app.take_pending_open();
        // Select Bob.
        app.on_key(key('j'));
        assert_eq!(app.selected_chat_id().as_deref(), Some("b"));

        // Refresh: Bob gets a newer incoming message and floats to top.
        let notifs = app.merge_chats(vec![
            chat_full(
                "a",
                "Ann",
                ChatKind::Dm,
                Some("2026-01-02T09:00:00Z"),
                false,
            ),
            chat_full("b", "Bob", ChatKind::Dm, Some("2026-01-05T09:00:00Z"), true),
        ]);
        // Notification fired for Bob.
        assert_eq!(notifs.len(), 1);
        assert_eq!(notifs[0].title, "Bob");
        // Bob is now at top, and selection followed Bob (not the index).
        assert_eq!(app.chats[0].id, "b");
        assert_eq!(app.selected_chat_id().as_deref(), Some("b"));
    }

    #[test]
    fn no_notification_for_active_chat() {
        let mut app = App::new("Me".into());
        app.set_chats(vec![chat_full(
            "a",
            "Ann",
            ChatKind::Dm,
            Some("2026-01-01T09:00:00Z"),
            false,
        )]);
        let id = app.take_pending_open().unwrap();
        app.open_chat(id); // active = a
        let notifs = app.merge_chats(vec![chat_full(
            "a",
            "Ann",
            ChatKind::Dm,
            Some("2026-01-09T09:00:00Z"),
            true,
        )]);
        assert!(notifs.is_empty());
        // Active chat stays read locally.
        assert!(!app.chats[0].unread);
    }

    #[test]
    fn tab_filters_channels() {
        let mut app = App::new("Me".into());
        app.set_chats(vec![
            chat_full(
                "a",
                "Ann",
                ChatKind::Dm,
                Some("2026-01-02T00:00:00Z"),
                false,
            ),
            chat_full(
                "g",
                "General",
                ChatKind::Channel,
                Some("2026-01-01T00:00:00Z"),
                false,
            ),
        ]);
        let _ = app.take_pending_open();
        app.on_key(key('3')); // Channels tab
        let vis: Vec<&str> = app
            .visible_chats()
            .iter()
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(vis, vec!["General"]);
        app.on_key(key('2')); // DMs tab
        let vis: Vec<&str> = app
            .visible_chats()
            .iter()
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(vis, vec!["Ann"]);
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
    }

    #[test]
    fn focus_switches_motion_target() {
        let mut app = App::new("Me".into());
        app.set_chats(vec![chat("a", "Ann"), chat("b", "Bob")]);
        let _ = app.take_pending_open();
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));
        assert_eq!(app.focus, Focus::Messages);
        app.on_key(key('k')); // scroll up in messages
        assert_eq!(app.selected, 0); // selection unchanged
        assert!(app.msg_scroll > 0);
    }

    #[test]
    fn new_chat_filter_select_and_create() {
        use crate::teams::people::Person;
        let mut app = App::new("Me".into());
        let action = app.on_key(key('n'));
        assert!(matches!(action, Action::LoadPeople));
        assert_eq!(app.mode, Mode::NewChat);
        app.set_people(vec![
            Person {
                mri: "8:orgid:a".into(),
                name: "Alice".into(),
            },
            Person {
                mri: "8:orgid:b".into(),
                name: "Bob".into(),
            },
            Person {
                mri: "8:orgid:c".into(),
                name: "Bobby".into(),
            },
        ]);
        app.on_key(key('b'));
        app.on_key(key('o'));
        let names: Vec<&str> = app.nc_results().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["Bob", "Bobby"]);

        // Tab adds the highlighted person (Bob).
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));
        assert!(app.nc_is_chosen("8:orgid:b"));

        // Enter creates a 1:1 with the chosen member.
        let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
        match action {
            Action::CreateChat { members, topic } => {
                assert_eq!(members, vec!["8:orgid:b".to_string()]);
                assert!(topic.is_none());
            }
            other => panic!("expected CreateChat, got {other:?}"),
        }
    }
}
