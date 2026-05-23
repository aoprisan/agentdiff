//! Centralized colors for the diff view, as a selectable [`Palette`].
//!
//! A built-in palette (`default`, `solarized-dark`, `solarized-light`) is chosen
//! via `config.toml` and installed once at startup; per-color overrides
//! (add/remove/intent) layer on top. Every widget reads the live palette through
//! the accessor functions below.

use std::sync::OnceLock;

use ratatui::style::Color;
use syntect::highlighting::Color as SynColor;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// The full set of UI colors plus the syntect syntax theme that pairs with them.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub added_fg: Color,
    pub removed_fg: Color,
    pub added_sign: Color,
    pub removed_sign: Color,
    pub added_emph_bg: Color,
    pub removed_emph_bg: Color,
    pub gutter_fg: Color,
    pub hunk_header_fg: Color,
    pub cursor_bg: Color,
    pub approved_fg: Color,
    pub needs_attention_fg: Color,
    pub changed_since_fg: Color,
    pub intent_fg: Color,
    /// syntect theme used when the user doesn't set one explicitly.
    pub syntax: &'static str,
}

impl Palette {
    /// The original built-in dark palette.
    pub const fn default_dark() -> Self {
        Palette {
            added_fg: rgb(0xa3, 0xd9, 0x77),
            removed_fg: rgb(0xe0, 0x6c, 0x75),
            added_sign: rgb(0x6a, 0x99, 0x55),
            removed_sign: rgb(0xc0, 0x4a, 0x52),
            added_emph_bg: rgb(0x2c, 0x4a, 0x2c),
            removed_emph_bg: rgb(0x4a, 0x2c, 0x2c),
            gutter_fg: rgb(0x60, 0x66, 0x70),
            hunk_header_fg: rgb(0x56, 0x9c, 0xd6),
            cursor_bg: rgb(0x33, 0x38, 0x42),
            approved_fg: rgb(0x6a, 0x99, 0x55),
            needs_attention_fg: rgb(0xd1, 0x9a, 0x33),
            changed_since_fg: rgb(0xd1, 0x9a, 0x33),
            intent_fg: rgb(0x6a, 0x99, 0x55),
            syntax: "base16-ocean.dark",
        }
    }

    /// Solarized dark (Ethan Schoonover) on a base03 background.
    pub const fn solarized_dark() -> Self {
        Palette {
            added_fg: SOL_GREEN,
            removed_fg: SOL_RED,
            added_sign: SOL_GREEN,
            removed_sign: SOL_RED,
            added_emph_bg: rgb(0x0b, 0x3a, 0x2e),
            removed_emph_bg: rgb(0x3a, 0x14, 0x14),
            gutter_fg: SOL_BASE01,
            hunk_header_fg: SOL_BLUE,
            cursor_bg: SOL_BASE02,
            approved_fg: SOL_GREEN,
            needs_attention_fg: SOL_YELLOW,
            changed_since_fg: SOL_ORANGE,
            intent_fg: SOL_CYAN,
            syntax: "Solarized (dark)",
        }
    }

    /// Solarized light on a base3 background.
    pub const fn solarized_light() -> Self {
        Palette {
            added_fg: SOL_GREEN,
            removed_fg: SOL_RED,
            added_sign: SOL_GREEN,
            removed_sign: SOL_RED,
            added_emph_bg: rgb(0xdd, 0xe7, 0xc8),
            removed_emph_bg: rgb(0xf0, 0xd8, 0xd5),
            gutter_fg: SOL_BASE1,
            hunk_header_fg: SOL_BLUE,
            cursor_bg: SOL_BASE2,
            approved_fg: SOL_GREEN,
            needs_attention_fg: SOL_YELLOW,
            changed_since_fg: SOL_ORANGE,
            intent_fg: SOL_CYAN,
            syntax: "Solarized (light)",
        }
    }

