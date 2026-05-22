use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Paragraph};

/// Centered help overlay, drawn on top of the current view when toggled.
pub fn render(frame: &mut Frame, area: Rect) {
    let popup = centered(area, 52, 45);
    // Clear the cells underneath so the overlay isn't drawn over the panes.
    frame.render_widget(Clear, popup);

    let lines = vec![
        Line::from("agentdiff — keys".bold()),
        Line::from(""),
        Line::from("  ?      toggle this help"),
        Line::from("  Esc    close overlay"),
        Line::from("  q      quit"),
        Line::from(""),
        Line::from("navigation & review keys arrive in later phases".dim()),
    ];
    let block = Block::bordered().title(" Help ");
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let [horizontal] = Layout::horizontal([Constraint::Percentage(width_pct)])
        .flex(Flex::Center)
        .areas(area);
    let [centered] = Layout::vertical([Constraint::Percentage(height_pct)])
        .flex(Flex::Center)
        .areas(horizontal);
    centered
}
