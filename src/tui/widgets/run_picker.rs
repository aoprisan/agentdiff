//! Modal overlay listing the loaded session's agent runs. `j`/`k` move,
//! `Enter` re-scopes the diff to the highlighted run, `Esc` closes. Only
//! reachable when the session has more than one run.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::AppState;
use crate::tui::theme;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let runs = state
        .session
        .as_ref()
        .map(|s| s.runs.as_slice())
        .unwrap_or_default();

    let width = 64.min(area.width);
    let height = ((runs.len() + 4) as u16).clamp(5, area.height);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);

    let block = Block::bordered()
        .title(" Agent runs (this session) ")
        .style(Style::default().bg(theme::bg()).fg(theme::fg()));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // Window around the cursor, mirroring the session picker.
    let visible = (inner.height.saturating_sub(2) as usize).max(1);
    let start = state
        .run_picker_cursor
        .saturating_sub(visible - 1)
        .min(runs.len().saturating_sub(visible));

    let mut lines: Vec<Line> = runs
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(i, item)| {
            let pointer = if i == state.run_picker_cursor { "▶ " } else { "  " };
            let current = if item.is_current { "* " } else { "  " };
            let line = Line::from(vec![
                Span::raw(pointer),
                Span::styled(current, Style::default().fg(theme::approved_fg())),
                Span::styled(
                    format!("run {}  ", item.index + 1),
                    Style::default().fg(theme::hunk_header_fg()),
                ),
                Span::raw(item.label.clone()),
            ]);
            if i == state.run_picker_cursor {
                line.style(
                    Style::default()
                        .bg(theme::cursor_bg())
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                line
            }
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "  Enter: review this run    Esc: close",
        Style::default().fg(theme::gutter_fg()),
    ));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [h] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [c] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(h);
    c
}
