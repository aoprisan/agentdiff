//! Centralized colors for the diff view: add/remove tints, word-diff emphasis,
//! verdict markers, and the syntect → ratatui color bridge.
//!
//! The add/remove/intent foregrounds are overridable from `config.toml` via
//! [`set_overrides`]; everything else uses the constant defaults below.

use std::sync::OnceLock;

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

/// Config-supplied color overrides for the add/remove/intent foregrounds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Overrides {
    pub added: Option<Color>,
    pub removed: Option<Color>,
    pub intent: Option<Color>,
}

static OVERRIDES: OnceLock<Overrides> = OnceLock::new();

/// Install theme overrides once at startup (later calls are ignored).
pub fn set_overrides(overrides: Overrides) {
    let _ = OVERRIDES.set(overrides);
}

fn overrides() -> Overrides {
    OVERRIDES.get().copied().unwrap_or_default()
}

pub fn added_fg() -> Color {
    overrides().added.unwrap_or(ADDED_FG)
}

pub fn removed_fg() -> Color {
    overrides().removed.unwrap_or(REMOVED_FG)
}

/// Foreground for the intent "WHY" header and confidence meter.
pub fn intent_fg() -> Color {
    overrides().intent.unwrap_or(APPROVED_FG)
}

/// Parse a `#rrggbb` hex color.
pub fn parse_color(s: &str) -> Option<Color> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colors() {
        assert_eq!(parse_color("#a3d977"), Some(Color::Rgb(0xa3, 0xd9, 0x77)));
        assert_eq!(parse_color("a3d977"), None);
        assert_eq!(parse_color("#xyz"), None);
        assert_eq!(parse_color("#12345"), None);
    }
}
