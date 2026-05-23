//! Modal overlay listing this project's sessions newest-first. `j`/`k` move,
//! `Enter` switches to the highlighted session, `Esc` closes.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::AppState;
use crate::tui::theme;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let width = 72.min(area.width);
    let height = ((state.sessions.len() + 4) as u16).clamp(5, area.height);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);

    let block = Block::bordered().title(" Sessions (newest first) ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let usable = inner.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = state
        .sessions
        .iter()
        .enumerate()
        .map(|(i, item)| session_line(item, i == state.picker_cursor, usable))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "  Enter: switch    Esc: close",
        Style::default().fg(theme::GUTTER_FG),
    ));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn session_line(
    item: &crate::app::SessionListItem,
    selected: bool,
    width: usize,
) -> Line<'static> {
    let pointer = if selected { "▶ " } else { "  " };
    let current = if item.is_current { "* " } else { "  " };
    let short_id: String = item.id.chars().take(8).collect();
    let label = item
        .title
        .clone()
        .or_else(|| item.last_prompt.clone())
        .unwrap_or_else(|| "(untitled)".to_string());
    let label = truncate(&label, width.saturating_sub(16));

    let line = Line::from(vec![
        Span::raw(pointer),
        Span::styled(current, Style::default().fg(theme::APPROVED_FG)),
        Span::styled(format!("{short_id}  "), Style::default().fg(theme::HUNK_HEADER_FG)),
        Span::raw(label),
    ]);
    if selected {
        line.style(
            Style::default()
                .bg(theme::CURSOR_BG)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        line
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
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
