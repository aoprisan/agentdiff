use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Paragraph};

/// Center pane: the virtualized diff. Implemented in Phase 1.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::bordered().title(" Diff ");
    frame.render_widget(Paragraph::new("(no diff loaded — Phase 1)").block(block), area);
}
