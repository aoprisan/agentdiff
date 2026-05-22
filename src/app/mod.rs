//! UI-framework-agnostic application core: the state model and the single
//! `(state, event)` reducer. No ratatui types leak out beyond the input `Event`.

mod commands;
mod event;
mod keymap;
mod rows;
mod state;
mod update;

pub use event::AppEvent;
pub use rows::Row;
pub use state::{AppState, View, file_collapsed};
pub use update::update;
