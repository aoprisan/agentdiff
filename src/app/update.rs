use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind};

use crate::domain::review::HunkVerdict;

use super::commands::Command;
use super::keymap::Resolved;
use super::rows;
use super::state::NoteEdit;
use super::{AppEvent, AppState};

/// The single place `AppState` is mutated for input/tick. Filesystem and
/// re-diff events are handled in the run loop, which owns the channels.
pub fn update(state: &mut AppState, event: AppEvent) {
    match event {
        AppEvent::Input(Event::Key(key)) => {
            // While editing a note, keystrokes are text, not commands.
            if state.note_edit.is_some() {
                edit_note_key(state, key);
                return;
            }
            match state.keymap.resolve(key, state.pending_key) {
                Resolved::Pending(leader) => state.pending_key = Some(leader),
                Resolved::Command(command) => {
                    state.pending_key = None;
                    apply(state, command);
                }
            }
        }
        AppEvent::Input(_) => {}
        AppEvent::Tick => {}
        // Filesystem / re-diff events are driven by the run loop, not the reducer.
        AppEvent::FsChanged | AppEvent::DiffReady { .. } => {}
    }
}

/// Route a keypress into the active note editor.
fn edit_note_key(state: &mut AppState, key: ratatui::crossterm::event::KeyEvent) {
    if key.kind != KeyEventKind::Press {
        return;
    }
    let Some(edit) = state.note_edit.as_mut() else {
        return;
    };
    match key.code {
        KeyCode::Esc => state.note_edit = None,
        KeyCode::Enter => commit_note(state),
        KeyCode::Backspace => {
            edit.buffer.pop();
        }
        KeyCode::Char(c) => edit.buffer.push(c),
        _ => {}
    }
}

fn commit_note(state: &mut AppState) {
    if let Some(edit) = state.note_edit.take() {
        let trimmed = edit.buffer.trim();
        if trimmed.is_empty() {
            state.review.notes.remove(&edit.href);
        } else {
            state.review.notes.insert(edit.href, trimmed.to_string());
        }
        state.review_dirty = true;
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
        Command::CloseOverlay => {
            state.show_help = false;
            state.show_verify = false;
        }

        Command::OpenSessionPicker => open_picker(state),
        Command::ToggleIntentDetail => state.intent_detail = !state.intent_detail,
        Command::ToggleVerification => toggle_verification(state),
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
        Command::NextUnreviewed => jump_unreviewed(state, true),
        Command::PrevUnreviewed => jump_unreviewed(state, false),
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
        Command::EditNote => open_note_editor(state),

        Command::Noop => {}
    }
}

/// Open the note editor for the hunk under the cursor, seeded with any existing
/// note.
fn open_note_editor(state: &mut AppState) {
    let Some(href) = state.current_hunk_ref() else {
        return;
    };
    let buffer = state.review.notes.get(&href).cloned().unwrap_or_default();
    state.note_edit = Some(NoteEdit { href, buffer });
}

