//! Modal note editor for the current hunk. `Enter` saves, `Esc` cancels.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use crate::app::AppState;
use crate::tui::theme;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let Some(edit) = &state.note_edit else {
        return;
    };

    let width = 64.min(area.width);
    let height = 7.min(area.height);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);

    let path = edit.href.path.display().to_string();
    let block = Block::bordered().title(format!(" Note — {path} "));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // A trailing block cursor so the caret is visible.
    let lines = vec![
        Line::styled(format!("{}\u{2588}", edit.buffer), Style::default()),
        Line::from(""),
        Line::styled(
            "Enter: save    Esc: cancel",
            Style::default()
                .fg(theme::GUTTER_FG)
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
