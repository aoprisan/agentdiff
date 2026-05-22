use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

/// Bottom status bar: identity + key hints. Review counts arrive in Phase 1.
pub fn render(frame: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        " agentdiff ".bold(),
        "   q ".into(),
        "quit".dim(),
        "   ? ".into(),
        "help".dim(),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}
