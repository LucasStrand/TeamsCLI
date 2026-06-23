//! The unified event type that drives the main loop. Input, timer ticks, and
//! async API results all funnel through a single `mpsc` channel.

use crossterm::event::KeyEvent;

use crate::teams::models::{ChatSummary, Message};

#[derive(Debug)]
pub enum Event {
    /// A key press from the terminal.
    Input(KeyEvent),
    /// The terminal was resized; trigger a redraw.
    Resize,
    /// Periodic timer tick — used to drive polling.
    Tick,
    /// Result of loading the chat list.
    Chats(Result<Vec<ChatSummary>, String>),
    /// Messages for a chat (initial load or poll result).
    Messages {
        chat_id: String,
        result: Result<Vec<Message>, String>,
        /// True for the first load of a chat.
        initial: bool,
    },
    /// Result of sending a message.
    Sent(Result<(), String>),
}
