//! Bottom status bar: diff base + live indicator, review progress, key hints.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::AppState;
use crate::domain::diff::DiffBase;
use crate::tui::theme;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let counts = state.counts();

    let mut spans = vec![
        " agentdiff ".bold(),
        Span::raw("  "),
        Span::styled(base_label(&state.diff.base), Style::default().fg(theme::HUNK_HEADER_FG)),
    ];
    if state.session.as_ref().is_some_and(|s| s.live) {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            "● live",
            Style::default()
                .fg(theme::REMOVED_FG)
                .add_modifier(Modifier::BOLD),
        ));
    }

    spans.push(Span::raw("   "));
    spans.push(Span::styled(
        format!("reviewed {}/{}", counts.reviewed, counts.total),
        Style::default().fg(theme::APPROVED_FG),
    ));
    if counts.needs_attention > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("✗ {}", counts.needs_attention),
            Style::default().fg(theme::NEEDS_ATTENTION_FG),
        ));
    }
    if counts.changed_since_reviewed > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("⚠ {} changed", counts.changed_since_reviewed),
            Style::default()
                .fg(theme::CHANGED_SINCE_FG)
                .add_modifier(Modifier::BOLD),
        ));
    }

    spans.push(Span::raw("    "));
    for (key, label) in [("a", "approve"), ("x", "flag"), ("n", "note"), ("?", "help"), ("q", "quit")] {
        spans.push(Span::styled(key, Style::default().add_modifier(Modifier::BOLD)));
        spans.push(format!(" {label}  ").dim());
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn base_label(base: &DiffBase) -> String {
    match base {
        DiffBase::WorkingTreeVsHead => "working tree".to_string(),
        DiffBase::WorkingTreeVsIndex => "staged".to_string(),
        DiffBase::Range { from, to } => format!("{from}..{to}"),
        DiffBase::AgentRun { run, .. } => format!("agent run {}", run.0 + 1),
    }
}