/// Move to the nearest hunk header without a verdict, wrapping past the ends so
/// the motion always finds whatever is left to review. Hunks inside collapsed
/// files have no header row and are skipped, like the other motions.
fn jump_unreviewed(state: &mut AppState, forward: bool) {
    let unreviewed = |r: &usize| -> bool {
        state
            .flat
            .get(*r)
            .and_then(|row| row.hunk())
            .and_then(|(fi, hi)| state.diff.files.get(fi)?.hunks.get(hi))
            .is_some_and(|h| state.review.verdict(&h.href) == HunkVerdict::Unreviewed)
    };
    let rows = state.flat.hunk_rows();
    let cursor = state.cursor;
    let target = if forward {
        let (before, after): (Vec<usize>, Vec<usize>) = rows.iter().partition(|&&r| r <= cursor);
        after
            .iter()
            .find(|r| unreviewed(r))
            .or_else(|| before.iter().find(|r| unreviewed(r)))
            .copied()
    } else {
        let (before, after): (Vec<usize>, Vec<usize>) = rows.iter().partition(|&&r| r < cursor);
        before
            .iter()
            .rev()
            .find(|r| unreviewed(r))
            .or_else(|| after.iter().rev().find(|r| unreviewed(r)))
            .copied()
    };
    if let Some(r) = target {
        move_cursor(state, r);
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

/// Toggle the verification overlay, closing the help overlay if it was up.
fn toggle_verification(state: &mut AppState) {
    state.show_verify = !state.show_verify;
    if state.show_verify {
        state.show_help = false;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Timestamp;
    use crate::domain::diff::{
        ChangeKind, Diff, DiffBase, FileChange, FileId, Hunk, Line, LineKind, LineRange,
    };
    use crate::domain::review::{HunkRef, ReviewState};
    use std::path::PathBuf;

    fn hunk(fp: u64) -> Hunk {
        Hunk {
            href: HunkRef {
                path: PathBuf::from("a.rs"),
                fingerprint: fp,
            },
            old: LineRange { start: 0, count: 0 },
            new: LineRange { start: 1, count: 1 },
            header: format!("@@ {fp} @@"),
            lines: vec![Line {
                kind: LineKind::Added,
                old_no: None,
                new_no: Some(1),
                text: "x".into(),
                intra: Vec::new(),
            }],
        }
    }

    /// One file with three hunks: rows are FileHeader, then (HunkHeader, Line)
    /// per hunk — hunk headers sit at rows 1, 3, and 5.
    fn three_hunk_state() -> AppState {
        let diff = Diff {
            base: DiffBase::WorkingTreeVsHead,
            generated_at: Timestamp::from_millis(0),
            files: vec![FileChange {
                id: FileId(0),
                path: PathBuf::from("a.rs"),
                old_path: None,
                change: ChangeKind::Modified,
                is_binary: false,
                is_created: false,
                language: None,
                hunks: vec![hunk(1), hunk(2), hunk(3)],
                stats: (3, 0),
            }],
        };
        let mut state = AppState::new(diff, ReviewState::default(), PathBuf::from("/tmp/r.toml"));
        state.viewport_height = 100;
        state
    }

    #[test]
    fn next_unreviewed_skips_verdicted_hunks() {
        let mut state = three_hunk_state();
        let second = state.diff.files[0].hunks[1].href.clone();
        state.review.set_verdict(second, HunkVerdict::Approved);

        apply(&mut state, Command::NextUnreviewed);
        assert_eq!(state.cursor, 1); // first hunk header
        apply(&mut state, Command::NextUnreviewed);
        assert_eq!(state.cursor, 5); // third — the approved second is skipped
    }

    #[test]
    fn next_unreviewed_wraps_past_the_end() {
        let mut state = three_hunk_state();
        apply(&mut state, Command::GotoBottom);
        apply(&mut state, Command::NextUnreviewed);
        assert_eq!(state.cursor, 1);
    }

    #[test]
    fn prev_unreviewed_walks_backwards_and_wraps() {
        let mut state = three_hunk_state();
        apply(&mut state, Command::PrevUnreviewed);
        assert_eq!(state.cursor, 5); // wraps from the top to the last hunk
        apply(&mut state, Command::PrevUnreviewed);
        assert_eq!(state.cursor, 3);
    }

    #[test]
    fn next_unreviewed_stays_put_when_everything_is_reviewed() {
        let mut state = three_hunk_state();
        let hrefs: Vec<_> = state.diff.files[0]
            .hunks
            .iter()
            .map(|h| h.href.clone())
            .collect();
        for href in hrefs {
            state.review.set_verdict(href, HunkVerdict::Approved);
        }
        state.cursor = 3;
        apply(&mut state, Command::NextUnreviewed);
        assert_eq!(state.cursor, 3);
    }
}
