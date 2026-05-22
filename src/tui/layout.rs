use ratatui::layout::{Constraint, Layout, Rect};

/// The three review panes plus the status bar, for a given full-frame area.
pub struct Panes {
    pub file_tree: Rect,
    pub diff: Rect,
    pub intent: Rect,
    pub status: Rect,
}

pub fn compute(area: Rect) -> Panes {
    let [main, status] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
    let [file_tree, diff, intent] = Layout::horizontal([
        Constraint::Percentage(28),
        Constraint::Percentage(44),
        Constraint::Percentage(28),
    ])
    .areas(main);

    Panes {
        file_tree,
        diff,
        intent,
        status,
    }
}
