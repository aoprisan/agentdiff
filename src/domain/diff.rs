//! The diff spine: a `Diff` is a list of `FileChange`s, each a list of `Hunk`s,
//! each a list of `Line`s. Risk, intent, and review state all anchor onto the
//! content-addressed [`HunkRef`] inside each hunk.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::Timestamp;
use crate::domain::review::HunkRef;
use crate::domain::session::{RunId, SessionId};

/// Stable-within-a-diff identifier for a changed file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileId(pub u32);

/// Which "before" the working tree was diffed against. The base only changes how
/// a `Diff` is *built*; everything downstream is identical.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffBase {
    WorkingTreeVsHead,
    WorkingTreeVsIndex,
    Range { from: String, to: String },
    AgentRun { session: SessionId, run: RunId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// A `(start, count)` span of 1-based line numbers, as in a unified-diff header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    pub start: u32,
    pub count: u32,
}

/// A byte range within a line's text, marking an intra-line (word-diff) change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineSpan {
    pub start: usize,
    pub end: usize,
    /// `true` if this span differs from the counterpart line.
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
    /// Word-diff spans, computed at model-build time (Phase 1).
    pub intra: Vec<InlineSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    pub href: HunkRef,
    pub old: LineRange,
    pub new: LineRange,
    pub header: String,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub id: FileId,
    pub path: PathBuf,
    /// Set for renames/copies.
    pub old_path: Option<PathBuf>,
    pub change: ChangeKind,
    pub is_binary: bool,
    /// `true` when the agent created this file (no prior version existed).
    pub is_created: bool,
    /// `true` when the intended pre-run base (a file-history backup or the
    /// run's base-commit blob) was unavailable and the "before" side fell back
    /// to `HEAD` (or empty). The diff is still shown, but labeled as degraded
    /// rather than silently misrendered as an agent-created file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub base_fallback: bool,
    pub language: Option<String>,
    pub hunks: Vec<Hunk>,
    /// `(added, removed)` line counts.
    pub stats: (usize, usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diff {
    pub base: DiffBase,
    pub files: Vec<FileChange>,
    pub generated_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Timestamp;
    use crate::domain::review::HunkRef;

    fn sample_diff() -> Diff {
        Diff {
            base: DiffBase::WorkingTreeVsHead,
            generated_at: Timestamp::from_millis(1_700_000_000_000),
            files: vec![FileChange {
                id: FileId(0),
                path: PathBuf::from("src/lib.rs"),
                old_path: None,
                change: ChangeKind::Modified,
                is_binary: false,
                is_created: false,
                base_fallback: false,
                language: Some("rust".into()),
                stats: (1, 1),
                hunks: vec![Hunk {
                    href: HunkRef {
                        path: PathBuf::from("src/lib.rs"),
                        fingerprint: 42,
                    },
                    old: LineRange { start: 1, count: 1 },
                    new: LineRange { start: 1, count: 1 },
                    header: "@@ -1 +1 @@".into(),
                    lines: vec![
                        Line {
                            kind: LineKind::Removed,
                            old_no: Some(1),
                            new_no: None,
                            text: "let x = 1;".into(),
                            intra: vec![InlineSpan {
                                start: 8,
                                end: 9,
                                changed: true,
                            }],
                        },
                        Line {
                            kind: LineKind::Added,
                            old_no: None,
                            new_no: Some(1),
                            text: "let x = 2;".into(),
                            intra: vec![InlineSpan {
                                start: 8,
                                end: 9,
                                changed: true,
                            }],
                        },
                    ],
                }],
            }],
        }
    }

    #[test]
    fn diff_round_trips_through_serde() {
        let diff = sample_diff();
        let json = serde_json::to_string(&diff).expect("serialize Diff");
        let back: Diff = serde_json::from_str(&json).expect("deserialize Diff");
        assert_eq!(diff, back);
    }

    #[test]
    fn diff_json_snapshot() {
        insta::assert_json_snapshot!(sample_diff());
    }
}
