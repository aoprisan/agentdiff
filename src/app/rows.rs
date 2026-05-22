//! Flattening the `Diff` tree into a single virtualizable row index.
//!
//! Rendering only ever touches the visible window of `rows`, so scrolling is
//! O(1) regardless of diff size. Jump tables (`file_header_rows`,
//! `hunk_header_rows`) back the next/prev file and hunk motions. Rebuilt from
//! scratch whenever the diff or the collapse state changes.

use crate::domain::diff::{Diff, FileChange};
use crate::domain::review::ReviewState;

/// Files with more changed lines than this are collapsed by default (generated
/// code, lockfiles); expanding still renders responsively via virtualization.
const HUGE_FILE_LINES: usize = 500;

/// One rendered row. Indices point back into the owning `Diff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    FileHeader { file: usize },
    HunkHeader { file: usize, hunk: usize },
    Line { file: usize, hunk: usize, line: usize },
    /// Stand-in row for a collapsed, binary, or empty file.
    CollapsedSummary { file: usize },
}

impl Row {
    pub fn file(self) -> usize {
        match self {
            Row::FileHeader { file }
            | Row::HunkHeader { file, .. }
            | Row::Line { file, .. }
            | Row::CollapsedSummary { file } => file,
        }
    }

    /// `(file, hunk)` when the row belongs to a specific hunk.
    pub fn hunk(self) -> Option<(usize, usize)> {
        match self {
            Row::HunkHeader { file, hunk } | Row::Line { file, hunk, .. } => Some((file, hunk)),
            _ => None,
        }
    }
}

/// The flattened diff plus its jump tables.
pub struct FlatDiff {
    rows: Vec<Row>,
    file_header_rows: Vec<usize>,
    hunk_header_rows: Vec<usize>,
}

impl FlatDiff {
    pub fn build(diff: &Diff, review: &ReviewState) -> FlatDiff {
        let mut rows = Vec::new();
        let mut file_header_rows = Vec::new();
        let mut hunk_header_rows = Vec::new();

        for (fi, file) in diff.files.iter().enumerate() {
            file_header_rows.push(rows.len());
            rows.push(Row::FileHeader { file: fi });

            if is_collapsed(file, review) || file.hunks.is_empty() {
                rows.push(Row::CollapsedSummary { file: fi });
                continue;
            }
            for (hi, hunk) in file.hunks.iter().enumerate() {
                hunk_header_rows.push(rows.len());
                rows.push(Row::HunkHeader { file: fi, hunk: hi });
                for li in 0..hunk.lines.len() {
                    rows.push(Row::Line {
                        file: fi,
                        hunk: hi,
                        line: li,
                    });
                }
            }
        }

        FlatDiff {
            rows,
            file_header_rows,
            hunk_header_rows,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn last_index(&self) -> usize {
        self.rows.len().saturating_sub(1)
    }

    pub fn get(&self, idx: usize) -> Option<Row> {
        self.rows.get(idx).copied()
    }

    /// Row index of the header for `file`, if present.
    pub fn file_header_row(&self, file: usize) -> Option<usize> {
        self.file_header_rows.get(file).copied()
    }

    pub fn next_hunk(&self, cursor: usize) -> Option<usize> {
        self.hunk_header_rows.iter().copied().find(|&r| r > cursor)
    }

    pub fn prev_hunk(&self, cursor: usize) -> Option<usize> {
        self.hunk_header_rows
            .iter()
            .copied()
            .rev()
            .find(|&r| r < cursor)
    }

    pub fn next_file(&self, cursor: usize) -> Option<usize> {
        self.file_header_rows.iter().copied().find(|&r| r > cursor)
    }

    pub fn prev_file(&self, cursor: usize) -> Option<usize> {
        self.file_header_rows
            .iter()
            .copied()
            .rev()
            .find(|&r| r < cursor)
    }
}

/// Effective collapse state: an explicit user choice wins, otherwise created,
/// binary, and huge files start collapsed.
pub fn is_collapsed(file: &FileChange, review: &ReviewState) -> bool {
    review
        .collapsed
        .get(&file.path)
        .copied()
        .unwrap_or_else(|| collapsed_by_default(file))
}

pub fn collapsed_by_default(file: &FileChange) -> bool {
    file.is_binary || file.is_created || total_lines(file) > HUGE_FILE_LINES
}

fn total_lines(file: &FileChange) -> usize {
    file.hunks.iter().map(|h| h.lines.len()).sum()
}
