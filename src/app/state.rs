use std::collections::HashSet;
use std::path::PathBuf;

use crate::domain::diff::Diff;
use crate::domain::review::{HunkRef, HunkVerdict, ReviewState};
use crate::domain::session::{CommandRun, Intent, Provider};

use super::keymap::Keymap;
use super::rows::{self, FlatDiff, Row};
use crate::session::intent::IntentMap;

/// Top-level screen. Phase 1 has only the review view; the session picker and
/// risk inbox arrive in later phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Review,
}

/// Header metadata for the loaded session, shown above the intent panel.
#[derive(Debug, Clone, Default)]
pub struct SessionSummary {
    /// Which agent produced the session, shown as a badge.
    pub provider: Provider,
    /// Session id, used to locate the transcript to watch for live updates.
    pub id: String,
    pub title: Option<String>,
    pub last_prompt: Option<String>,
    /// Human label for the diff base, e.g. "agent run 2" or "working tree".
    pub base_label: String,
    /// The selected run is still in progress (no closing turn yet).
    pub live: bool,
    /// Shell commands the selected run ran, for the verification badge/overlay.
    /// Empty under the git-only fallback or for a run that ran nothing.
    pub commands: Vec<CommandRun>,
}

/// In-progress per-hunk note edit (a tiny modal input).
#[derive(Debug, Clone)]
pub struct NoteEdit {
    pub href: HunkRef,
    pub buffer: String,
}

/// One row in the session picker.
#[derive(Debug, Clone)]
pub struct SessionListItem {
    pub id: String,
    pub title: Option<String>,
    pub last_prompt: Option<String>,
    pub is_current: bool,
}

/// Aggregate review progress for the status bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReviewCounts {
    pub total: usize,
    pub reviewed: usize,
    pub needs_attention: usize,
    /// Verdicts whose hunk content changed since they were recorded.
    pub changed_since_reviewed: usize,
}

/// The whole application state. Mutated only by `update::update`.
pub struct AppState {
    pub view: View,
    pub should_quit: bool,
    pub show_help: bool,

    pub diff: Diff,
    pub review: ReviewState,
    /// Where the review state is persisted (per repo + base).
    pub state_path: PathBuf,
    /// Set when a verdict/collapse changes, so we only rewrite on real edits.
    pub review_dirty: bool,

    pub flat: FlatDiff,
    pub cursor: usize,
    pub scroll: usize,
    /// Visible line count of the diff pane, refreshed each loop from the
    /// terminal size so paging/scrolling math matches what's on screen.
    pub viewport_height: usize,
    pub pending_key: Option<char>,

    // --- Session integration (Phase 2) ---
    /// Repo-relative path → the agent's stated intent for that file.
    pub intent: IntentMap,
    /// Loaded-session header, or `None` under the git-only fallback.
    pub session: Option<SessionSummary>,
    /// Show the full intent text vs. a compact preview.
    pub intent_detail: bool,
    /// Verification overlay (commands the agent ran) is open.
    pub show_verify: bool,
    /// Sessions for the picker, newest-first.
    pub sessions: Vec<SessionListItem>,
    pub show_picker: bool,
    pub picker_cursor: usize,
    /// Set when the user selects a different session; the run loop reloads it.
    pub pending_switch: Option<String>,

    // --- Live re-diff & notes (Phase 3) ---
    /// Bumps on each re-diff request; a `DiffReady` with a stale generation is
    /// dropped so superseded background diffs never clobber newer state.
    pub generation: u64,
    /// Active per-hunk note editor, or `None`.
    pub note_edit: Option<NoteEdit>,
    /// Search input being typed (`/`), or `None` when the prompt is closed.
    pub search_edit: Option<String>,
    /// Committed search query; `m`/`M` jump between matching rows.
    pub search_query: Option<String>,
    /// Key bindings (defaults + config overrides).
    pub keymap: Keymap,
}

