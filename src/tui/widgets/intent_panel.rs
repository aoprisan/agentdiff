use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Paragraph};

/// Right pane: the agent's stated intent for the current file/hunk. Phase 2.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::bordered().title(" Intent ");
    frame.render_widget(Paragraph::new("(no agent session — Phase 2)").block(block), area);
}
