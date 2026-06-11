//! UI-framework-agnostic application core: the state model and the single
//! `(state, event)` reducer. No ratatui types leak out beyond the input `Event`.

pub mod bootstrap;
mod commands;
mod event;
mod keymap;
pub mod report;
mod rows;
pub mod state;
mod update;

pub use bootstrap::{Selectors, build_bundle, build_state};
pub use event::AppEvent;
pub use keymap::Keymap;
pub use rows::Row;
pub use state::{AppState, SessionListItem, View, file_collapsed};
pub use update::update;
