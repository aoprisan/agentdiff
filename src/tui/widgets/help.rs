use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Paragraph};

/// Centered help overlay, drawn on top of the current view when toggled.
pub fn render(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from("agentdiff — keys".bold()),
        Line::from(""),
        Line::from("  j / k        line down / up"),
        Line::from("  C-d / C-u    half page down / up"),
        Line::from("  ]c / [c      next / prev hunk"),
        Line::from("  } / {        next / prev file"),
        Line::from("  gg / G       top / bottom"),
        Line::from("  Space        collapse / expand file"),
        Line::from(""),
        Line::from("  a            approve hunk"),
        Line::from("  x            flag (needs attention)"),
        Line::from("  u            clear verdict"),
        Line::from(""),
        Line::from("  n            add / edit note"),
        Line::from("  s            session picker"),
        Line::from("  i            toggle intent detail"),
        Line::from(""),
        Line::from("  ?            toggle this help"),
        Line::from("  Esc          close overlay"),
        Line::from("  q            quit"),
    ];

    // Size the popup to its content, clamped to the available area.
    let width = 46.min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = centered(area, width, height);
    // Clear the cells underneath so the overlay isn't drawn over the panes.
    frame.render_widget(Clear, popup);

    let block = Block::bordered().title(" Help ");
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [horizontal] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [centered] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(horizontal);
    centered
}
