/// Top-level screen. Phase 0 has only the review view; the session picker and
/// risk inbox arrive in later phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Review,
}

/// The whole application state. Mutated only by `update::update`.
#[derive(Debug)]
pub struct AppState {
    pub view: View,
    pub should_quit: bool,
    pub show_help: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            view: View::Review,
            should_quit: false,
            show_help: false,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
