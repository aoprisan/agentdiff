//! Bottom status bar: review progress on the left, key hints on the right.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::AppState;
use crate::tui::theme;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let counts = state.counts();

    let mut spans = vec![
        " agentdiff ".bold(),
        Span::raw("  "),
        Span::styled(
            format!("reviewed {}/{}", counts.reviewed, counts.total),
            Style::default().fg(theme::APPROVED_FG),
        ),
    ];
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
            format!("⚠ {} changed since reviewed", counts.changed_since_reviewed),
            Style::default()
                .fg(theme::CHANGED_SINCE_FG)
                .add_modifier(Modifier::BOLD),
        ));
    }

    spans.push(Span::raw("     "));
    for (key, label) in [("a", "approve"), ("x", "flag"), ("?", "help"), ("q", "quit")] {
        spans.push(Span::styled(key, Style::default().add_modifier(Modifier::BOLD)));
        spans.push(format!(" {label}   ").dim());
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
