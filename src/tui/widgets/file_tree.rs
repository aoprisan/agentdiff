use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Paragraph};

/// Left pane: the list of changed files. Populated in Phase 1.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::bordered().title(" Files ");
    frame.render_widget(Paragraph::new("(no changes yet)").block(block), area);
}