impl AppState {
    pub fn new(diff: Diff, review: ReviewState, state_path: PathBuf) -> Self {
        let flat = FlatDiff::build(&diff, &review);
        Self {
            view: View::Review,
            should_quit: false,
            show_help: false,
            diff,
            review,
            state_path,
            review_dirty: false,
            flat,
            cursor: 0,
            scroll: 0,
            viewport_height: 1,
            pending_key: None,
            intent: IntentMap::new(),
            session: None,
            intent_detail: false,
            show_verify: false,
            sessions: Vec::new(),
            show_picker: false,
            picker_cursor: 0,
            pending_switch: None,
            generation: 0,
            note_edit: None,
            search_edit: None,
            search_query: None,
            keymap: Keymap::default(),
        }
    }

    /// `HunkRef` under the cursor (a hunk header or one of its lines), used to
    /// re-anchor the cursor across a live re-diff and to target note edits.
    pub fn current_hunk_ref(&self) -> Option<HunkRef> {
        let (fi, hi) = self.current_row()?.hunk()?;
        Some(self.diff.files[fi].hunks[hi].href.clone())
    }

    /// Swap in a freshly built diff/intent/session, re-anchoring the cursor to
    /// the same logical hunk (by fingerprint) when it survives the re-diff.
    /// Verdicts and notes re-attach automatically via their `HunkRef` keys.
    pub fn apply_rediff(
        &mut self,
        diff: Diff,
        intent: IntentMap,
        session: Option<SessionSummary>,
        sessions: Vec<SessionListItem>,
    ) {
        let anchor = self.current_hunk_ref();
        self.diff = diff;
        self.intent = intent;
        self.session = session;
        // Preserve the picker's current-session marker across the swap.
        if !sessions.is_empty() {
            self.sessions = sessions;
        }
        self.rebuild_flat();
        if let Some(href) = anchor
            && let Some(row) = self.row_for_hunk(&href)
        {
            self.cursor = row;
        }
        self.ensure_cursor_visible();
    }

    /// First flattened row belonging to the hunk with this `HunkRef`.
    fn row_for_hunk(&self, href: &HunkRef) -> Option<usize> {
        (0..self.flat.len()).find(|&i| {
            self.flat
                .get(i)
                .and_then(|r| r.hunk())
                .and_then(|(fi, hi)| self.diff.files.get(fi)?.hunks.get(hi))
                .is_some_and(|h| &h.href == href)
        })
    }

    /// The agent's intent for the file under the cursor, if any.
    pub fn current_intent(&self) -> Option<&Intent> {
        let row = self.current_row()?;
        let path = &self.diff.files.get(row.file())?.path;
        self.intent.get(path)
    }

    /// Rebuild the flattened rows after the diff or collapse state changes,
    /// keeping the cursor in range.
    pub fn rebuild_flat(&mut self) {
        self.flat = FlatDiff::build(&self.diff, &self.review);
        if self.cursor > self.flat.last_index() {
            self.cursor = self.flat.last_index();
        }
        self.ensure_cursor_visible();
    }

    /// Keep the cursor within the visible window, then clamp the scroll so we
    /// never page past the end of the diff.
    pub fn ensure_cursor_visible(&mut self) {
        let h = self.viewport_height.max(1);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + h {
            self.scroll = self.cursor + 1 - h;
        }
        let max_scroll = self.flat.len().saturating_sub(h);
        self.scroll = self.scroll.min(max_scroll);
    }

    pub fn current_row(&self) -> Option<Row> {
        self.flat.get(self.cursor)
    }

    /// `HunkRef`s the current cursor targets for a verdict: the hunk under the
    /// cursor, or every hunk in the file when the cursor is on a header/summary.
    pub fn targeted_hunks(&self) -> Vec<HunkRef> {
        let Some(row) = self.current_row() else {
            return Vec::new();
        };
        if let Some((fi, hi)) = row.hunk() {
            return self.diff.files[fi]
                .hunks
                .get(hi)
                .map(|h| vec![h.href.clone()])
                .unwrap_or_default();
        }
        // File header / collapsed summary → the whole file.
        self.diff.files[row.file()]
            .hunks
            .iter()
            .map(|h| h.href.clone())
            .collect()
    }

