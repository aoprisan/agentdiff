//! Centralized colors for the diff view: add/remove tints, word-diff emphasis,
//! verdict markers, and the syntect → ratatui color bridge.

use ratatui::style::Color;
use syntect::highlighting::Color as SynColor;

pub const ADDED_FG: Color = Color::Rgb(0xa3, 0xd9, 0x77);
pub const REMOVED_FG: Color = Color::Rgb(0xe0, 0x6c, 0x75);
pub const ADDED_SIGN: Color = Color::Rgb(0x6a, 0x99, 0x55);
pub const REMOVED_SIGN: Color = Color::Rgb(0xc0, 0x4a, 0x52);

/// Background tint for the word-diff changed substrings.
pub const ADDED_EMPH_BG: Color = Color::Rgb(0x2c, 0x4a, 0x2c);
pub const REMOVED_EMPH_BG: Color = Color::Rgb(0x4a, 0x2c, 0x2c);

pub const GUTTER_FG: Color = Color::Rgb(0x60, 0x66, 0x70);
pub const HUNK_HEADER_FG: Color = Color::Rgb(0x56, 0x9c, 0xd6);
pub const CURSOR_BG: Color = Color::Rgb(0x33, 0x38, 0x42);

pub const APPROVED_FG: Color = Color::Rgb(0x6a, 0x99, 0x55);
pub const NEEDS_ATTENTION_FG: Color = Color::Rgb(0xd1, 0x9a, 0x33);
pub const CHANGED_SINCE_FG: Color = Color::Rgb(0xd1, 0x9a, 0x33);

/// Convert a syntect color to a ratatui RGB color, dropping the alpha channel.
pub fn syn_to_ratatui(c: SynColor) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}