    /// Resolve a palette by config name. `None` for an unknown name.
    pub fn by_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "" | "default" | "default-dark" => Some(Self::default_dark()),
            "solarized" | "solarized-dark" => Some(Self::solarized_dark()),
            "solarized-light" => Some(Self::solarized_light()),
            _ => None,
        }
    }

    /// Apply optional per-color overrides (config `added`/`removed`/`intent`).
    pub fn with_overrides(
        mut self,
        added: Option<Color>,
        removed: Option<Color>,
        intent: Option<Color>,
    ) -> Self {
        if let Some(c) = added {
            self.added_fg = c;
            self.added_sign = c;
        }
        if let Some(c) = removed {
            self.removed_fg = c;
            self.removed_sign = c;
        }
        if let Some(c) = intent {
            self.intent_fg = c;
        }
        self
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::default_dark()
    }
}

// Canonical Solarized accent + base colors.
const SOL_YELLOW: Color = rgb(0xb5, 0x89, 0x00);
const SOL_ORANGE: Color = rgb(0xcb, 0x4b, 0x16);
const SOL_RED: Color = rgb(0xdc, 0x32, 0x2f);
const SOL_BLUE: Color = rgb(0x26, 0x8b, 0xd2);
const SOL_CYAN: Color = rgb(0x2a, 0xa1, 0x98);
const SOL_GREEN: Color = rgb(0x85, 0x99, 0x00);
const SOL_BASE02: Color = rgb(0x07, 0x36, 0x42);
const SOL_BASE01: Color = rgb(0x58, 0x6e, 0x75);
const SOL_BASE1: Color = rgb(0x93, 0xa1, 0xa1);
const SOL_BASE2: Color = rgb(0xee, 0xe8, 0xd5);

static DEFAULT: Palette = Palette::default_dark();
static PALETTE: OnceLock<Palette> = OnceLock::new();

/// Install the active palette once at startup (later calls are ignored).
pub fn install(palette: Palette) {
    let _ = PALETTE.set(palette);
}

fn current() -> &'static Palette {
    PALETTE.get().unwrap_or(&DEFAULT)
}

pub fn added_fg() -> Color {
    current().added_fg
}
pub fn removed_fg() -> Color {
    current().removed_fg
}
pub fn added_sign() -> Color {
    current().added_sign
}
pub fn removed_sign() -> Color {
    current().removed_sign
}
pub fn added_emph_bg() -> Color {
    current().added_emph_bg
}
pub fn removed_emph_bg() -> Color {
    current().removed_emph_bg
}
pub fn gutter_fg() -> Color {
    current().gutter_fg
}
pub fn hunk_header_fg() -> Color {
    current().hunk_header_fg
}
pub fn cursor_bg() -> Color {
    current().cursor_bg
}
pub fn approved_fg() -> Color {
    current().approved_fg
}
pub fn needs_attention_fg() -> Color {
    current().needs_attention_fg
}
pub fn changed_since_fg() -> Color {
    current().changed_since_fg
}
/// Foreground for the intent "WHY" header and confidence meter.
pub fn intent_fg() -> Color {
    current().intent_fg
}

/// Convert a syntect color to a ratatui RGB color, dropping the alpha channel.
pub fn syn_to_ratatui(c: SynColor) -> Color {
    Color::Rgb(c.r, c.g, c.b)
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

    #[test]
    fn resolves_palette_names() {
        assert_eq!(Palette::solarized_dark().syntax, "Solarized (dark)");
        assert_eq!(Palette::solarized_light().syntax, "Solarized (light)");
        assert!(Palette::by_name("Solarized-Dark").is_some());
        assert!(Palette::by_name("nonsense").is_none());
        // Solarized uses the canonical green/red accents.
        assert_eq!(Palette::solarized_dark().added_fg, SOL_GREEN);
        assert_eq!(Palette::solarized_dark().removed_fg, SOL_RED);
    }

    #[test]
    fn overrides_replace_individual_colors() {
        let p = Palette::default_dark().with_overrides(parse_color("#010203"), None, None);
        assert_eq!(p.added_fg, Color::Rgb(1, 2, 3));
        assert_eq!(p.added_sign, Color::Rgb(1, 2, 3));
        // Untouched colors keep their defaults.
        assert_eq!(p.removed_fg, Palette::default_dark().removed_fg);
    }
}