    pub fn counts(&self) -> ReviewCounts {
        let mut present: HashSet<&HunkRef> = HashSet::new();
        let mut counts = ReviewCounts::default();
        for file in &self.diff.files {
            for hunk in &file.hunks {
                present.insert(&hunk.href);
                counts.total += 1;
                match self.review.verdict(&hunk.href) {
                    HunkVerdict::Unreviewed => {}
                    HunkVerdict::Approved => counts.reviewed += 1,
                    HunkVerdict::NeedsAttention => {
                        counts.reviewed += 1;
                        counts.needs_attention += 1;
                    }
                }
            }
        }
        // A recorded verdict whose fingerprint is gone means the hunk's content
        // changed since the human reviewed it.
        counts.changed_since_reviewed = self
            .review
            .verdicts
            .keys()
            .filter(|href| !present.contains(href))
            .count();
        counts
    }
}

/// Effective collapse state for a file, exposed for the file-tree widget.
pub fn file_collapsed(state: &AppState, file_idx: usize) -> bool {
    state
        .diff
        .files
        .get(file_idx)
        .map(|f| rows::is_collapsed(f, &state.review))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Timestamp;
    use crate::domain::diff::{
        ChangeKind, Diff, DiffBase, FileChange, FileId, Hunk, Line, LineKind, LineRange,
    };

    fn hunk(path: &str, fp: u64, text: &str) -> Hunk {
        Hunk {
            href: HunkRef {
                path: path.into(),
                fingerprint: fp,
            },
            old: LineRange { start: 0, count: 0 },
            new: LineRange { start: 1, count: 1 },
            header: format!("@@ {fp} @@"),
            lines: vec![Line {
                kind: LineKind::Added,
                old_no: None,
                new_no: Some(1),
                text: text.into(),
                intra: Vec::new(),
            }],
        }
    }

    fn diff_with(hunks: Vec<Hunk>) -> Diff {
        Diff {
            base: DiffBase::WorkingTreeVsHead,
            generated_at: Timestamp::from_millis(0),
            files: vec![FileChange {
                id: FileId(0),
                path: "a.rs".into(),
                old_path: None,
                change: ChangeKind::Modified,
                is_binary: false,
                is_created: false,
                language: Some("rust".into()),
                hunks,
                stats: (0, 0),
            }],
        }
    }

    #[test]
    fn rediff_reanchors_cursor_and_keeps_verdict() {
        let h1 = hunk("a.rs", 11, "first");
        let h2 = hunk("a.rs", 22, "second");
        let mut state = AppState::new(
            diff_with(vec![h1.clone(), h2.clone()]),
            ReviewState::default(),
            PathBuf::from("/tmp/r.toml"),
        );
        state.viewport_height = 100;

        // Park the cursor on the second hunk and approve it.
        state.cursor = state.row_for_hunk(&h2.href).unwrap();
        state.review.set_verdict(h2.href.clone(), HunkVerdict::Approved);

        // A re-diff prepends a new hunk (h2 shifts down) but keeps h2 unchanged.
        let h0 = hunk("a.rs", 99, "inserted");
        let new_diff = diff_with(vec![h0, h1, h2.clone()]);
        state.apply_rediff(new_diff, IntentMap::new(), None, Vec::new());

        // Cursor stays on the same logical hunk; its verdict survives.
        assert_eq!(state.current_hunk_ref(), Some(h2.href.clone()));
        assert_eq!(state.review.verdict(&h2.href), HunkVerdict::Approved);
        assert_eq!(state.counts().reviewed, 1);
    }
}
