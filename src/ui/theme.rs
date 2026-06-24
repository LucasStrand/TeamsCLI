//! Centralized colors so the look can be tweaked in one place.

use ratatui::style::Color;

/// Teams-ish purple accent (other people's names).
pub const ACCENT: Color = Color::Rgb(98, 100, 167);
/// Distinct accent for your own messages.
pub const OWN: Color = Color::Rgb(120, 160, 120);
pub const MUTED: Color = Color::DarkGray;
pub const BG: Color = Color::Black;

/// Subtle fill behind other people's message bubbles (dark purple-gray).
pub const ACCENT_BG: Color = Color::Rgb(30, 30, 46);
/// Subtle fill behind your own message bubbles (dark green-gray).
pub const OWN_BG: Color = Color::Rgb(26, 38, 30);

/// Warm accent for attachment chips (files / images / cards).
pub const CHIP: Color = Color::Rgb(201, 148, 99);
/// Subtle fill behind attachment chips.
pub const CHIP_BG: Color = Color::Rgb(38, 31, 24);
