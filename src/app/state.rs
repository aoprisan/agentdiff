use std::collections::HashSet;
use std::path::PathBuf;

use crate::domain::diff::Diff;
use crate::domain::review::{HunkRef, HunkVerdict, ReviewState};

use super::rows::{self, FlatDiff, Row};

/// Top-level screen. Phase 1 has only the review view; the session picker and
/// risk inbox arrive in later phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Review,
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
        }
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
