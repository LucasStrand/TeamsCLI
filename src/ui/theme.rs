//! Centralized colors so the look can be tweaked in one place.

use ratatui::style::Color;

/// Teams-ish purple accent (other people's names).
pub const ACCENT: Color = Color::Rgb(98, 100, 167);
/// Distinct accent for your own messages.
pub const OWN: Color = Color::Rgb(120, 160, 120);
pub const MUTED: Color = Color::DarkGray;
pub const BG: Color = Color::Black;
