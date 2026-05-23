use ratatui::crossterm::event::Event;

use crate::domain::review::HunkVerdict;

use super::commands::Command;
use super::keymap::{Resolved, resolve};
use super::rows;
use super::{AppEvent, AppState};

/// The single place `AppState` is mutated. Long-running side effects (live
/// re-diff, session parsing) will be requested from here in later phases.
pub fn update(state: &mut AppState, event: AppEvent) {
    match event {
        AppEvent::Input(Event::Key(key)) => match resolve(key, state.pending_key) {
            Resolved::Pending(leader) => state.pending_key = Some(leader),
            Resolved::Command(command) => {
                state.pending_key = None;
                apply(state, command);
            }
        },
        AppEvent::Input(_) => {}
        AppEvent::Tick => {}
    }
}

fn apply(state: &mut AppState, command: Command) {
    // The session picker is a modal overlay that captures navigation.
    if state.show_picker {
        apply_picker(state, command);
        return;
    }

    match command {
        Command::Quit => state.should_quit = true,
        Command::ToggleHelp => state.show_help = !state.show_help,
        Command::CloseOverlay => state.show_help = false,

        Command::OpenSessionPicker => open_picker(state),
        Command::ToggleIntentDetail => state.intent_detail = !state.intent_detail,
        Command::Select => {}

        Command::CursorDown => move_cursor(state, state.cursor + 1),
        Command::CursorUp => move_cursor(state, state.cursor.saturating_sub(1)),
        Command::HalfPageDown => {
            let step = (state.viewport_height / 2).max(1);
            move_cursor(state, state.cursor + step);
        }
        Command::HalfPageUp => {
            let step = (state.viewport_height / 2).max(1);
            move_cursor(state, state.cursor.saturating_sub(step));
        }
        Command::NextHunk => {
            if let Some(r) = state.flat.next_hunk(state.cursor) {
                move_cursor(state, r);
            }
        }
        Command::PrevHunk => {
            if let Some(r) = state.flat.prev_hunk(state.cursor) {
                move_cursor(state, r);
            }
        }
        Command::NextFile => {
            if let Some(r) = state.flat.next_file(state.cursor) {
                move_cursor(state, r);
            }
        }
        Command::PrevFile => {
            if let Some(r) = state.flat.prev_file(state.cursor) {
                move_cursor(state, r);
            }
        }
        Command::GotoTop => move_cursor(state, 0),
        Command::GotoBottom => move_cursor(state, state.flat.last_index()),

        Command::ToggleCollapse => toggle_collapse(state),
        Command::Approve => set_verdict(state, HunkVerdict::Approved),
        Command::NeedsAttention => set_verdict(state, HunkVerdict::NeedsAttention),
        Command::Unset => set_verdict(state, HunkVerdict::Unreviewed),

        Command::Noop => {}
    }
}

fn move_cursor(state: &mut AppState, to: usize) {
    state.cursor = to.min(state.flat.last_index());
    state.ensure_cursor_visible();
}

fn toggle_collapse(state: &mut AppState) {
    let Some(row) = state.current_row() else {
        return;
    };
    let file_idx = row.file();
    let file = &state.diff.files[file_idx];
    let collapsed_now = rows::is_collapsed(file, &state.review);
    state
        .review
        .collapsed
        .insert(file.path.clone(), !collapsed_now);
    state.review_dirty = true;

    state.rebuild_flat();
    // Anchor the cursor to the file we just toggled so it doesn't jump.
    if let Some(r) = state.flat.file_header_row(file_idx) {
        state.cursor = r;
    }
    state.ensure_cursor_visible();
}

fn set_verdict(state: &mut AppState, verdict: HunkVerdict) {
    let targets = state.targeted_hunks();
    if targets.is_empty() {
        return;
    }
    for href in targets {
        state.review.set_verdict(href, verdict);
    }
    state.review_dirty = true;
}

fn open_picker(state: &mut AppState) {
    if state.sessions.is_empty() {
        return;
    }
    state.show_help = false;
    state.show_picker = true;
    state.picker_cursor = state
        .sessions
        .iter()
        .position(|s| s.is_current)
        .unwrap_or(0);
}

/// Command routing while the session picker is open.
fn apply_picker(state: &mut AppState, command: Command) {
    let last = state.sessions.len().saturating_sub(1);
    match command {
        Command::Quit => state.should_quit = true,
        Command::CursorDown => state.picker_cursor = (state.picker_cursor + 1).min(last),
        Command::CursorUp => state.picker_cursor = state.picker_cursor.saturating_sub(1),
        Command::CloseOverlay | Command::OpenSessionPicker => state.show_picker = false,
        Command::Select => {
            if let Some(item) = state.sessions.get(state.picker_cursor)
                && !item.is_current
            {
                state.pending_switch = Some(item.id.clone());
            }
            state.show_picker = false;
        }
        _ => {}
    }
}
