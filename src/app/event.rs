use ratatui::crossterm::event::Event;

/// Everything that can drive a state transition. Phase 0 has terminal input and
/// a periodic tick; later phases add filesystem and worker-job results.
#[derive(Debug)]
pub enum AppEvent {
    Input(Event),
    Tick,
}
