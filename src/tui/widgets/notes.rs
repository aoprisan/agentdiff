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
    // Grow with the note's lines (border + content + blank + hint), capped so
    // a long note scrolls out of view instead of eating the screen.
    let note_lines = edit.buffer.split('\n').count() as u16;
    let height = (note_lines + 4).clamp(7, 14).min(area.height);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);

    let path = edit.href.path.display().to_string();
    let block = Block::bordered()
        .title(format!(" Note — {path} "))
        .style(Style::default().bg(theme::bg()).fg(theme::fg()));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // One rendered line per buffer line, with a block cursor on the last so
    // the caret stays visible across Alt+Enter newlines.
    let mut buffer_lines: Vec<String> = edit.buffer.split('\n').map(str::to_string).collect();
    if let Some(last) = buffer_lines.last_mut() {
        last.push('\u{2588}');
    }
    let mut lines: Vec<Line> = buffer_lines
        .into_iter()
        .map(|l| Line::styled(l, Style::default()))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "Enter: save    Alt-Enter: newline    Esc: cancel",
        Style::default()
            .fg(theme::gutter_fg())
            .add_modifier(Modifier::DIM),
    ));
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
