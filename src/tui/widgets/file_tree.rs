//! Left pane: the changed-file list with status badges, per-file line stats, a
//! collapse indicator, and a roll-up verdict marker. The file containing the
//! diff-pane cursor is highlighted; selection follows navigation.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::app::{AppState, file_collapsed};
use crate::domain::diff::{ChangeKind, FileChange};
use crate::domain::review::HunkVerdict;
use crate::tui::theme;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let block = Block::bordered().title(" Files ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.diff.files.is_empty() {
        frame.render_widget(
            Paragraph::new("(no changes)").style(Style::default().fg(theme::gutter_fg())),
            inner,
        );
        return;
    }

    let current = state.current_row().map(|r| r.file());
    let height = inner.height as usize;
    let offset = scroll_offset(current.unwrap_or(0), state.diff.files.len(), height);

    let mut lines = Vec::with_capacity(height);
    for (idx, file) in state
        .diff
        .files
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
    {
        lines.push(file_line(state, file, idx, current == Some(idx)));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn scroll_offset(current: usize, total: usize, height: usize) -> usize {
    if height == 0 || total <= height {
        return 0;
    }
    if current >= height {
        (current + 1 - height).min(total - height)
    } else {
        0
    }
}

fn file_line(state: &AppState, file: &FileChange, idx: usize, selected: bool) -> Line<'static> {
    let (letter, color) = badge(file.change);
    let collapse = if file_collapsed(state, idx) { "▸" } else { "▾" };

    let mut spans = vec![
        Span::raw(format!("{collapse} ")),
        Span::styled(
            format!("{letter} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(file.path.display().to_string()),
        Span::styled(
            format!("  +{} -{}", file.stats.0, file.stats.1),
            Style::default().fg(theme::gutter_fg()),
        ),
    ];
    if let Some(marker) = verdict_rollup(state, file) {
        spans.push(marker);
    }

    let line = Line::from(spans);
    if selected {
        line.style(Style::default().bg(theme::cursor_bg()))
    } else {
        line
    }
}

/// One marker reflecting the file's overall review progress.
fn verdict_rollup(state: &AppState, file: &FileChange) -> Option<Span<'static>> {
    if file.hunks.is_empty() {
        return None;
    }
    let mut approved = 0;
    let mut needs = 0;
    for hunk in &file.hunks {
        match state.review.verdict(&hunk.href) {
            HunkVerdict::Approved => approved += 1,
            HunkVerdict::NeedsAttention => needs += 1,
            HunkVerdict::Unreviewed => {}
        }
    }
    if needs > 0 {
        Some(Span::styled(
            format!("  ✗{needs}"),
            Style::default().fg(theme::needs_attention_fg()),
        ))
    } else if approved == file.hunks.len() {
        Some(Span::styled("  ✓", Style::default().fg(theme::approved_fg())))
    } else if approved > 0 {
        Some(Span::styled("  ·", Style::default().fg(theme::approved_fg())))
    } else {
        None
    }
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
