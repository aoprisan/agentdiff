use ratatui::crossterm::event::Event;

use super::bootstrap::DiffBundle;

/// Everything that can drive a state transition. Phase 3 adds filesystem change
/// notifications and completed background re-diffs.
pub enum AppEvent {
    Input(Event),
    Tick,
    /// The working tree or the active transcript changed on disk.
    FsChanged,
    /// A background re-diff finished. Dropped if `generation` is stale.
    DiffReady {
        generation: u64,
        bundle: Box<DiffBundle>,
    },
}
