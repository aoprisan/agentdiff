use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind};

use crate::domain::diff::ChangeKind;
use crate::domain::review::HunkVerdict;

use super::commands::Command;
use super::keymap::Resolved;
use super::rows;
use super::state::{EditRequest, NoteEdit};
use super::{AppEvent, AppState};

/// The single place `AppState` is mutated for input/tick. Filesystem and
/// re-diff events are handled in the run loop, which owns the channels.
pub fn update(state: &mut AppState, event: AppEvent) {
    match event {
        AppEvent::Input(Event::Key(key)) => {
            // While editing a note or a search query, keystrokes are text,
            // not commands.
            if state.note_edit.is_some() {
                edit_note_key(state, key);
                return;
            }
            if state.search_edit.is_some() {
                edit_search_key(state, key);
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

/// Route a keypress into the active note editor. `Alt+Enter` inserts a
/// newline; plain `Enter` saves.
fn edit_note_key(state: &mut AppState, key: ratatui::crossterm::event::KeyEvent) {
    if key.kind != KeyEventKind::Press {
        return;
    }
    let Some(edit) = state.note_edit.as_mut() else {
        return;
    };
    let alt = key
        .modifiers
        .contains(ratatui::crossterm::event::KeyModifiers::ALT);
    match key.code {
        KeyCode::Esc => state.note_edit = None,
        KeyCode::Enter if alt => edit.buffer.push('\n'),
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
            state.search_query = None;
            state.search_matches = 0;
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

        Command::OpenSearch => state.search_edit = Some(String::new()),
        Command::NextMatch => jump_match(state, true),
        Command::PrevMatch => jump_match(state, false),
        Command::OpenEditor => request_edit(state),

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

/// Record a request to open the cursor's file in the user's editor, targeting
/// the new-side line so the editor lands where the file is on disk now. Deleted
/// and binary files have nothing useful to open.
fn request_edit(state: &mut AppState) {
    let Some(row) = state.current_row() else {
        return;
    };
    let Some(file) = state.diff.files.get(row.file()) else {
        return;
    };
    if file.change == ChangeKind::Deleted || file.is_binary {
        return;
    }
    let line = match row {
        rows::Row::Line { hunk, line, .. } => {
            let l = &file.hunks[hunk].lines[line];
            // A removed line has no new-side number; fall back to the hunk's
            // new-side start, which is where the deletion happened.
            l.new_no.unwrap_or(file.hunks[hunk].new.start)
        }
        rows::Row::HunkHeader { hunk, .. } => file.hunks[hunk].new.start,
        rows::Row::FileHeader { .. } | rows::Row::CollapsedSummary { .. } => 1,
    };
    state.pending_edit = Some(EditRequest {
        path: file.path.clone(),
        line: line.max(1),
    });
}

/// Route a keypress into the search prompt. `Enter` commits the query and jumps
/// to the first match at or after the cursor; `Esc` cancels.
fn edit_search_key(state: &mut AppState, key: ratatui::crossterm::event::KeyEvent) {
    if key.kind != KeyEventKind::Press {
        return;
    }
    let Some(buffer) = state.search_edit.as_mut() else {
        return;
    };
    match key.code {
        KeyCode::Esc => state.search_edit = None,
        KeyCode::Enter => commit_search(state),
        KeyCode::Backspace => {
            buffer.pop();
        }
        KeyCode::Char(c) => buffer.push(c),
        _ => {}
    }
}

fn commit_search(state: &mut AppState) {
    let Some(buffer) = state.search_edit.take() else {
        return;
    };
    let query = buffer.trim().to_string();
    if query.is_empty() {
        state.search_query = None;
        state.recount_matches();
        return;
    }
    let needle = query.to_lowercase();
    state.search_query = Some(query);
    state.recount_matches();
    if !state.row_matches(state.cursor, &needle) {
        jump_match(state, true);
    }
}

/// Move to the next/previous row matching the committed query, wrapping.
fn jump_match(state: &mut AppState, forward: bool) {
    let Some(query) = &state.search_query else {
        return;
    };
    let needle = query.to_lowercase();
    let n = state.flat.len();
    if n == 0 {
        return;
    }
    let target = (1..=n)
        .map(|step| {
            if forward {
                (state.cursor + step) % n
            } else {
                (state.cursor + n - step) % n
            }
        })
        .find(|&idx| state.row_matches(idx, &needle));
    if let Some(idx) = target {
        move_cursor(state, idx);
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

    use ratatui::crossterm::event::{Event, KeyEvent, KeyModifiers};

    fn press(state: &mut AppState, code: KeyCode) {
        update(
            state,
            AppEvent::Input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))),
        );
    }

    #[test]
    fn search_commits_and_jumps_to_the_matching_row() {
        let mut state = three_hunk_state();
        press(&mut state, KeyCode::Char('/'));
        assert_eq!(state.search_edit.as_deref(), Some(""));

        // Hunk headers are "@@ <fp> @@"; "@@ 3" only matches the third hunk.
        for c in "@@ 3".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        press(&mut state, KeyCode::Enter);

        assert_eq!(state.search_edit, None);
        assert_eq!(state.search_query.as_deref(), Some("@@ 3"));
        assert_eq!(state.cursor, 5); // third hunk header
    }

    #[test]
    fn match_motions_wrap_and_esc_clears_the_query() {
        let mut state = three_hunk_state();
        state.search_query = Some("@@".into());

        apply(&mut state, Command::NextMatch);
        assert_eq!(state.cursor, 1);
        apply(&mut state, Command::NextMatch);
        assert_eq!(state.cursor, 3);
        apply(&mut state, Command::PrevMatch);
        assert_eq!(state.cursor, 1);
        apply(&mut state, Command::PrevMatch);
        assert_eq!(state.cursor, 5); // wraps backwards to the last header

        press(&mut state, KeyCode::Esc);
        assert_eq!(state.search_query, None);
    }

    #[test]
    fn esc_cancels_the_search_prompt_without_committing() {
        let mut state = three_hunk_state();
        press(&mut state, KeyCode::Char('/'));
        press(&mut state, KeyCode::Char('z'));
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.search_edit, None);
        assert_eq!(state.search_query, None);
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn open_editor_targets_the_cursor_line() {
        let mut state = three_hunk_state();
        state.cursor = 4; // the added line of the second hunk (new_no = 1)
        apply(&mut state, Command::OpenEditor);
        assert_eq!(
            state.pending_edit,
            Some(EditRequest {
                path: PathBuf::from("a.rs"),
                line: 1,
            })
        );
    }

    #[test]
    fn open_editor_clamps_zero_line_hunks_and_skips_deleted_files() {
        let mut state = three_hunk_state();
        // A pure-deletion hunk can have new.start == 0; the editor needs 1-based.
        state.diff.files[0].hunks[0].new.start = 0;
        state.cursor = 1; // hunk header
        apply(&mut state, Command::OpenEditor);
        assert_eq!(state.pending_edit.as_ref().map(|e| e.line), Some(1));

        state.pending_edit = None;
        state.diff.files[0].change = ChangeKind::Deleted;
        apply(&mut state, Command::OpenEditor);
        assert_eq!(state.pending_edit, None);
    }

    #[test]
    fn alt_enter_builds_a_multiline_note() {
        let mut state = three_hunk_state();
        state.cursor = 1; // first hunk header
        apply(&mut state, Command::EditNote);

        for c in "first".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        update(
            &mut state,
            AppEvent::Input(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::ALT,
            ))),
        );
        for c in "second".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        press(&mut state, KeyCode::Enter);

        let href = state.diff.files[0].hunks[0].href.clone();
        assert_eq!(state.review.notes.get(&href).map(String::as_str), Some("first\nsecond"));
        assert!(state.note_edit.is_none());
    }

    #[test]
    fn match_count_tracks_commits_and_row_rebuilds() {
        let mut state = three_hunk_state();
        press(&mut state, KeyCode::Char('/'));
        for c in "@@".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.search_matches, 3); // one header per hunk

        // Collapsing the file removes the hunk rows; the count follows.
        apply(&mut state, Command::GotoTop);
        apply(&mut state, Command::ToggleCollapse);
        assert_eq!(state.search_matches, 0);

        press(&mut state, KeyCode::Esc);
        assert_eq!(state.search_matches, 0);
        assert_eq!(state.search_query, None);
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
