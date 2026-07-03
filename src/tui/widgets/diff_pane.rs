//! The virtualized, syntax-highlighted diff pane.
//!
//! Only the rows in the visible window are turned into ratatui `Line`s, and only
//! those lines are highlighted (lazily, via the cached `Highlighter`). Add/remove
//! coloring, syntax colors, and word-diff emphasis are layered per line.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::app::{AppState, Row};
use crate::domain::diff::{ChangeKind, FileChange, InlineSpan, LineKind};
use crate::domain::review::HunkVerdict;
use crate::tui::highlight::{HlSpan, Highlighter};
use crate::tui::theme;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState, hl: &mut Highlighter) {
    let title = format!(
        " Diff — working tree vs HEAD ({} file{}) ",
        state.diff.files.len(),
        if state.diff.files.len() == 1 { "" } else { "s" }
    );
    let block = Block::bordered().title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.flat.is_empty() {
        let msg = Paragraph::new("No changes in the working tree.")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme::gutter_fg()));
        frame.render_widget(msg, inner);
        return;
    }

    let height = inner.height as usize;
    let num_w = gutter_width(state);
    let end = (state.scroll + height).min(state.flat.len());

    let mut lines = Vec::with_capacity(height);
    for idx in state.scroll..end {
        let Some(row) = state.flat.get(idx) else {
            break;
        };
        let on_cursor = idx == state.cursor;
        lines.push(render_row(state, hl, row, num_w, on_cursor));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Widest line number across the diff, clamped to a sane range, for gutter sizing.
fn gutter_width(state: &AppState) -> usize {
    let mut max = 0u32;
    for file in &state.diff.files {
        for hunk in &file.hunks {
            max = max.max(hunk.old.start + hunk.old.count);
            max = max.max(hunk.new.start + hunk.new.count);
        }
    }
    let digits = if max == 0 {
        1
    } else {
        (max as f64).log10().floor() as usize + 1
    };
    digits.clamp(3, 6)
}

fn render_row(
    state: &AppState,
    hl: &mut Highlighter,
    row: Row,
    num_w: usize,
    on_cursor: bool,
) -> Line<'static> {
    let line = match row {
        Row::FileHeader { file } => file_header(&state.diff.files[file]),
        Row::CollapsedSummary { file } => collapsed_summary(&state.diff.files[file]),
        Row::HunkHeader { file, hunk } => hunk_header(state, file, hunk),
        Row::Line { file, hunk, line } => diff_line(state, hl, file, hunk, line, num_w),
    };
    if on_cursor {
        line.style(Style::default().bg(theme::cursor_bg()))
    } else {
        line
    }
}

fn file_header(file: &FileChange) -> Line<'static> {
    let (letter, color) = badge(file.change);
    let arrow = "▾ "; // header rows only exist for files with visible content
    let path = match &file.old_path {
        Some(old) => format!("{} → {}", old.display(), file.path.display()),
        None => file.path.display().to_string(),
    };

    let mut spans = vec![
        Span::raw(arrow),
        Span::styled(
            format!("{letter} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(path, Style::default().add_modifier(Modifier::BOLD)),
    ];
    if file.is_binary {
        spans.push(Span::styled(
            "  (binary)",
            Style::default().fg(theme::gutter_fg()),
        ));
    } else {
        spans.push(Span::styled(
            format!("  +{} -{}", file.stats.0, file.stats.1),
            Style::default().fg(theme::gutter_fg()),
        ));
    }
    if file.base_fallback {
        spans.push(Span::styled(
            "  ⚠ pre-run base missing — diffed vs HEAD",
            Style::default().fg(theme::needs_attention_fg()),
        ));
    }
    Line::from(spans)
}

fn collapsed_summary(file: &FileChange) -> Line<'static> {
    let total: usize = file.hunks.iter().map(|h| h.lines.len()).sum();
    let text = if file.is_binary {
        "    … binary file".to_string()
    } else if total == 0 {
        "    … empty file".to_string()
    } else {
        format!("    … {total} lines collapsed — Space to expand")
    };
    Line::styled(text, Style::default().fg(theme::gutter_fg()))
}

