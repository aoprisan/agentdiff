use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use super::commands::Command;

/// Vim-ish keymap stub. Navigation and review keys arrive in Phase 1; for now
/// only help and quit are bound.
pub fn map_key(key: KeyEvent) -> Command {
    // Ignore key-release / repeat events (Windows reports both).
    if key.kind != KeyEventKind::Press {
        return Command::Noop;
    }
    match key.code {
        KeyCode::Char('q') => Command::Quit,
        KeyCode::Char('?') => Command::ToggleHelp,
        KeyCode::Esc => Command::CloseOverlay,
        _ => Command::Noop,
    }
}
