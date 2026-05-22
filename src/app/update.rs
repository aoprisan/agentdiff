use ratatui::crossterm::event::Event;

use super::commands::Command;
use super::keymap::map_key;
use super::{AppEvent, AppState};

/// The single place `AppState` is mutated. Side effects (git, session, watch)
/// will be requested from here in later phases.
pub fn update(state: &mut AppState, event: AppEvent) {
    match event {
        AppEvent::Input(Event::Key(key)) => apply(state, map_key(key)),
        AppEvent::Input(_) => {}
        AppEvent::Tick => {}
    }
}

fn apply(state: &mut AppState, command: Command) {
    match command {
        Command::Quit => state.should_quit = true,
        Command::ToggleHelp => state.show_help = !state.show_help,
        Command::CloseOverlay => state.show_help = false,
        Command::Noop => {}
    }
}
