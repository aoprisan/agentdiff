//! Synthesized diffs for untracked (agent-created) files.
//!
//! libgit2 won't hand us content hunks for untracked files, and agents create a
//! lot of them, so we enumerate them ourselves with the gitignore-aware `ignore`
//! walker and build an "empty → content" diff via `similar` into the same
//! `Hunk`/`Line` model the tracked differ uses.

use std::path::Path;

use ignore::WalkBuilder;
use similar::{ChangeTag, TextDiff};

use super::differ::language_for;
use super::repo::Repo;
use crate::domain::diff::{
    ChangeKind, FileChange, FileId, Hunk, Line, LineKind, LineRange,
};
use crate::domain::ids::fingerprint;
use crate::domain::review::HunkRef;
use crate::error::Result;

/// Append a `FileChange` for every untracked, non-ignored file in the tree. The
/// `FileId`s are placeholders; the caller renumbers after merging with the
/// tracked changes.
pub fn collect(repo: &Repo) -> Result<Vec<FileChange>> {
    let workdir = repo.workdir();
    let index = repo.inner().index()?;
    let mut out = Vec::new();

    // gitignore-aware, but keep dotfiles (git surfaces untracked dotfiles too).
    // `ignore` only skips `.git` when hidden filtering is on, so exclude it
    // explicitly now that we've turned hidden filtering off.
    let walk = WalkBuilder::new(workdir)
        .hidden(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();

    for entry in walk {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let abs = entry.path();
        let Ok(rel) = abs.strip_prefix(workdir) else {
            continue;
        };
        // Tracked files (modified or staged-new) are already covered by the
        // libgit2 diff; we only synthesize for files git doesn't know about.
        if index.get_path(rel, 0).is_some() {
            continue;
        }
        if let Some(change) = synth_created(abs, rel)? {
            out.push(change);
        }
    }
    Ok(out)
}

/// Build an "empty → content" `FileChange` for one created file.
fn synth_created(abs: &Path, rel: &Path) -> Result<Option<FileChange>> {
    let bytes = std::fs::read(abs)?;

    if bytes.contains(&0) {
        return Ok(Some(FileChange {
            id: FileId(0),
            path: rel.to_path_buf(),
            old_path: None,
            change: ChangeKind::Added,
            is_binary: true,
            is_created: true,
            language: language_for(rel),
            hunks: Vec::new(),
            stats: (0, 0),
        }));
    }

    let content = String::from_utf8_lossy(&bytes);
    let mut lines = Vec::new();
    for change in TextDiff::from_lines("", content.as_ref()).iter_all_changes() {
        // Diffing against an empty old side yields only insertions.
        if change.tag() != ChangeTag::Insert {
            continue;
        }
        let new_no = change.new_index().map(|i| i as u32 + 1);
        lines.push(Line {
            kind: LineKind::Added,
            old_no: None,
            new_no,
            text: trim_eol(change.value()).to_string(),
            intra: Vec::new(),
        });
    }

    let added = lines.len();
    let hunks = if added == 0 {
        Vec::new()
    } else {
        let href = HunkRef {
            path: rel.to_path_buf(),
            fingerprint: fingerprint(rel, &lines),
        };
        vec![Hunk {
            href,
            old: LineRange { start: 0, count: 0 },
            new: LineRange {
                start: 1,
                count: added as u32,
            },
            header: format!("@@ -0,0 +1,{added} @@"),
            lines,
        }]
    };

    Ok(Some(FileChange {
        id: FileId(0),
        path: rel.to_path_buf(),
        old_path: None,
        change: ChangeKind::Added,
        is_binary: false,
        is_created: true,
        language: language_for(rel),
        hunks,
        stats: (added, 0),
    }))
}

/// Strip a single trailing line terminator (`\n` or `\r\n`) for display.
fn trim_eol(s: &str) -> &str {
    s.strip_suffix('\n')
        .map(|s| s.strip_suffix('\r').unwrap_or(s))
        .unwrap_or(s)
}
