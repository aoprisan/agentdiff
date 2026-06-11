//! Modal search prompt (`/`). `Enter` commits the query and jumps to the first
//! match; `Esc` cancels. `m` / `M` then walk the matches.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use crate::app::AppState;
use crate::tui::theme;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let Some(buffer) = &state.search_edit else {
        return;
    };

    let width = 64.min(area.width);
    let height = 7.min(area.height);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);

    let block = Block::bordered()
        .title(" Search ")
        .style(Style::default().bg(theme::bg()).fg(theme::fg()));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // A trailing block cursor so the caret is visible.
    let lines = vec![
        Line::styled(format!("/{buffer}\u{2588}"), Style::default()),
        Line::from(""),
        Line::styled(
            "Enter: search    Esc: cancel    then m / M: next / prev match",
            Style::default()
                .fg(theme::gutter_fg())
                .add_modifier(Modifier::DIM),
        ),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
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