fn hunk_header(state: &AppState, file: usize, hunk: usize) -> Line<'static> {
    let h = &state.diff.files[file].hunks[hunk];
    let (marker, marker_color) = verdict_marker(state.review.verdict(&h.href));
    let mut spans = vec![
        Span::styled(
            format!("{marker} "),
            Style::default().fg(marker_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            h.header.clone(),
            Style::default().fg(theme::hunk_header_fg()),
        ),
    ];
    if state.review.notes.contains_key(&h.href) {
        spans.push(Span::styled(
            "  ✎",
            Style::default().fg(theme::needs_attention_fg()),
        ));
    }
    Line::from(spans)
}

fn diff_line(
    state: &AppState,
    hl: &mut Highlighter,
    file: usize,
    hunk: usize,
    line: usize,
    num_w: usize,
) -> Line<'static> {
    let fc = &state.diff.files[file];
    let h = &fc.hunks[hunk];
    let l = &h.lines[line];

    let (num, sign, sign_color) = match l.kind {
        LineKind::Added => (l.new_no, '+', theme::added_sign()),
        LineKind::Removed => (l.old_no, '-', theme::removed_sign()),
        LineKind::Context => (l.new_no.or(l.old_no), ' ', theme::gutter_fg()),
    };
    let num_text = num.map(|n| n.to_string()).unwrap_or_default();

    let mut spans = vec![
        Span::styled(
            format!("{num_text:>num_w$} "),
            Style::default().fg(theme::gutter_fg()),
        ),
        Span::styled(format!("{sign} "), Style::default().fg(sign_color)),
    ];

    let ext = fc.path.extension().and_then(|e| e.to_str());
    let hl_spans = hl.line(fc.id, hunk, line, ext, &l.text);
    spans.extend(text_spans(&l.text, l.kind, &hl_spans, &l.intra));
    Line::from(spans)
}

/// Split a line's text at syntax + word-diff boundaries and style each segment:
/// syntax color for the foreground, an emphasis background on changed substrings.
fn text_spans(
    text: &str,
    kind: LineKind,
    hl: &[HlSpan],
    intra: &[InlineSpan],
) -> Vec<Span<'static>> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut cuts: Vec<usize> = vec![0, text.len()];
    for s in hl {
        cuts.push(s.start.min(text.len()));
        cuts.push(s.end.min(text.len()));
    }
    for s in intra {
        cuts.push(s.start.min(text.len()));
        cuts.push(s.end.min(text.len()));
    }
    cuts.sort_unstable();
    cuts.dedup();

    let emph_bg = match kind {
        LineKind::Added => theme::added_emph_bg(),
        LineKind::Removed => theme::removed_emph_bg(),
        LineKind::Context => theme::cursor_bg(),
    };
    let default_fg = match kind {
        LineKind::Added => theme::added_fg(),
        LineKind::Removed => theme::removed_fg(),
        LineKind::Context => theme::fg(),
    };

    let mut spans = Vec::new();
    for pair in cuts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a >= b {
            continue;
        }
        let fg = hl
            .iter()
            .find(|s| s.start <= a && a < s.end)
            .map(|s| s.color)
            .unwrap_or(default_fg);
        let changed = intra.iter().any(|s| s.start <= a && a < s.end);
        let mut style = Style::default().fg(fg);
        if changed {
            style = style.bg(emph_bg).add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(text[a..b].to_string(), style));
    }
    spans
}

fn badge(change: ChangeKind) -> (char, Color) {
    match change {
        ChangeKind::Added => ('A', theme::added_sign()),
        ChangeKind::Modified => ('M', theme::needs_attention_fg()),
        ChangeKind::Deleted => ('D', theme::removed_sign()),
        ChangeKind::Renamed => ('R', theme::hunk_header_fg()),
        ChangeKind::Copied => ('C', theme::hunk_header_fg()),
        ChangeKind::TypeChange => ('T', theme::gutter_fg()),
    }
}

fn verdict_marker(verdict: HunkVerdict) -> (char, Color) {
    match verdict {
        HunkVerdict::Unreviewed => (' ', Color::Reset),
        HunkVerdict::Approved => ('✓', theme::approved_fg()),
        HunkVerdict::NeedsAttention => ('✗', theme::needs_attention_fg()),
    }
}
