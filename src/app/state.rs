use std::collections::HashSet;
use std::path::PathBuf;

use crate::domain::diff::Diff;
use crate::domain::review::{HunkRef, HunkVerdict, ReviewState};
use crate::domain::session::{CommandRun, Intent, Provider};

use super::keymap::Keymap;
use super::rows::{self, FlatDiff, Row};
use crate::session::intent::{HunkIntentMap, IntentMap};

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

/// A request to open the file under the cursor in the user's editor. Recorded
/// by the reducer; consumed by the run loop, which owns the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditRequest {
    /// Repo-relative path; the run loop joins it onto the workdir.
    pub path: PathBuf,
    /// 1-based line on the new (working-tree) side.
    pub line: u32,
}

/// One row in the session picker.
#[derive(Debug, Clone)]
pub struct SessionListItem {
    pub id: String,
    pub title: Option<String>,
    pub last_prompt: Option<String>,
    pub is_current: bool,
}

/// How precisely an intent is anchored to the cursor position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentScope {
    /// Matched to this exact hunk by the edit's content.
    Hunk,
    /// The file's most recent intent — the coarse fallback.
    File,
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
    /// Hunk → the intent of the specific edit that produced it, matched by
    /// content. Preferred over the per-file map when a hunk is present.
    pub hunk_intent: HunkIntentMap,
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
    /// Visible rows matching `search_query`, for the statusbar. Recounted when
    /// the query commits and whenever the flattened rows are rebuilt.
    pub search_matches: usize,
    /// Set when the user asks to open the cursor's file in their editor; the
    /// run loop suspends the TUI, launches it, and re-diffs on return.
    pub pending_edit: Option<EditRequest>,
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
            hunk_intent: HunkIntentMap::new(),
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
            search_matches: 0,
            pending_edit: None,
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
        hunk_intent: HunkIntentMap,
        session: Option<SessionSummary>,
        sessions: Vec<SessionListItem>,
    ) {
        let anchor = self.current_hunk_ref();
        self.diff = diff;
        self.intent = intent;
        self.hunk_intent = hunk_intent;
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

    /// The agent's intent for the cursor position: the specific edit's intent
    /// when the hunk was content-matched to one, else the file-level fallback.
    pub fn current_intent(&self) -> Option<(&Intent, IntentScope)> {
        if let Some(href) = self.current_hunk_ref()
            && let Some(intent) = self.hunk_intent.get(&href)
        {
            return Some((intent, IntentScope::Hunk));
        }
        let row = self.current_row()?;
        let path = &self.diff.files.get(row.file())?.path;
        self.intent.get(path).map(|i| (i, IntentScope::File))
    }

    /// Rebuild the flattened rows after the diff or collapse state changes,
    /// keeping the cursor in range.
    pub fn rebuild_flat(&mut self) {
        self.flat = FlatDiff::build(&self.diff, &self.review);
        if self.cursor > self.flat.last_index() {
            self.cursor = self.flat.last_index();
        }
        self.recount_matches();
        self.ensure_cursor_visible();
    }

    /// Case-insensitive match of a flattened row against a lowercased needle:
    /// file path for headers/summaries, hunk header text, or the line's text.
    pub fn row_matches(&self, idx: usize, needle: &str) -> bool {
        let Some(row) = self.flat.get(idx) else {
            return false;
        };
        let Some(file) = self.diff.files.get(row.file()) else {
            return false;
        };
        let hay = match row {
            Row::FileHeader { .. } | Row::CollapsedSummary { .. } => {
                file.path.display().to_string()
            }
            Row::HunkHeader { hunk, .. } => file.hunks[hunk].header.clone(),
            Row::Line { hunk, line, .. } => file.hunks[hunk].lines[line].text.clone(),
        };
        hay.to_lowercase().contains(needle)
    }

    /// Refresh `search_matches` for the current query over the visible rows.
    pub fn recount_matches(&mut self) {
        let Some(query) = &self.search_query else {
            self.search_matches = 0;
            return;
        };
        let needle = query.to_lowercase();
        self.search_matches = (0..self.flat.len())
            .filter(|&i| self.row_matches(i, &needle))
            .count();
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
        // A recorded verdict whose fingerprint is gone — while its file still
        // has changes in the diff — means the hunk's content changed since the
        // human reviewed it. A verdict for a file that left the diff entirely
        // is a *resolved* review (committed/reverted), not a changed one; those
        // are pruned on save, not flagged forever.
        let files_present: HashSet<&std::path::Path> =
            self.diff.files.iter().map(|f| f.path.as_path()).collect();
        counts.changed_since_reviewed = self
            .review
            .verdicts
            .keys()
            .filter(|href| !present.contains(href) && files_present.contains(href.path.as_path()))
            .count();
        counts
    }

    /// Garbage-collect verdicts/notes for files that have left the diff. Hunks
    /// that merely changed content keep their entries (that's the "changed
    /// since reviewed" signal); called at save time so transient mid-rewrite
    /// states aren't pruned on every re-diff.
    pub fn prune_review(&mut self) {
        let files: HashSet<std::path::PathBuf> =
            self.diff.files.iter().map(|f| f.path.clone()).collect();
        let before = self.review.verdicts.len() + self.review.notes.len();
        self.review.retain_refs(|href| files.contains(&href.path));
        if self.review.verdicts.len() + self.review.notes.len() != before {
            self.review_dirty = true;
        }
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
                base_fallback: false,
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
        state.apply_rediff(
            new_diff,
            IntentMap::new(),
            HunkIntentMap::new(),
            None,
            Vec::new(),
        );

        // Cursor stays on the same logical hunk; its verdict survives.
        assert_eq!(state.current_hunk_ref(), Some(h2.href.clone()));
        assert_eq!(state.review.verdict(&h2.href), HunkVerdict::Approved);
        assert_eq!(state.counts().reviewed, 1);
    }

    #[test]
    fn changed_count_and_pruning_distinguish_edited_from_departed() {
        let h1 = hunk("a.rs", 11, "first");
        let mut state = AppState::new(
            diff_with(vec![h1.clone()]),
            ReviewState::default(),
            PathBuf::from("/tmp/r.toml"),
        );
        state.viewport_height = 100;
        state.review.set_verdict(h1.href.clone(), HunkVerdict::Approved);
        // A verdict for a hunk in a file that left the diff (committed).
        let departed = HunkRef {
            path: "gone.rs".into(),
            fingerprint: 77,
        };
        state.review.set_verdict(departed.clone(), HunkVerdict::Approved);
        // A verdict whose hunk content changed but whose file is still here.
        let edited = HunkRef {
            path: "a.rs".into(),
            fingerprint: 55,
        };
        state.review.set_verdict(edited.clone(), HunkVerdict::Approved);

        // Only the still-present file's missing fingerprint counts as changed;
        // the committed file does not show up as "changed" forever.
        assert_eq!(state.counts().changed_since_reviewed, 1);

        // Pruning drops the departed file's entry, keeps the changed one.
        state.prune_review();
        assert!(!state.review.verdicts.contains_key(&departed));
        assert!(state.review.verdicts.contains_key(&edited));
        assert!(state.review.verdicts.contains_key(&h1.href));
        assert!(state.review_dirty, "a prune must mark state for saving");
    }

    #[test]
    fn current_intent_prefers_the_hunk_match_over_the_file_fallback() {
        let h = hunk("a.rs", 11, "first");
        let mut state = AppState::new(
            diff_with(vec![h.clone()]),
            ReviewState::default(),
            PathBuf::from("/tmp/r.toml"),
        );
        state.viewport_height = 100;
        let intent_for = |text: &str| Intent {
            file_path: PathBuf::from("a.rs"),
            text: text.into(),
            source_uuid: "u".into(),
            confidence: 0.9,
        };
        state
            .intent
            .insert(PathBuf::from("a.rs"), intent_for("file-level why"));
        state
            .hunk_intent
            .insert(h.href.clone(), intent_for("hunk-level why"));

        state.cursor = state.row_for_hunk(&h.href).unwrap();
        let (intent, scope) = state.current_intent().unwrap();
        assert_eq!(intent.text, "hunk-level why");
        assert_eq!(scope, IntentScope::Hunk);

        state.hunk_intent.clear();
        let (intent, scope) = state.current_intent().unwrap();
        assert_eq!(intent.text, "file-level why");
        assert_eq!(scope, IntentScope::File);
    }
}
