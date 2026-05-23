//! Lazy, viewport-only syntax highlighting.
//!
//! The syntax and theme sets load once (on first use) behind `LazyLock`. We only
//! ever highlight the diff lines that are about to be drawn, and cache the result
//! per `(file, hunk, line)` in an LRU so scrolling back is free. Each line is
//! highlighted independently — fast, and good enough for a diff — at the cost of
//! perfect multi-line constructs (block comments/strings).

use std::num::NonZeroUsize;
use std::sync::LazyLock;

use lru::LruCache;
use ratatui::style::Color;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

use super::theme::syn_to_ratatui;
use crate::domain::diff::FileId;

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

const CACHE_CAPACITY: usize = 2048;

/// A colored slice of a line: a byte range and its foreground color.
#[derive(Clone, Copy)]
pub struct HlSpan {
    pub start: usize,
    pub end: usize,
    pub color: Color,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    file: u32,
    hunk: u32,
    line: u32,
}

const DEFAULT_THEME: &str = "base16-ocean.dark";

pub struct Highlighter {
    theme_name: String,
    cache: LruCache<CacheKey, Vec<HlSpan>>,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl Highlighter {
    pub fn new() -> Self {
        Self::with_theme(DEFAULT_THEME)
    }

    /// Build a highlighter using a named syntect theme, falling back to the
    /// default if the name isn't a bundled theme.
    pub fn with_theme(name: &str) -> Self {
        let theme_name = if THEMES.themes.contains_key(name) {
            name.to_string()
        } else {
            DEFAULT_THEME.to_string()
        };
        Self {
            theme_name,
            cache: LruCache::new(NonZeroUsize::new(CACHE_CAPACITY).unwrap()),
        }
    }

    /// Drop all cached highlights. Called after a re-diff, where `(file, hunk,
    /// line)` indices may now point at different content.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Highlight one diff line, memoized by its position in the diff.
    pub fn line(
        &mut self,
        file: FileId,
        hunk: usize,
        line: usize,
        ext: Option<&str>,
        text: &str,
    ) -> Vec<HlSpan> {
        let key = CacheKey {
            file: file.0,
            hunk: hunk as u32,
            line: line as u32,
        };
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let spans = self.compute(ext, text);
        self.cache.put(key, spans.clone());
        spans
    }

    fn compute(&self, ext: Option<&str>, text: &str) -> Vec<HlSpan> {
        if text.is_empty() {
            return Vec::new();
        }
        let syntax = ext
            .and_then(|e| SYNTAXES.find_syntax_by_extension(e))
            .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());
        let theme = &THEMES.themes[&self.theme_name];
        let mut hl = HighlightLines::new(syntax, theme);

        // syntect tokenizes line-at-a-time and wants the trailing newline.
        let mut owned = String::with_capacity(text.len() + 1);
        owned.push_str(text);
        owned.push('\n');

        let Ok(ranges) = hl.highlight_line(&owned, &SYNTAXES) else {
            return Vec::new();
        };

        let mut spans = Vec::new();
        let mut pos = 0usize;
        for (style, piece) in ranges {
            let end = (pos + piece.len()).min(text.len());
            if pos < end {
                spans.push(HlSpan {
                    start: pos,
                    end,
                    color: syn_to_ratatui(style.foreground),
                });
            }
            pos += piece.len();
            if pos >= text.len() {
                break;
            }
        }
        spans
    }
}
