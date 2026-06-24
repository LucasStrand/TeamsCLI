//! The unified event type that drives the main loop. Input, timer ticks, and
//! async API results all funnel through a single `mpsc` channel.

use crossterm::event::KeyEvent;

use crate::teams::models::{ChatSummary, Message};
use crate::teams::people::Person;

#[derive(Debug)]
pub enum Event {
    /// A key press from the terminal.
    Input(KeyEvent),
    /// Mouse wheel scrolled up (`true`) or down (`false`) over the message view.
    Scroll(bool),
    /// The terminal was resized; trigger a redraw.
    Resize,
    /// Periodic timer tick — used to drive polling.
    Tick,
    /// Result of the initial chat-list load.
    Chats(Result<Vec<ChatSummary>, String>),
    /// Result of a periodic background refresh of the chat list (merged in,
    /// preserving selection; drives unread badges + notifications).
    ChatsRefresh(Result<Vec<ChatSummary>, String>),
    /// Messages for a chat (initial load or poll result).
    Messages {
        chat_id: String,
        result: Result<Vec<Message>, String>,
        /// True for the first load of a chat.
        initial: bool,
    },
    /// Result of sending a message.
    Sent(Result<(), String>),
    /// Known contacts (from the name cache) for the new-chat picker.
    People(Vec<Person>),
    /// Result of resolving an email to a person for the new-chat picker.
    PersonResolved(Result<Option<Person>, String>),
    /// Result of creating a new chat — the new conversation id.
    ChatCreated(Result<String, String>),
    /// A hosted image finished downloading: its object URL and raw bytes.
    ImageLoaded { url: String, bytes: Vec<u8> },
    /// A hosted image failed to download / decode; its chip stays shown.
    ImageFailed { url: String },
}
