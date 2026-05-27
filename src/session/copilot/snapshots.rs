//! Resolving Copilot's `rewind-snapshots` into the set of files a run touched
//! and the commit its "before" content lives at.
//!
//! `<session>/rewind-snapshots/index.json` records, in chronological order, a
//! list of snapshots and a top-level `filePathMap` from each file key to its
//! absolute path. Unlike Claude's file-history, a Copilot snapshot captures each
//! touched file's content **at snapshot time** under `backups/<hash>` — i.e. the
//! *post*-edit state (a snapshot you can rewind *to*), not the pre-edit content.
//! The earliest snapshot taken at the run's start records the base `gitCommit`
//! with the touched files still clean, so the reliable pre-run content is each
//! file's blob at that commit, *not* any rewind backup. We therefore resolve the
//! run to: the set of in-repo files it touched (from `filePathMap`) plus the
//! base commit (from the earliest snapshot's `gitCommit`), and let the differ
//! read the "before" from git. Keys are relativized to the repo root and
//! out-of-repo entries dropped, mirroring the Claude
//! [`backups`](super::super::backups) path.
//!
//! Limitation: a file the user had *uncommitted* changes to before the run will
//! diff against its committed blob, so those pre-run edits show up alongside the
//! agent's — acceptable for an advisory overlay, and the same as the plain
//! working-tree fallback.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::domain::session::Backup;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Index {
    #[serde(default)]
    pub snapshots: Vec<Snapshot>,
    #[serde(rename = "filePathMap", default)]
    pub file_path_map: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Snapshot {
    #[serde(rename = "gitCommit", default)]
    pub git_commit: Option<String>,
}

impl Index {
    /// The commit a run started from: the earliest snapshot that recorded one.
    /// `None` for an empty/commit-less index, in which case the run has no
    /// resolvable base and the caller falls back to working-tree-vs-HEAD.
    pub fn base_commit(&self) -> Option<&str> {
        self.snapshots.iter().find_map(|s| s.git_commit.as_deref())
    }
}

/// Parse `<session_dir>/rewind-snapshots/index.json`. Returns an empty index for
/// a missing/unreadable/malformed file (session data is advisory).
pub fn parse(session_dir: &Path) -> Index {
    let path = session_dir.join("rewind-snapshots").join("index.json");
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::debug!(error = %e, "unparseable copilot rewind index");
            Index::default()
        }),
        Err(_) => Index::default(),
    }
}

/// The repo-relative set of files the run touched, each marked as having no
/// file backup so the differ sources its pre-run content from the run's
/// `base_commit` blob. Empty when the index records no base commit (→ the
/// caller falls back to a plain working-tree diff).
pub fn resolve(index: &Index, repo_root: &Path) -> HashMap<PathBuf, Backup> {
    let mut out: HashMap<PathBuf, Backup> = HashMap::new();
    if index.base_commit().is_none() {
        return out;
    }
    for abs in index.file_path_map.values() {
        if let Some(rel) = relativize(abs, repo_root) {
            out.insert(
                rel,
                Backup {
                    backup_path: None,
                    version: 0,
                },
            );
        }
    }
    out
}

fn relativize(abs: &str, repo_root: &Path) -> Option<PathBuf> {
    let p = Path::new(abs);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        repo_root.join(p)
    };
    abs.strip_prefix(repo_root)
        .ok()
        .filter(|r| !r.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_json() -> Index {
        let json = r#"{
            "version": 1,
            "snapshots": [
                {"timestamp":"t0","gitCommit":"base000","files":{}},
                {"timestamp":"t1","gitCommit":"later11","files":{
                    "k_a":{"backupFile":"a-v2"},
                    "k_b":{"backupFile":"b-v1"},
                    "k_out":{"backupFile":"out-v1"}
                }}
            ],
            "filePathMap": {
                "k_a":"/repo/src/a.rs",
                "k_b":"/repo/src/b.rs",
                "k_out":"/home/user/outside.rs"
            }
        }"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn base_commit_is_the_earliest_recorded() {
        assert_eq!(index_json().base_commit(), Some("base000"));
    }

    #[test]
    fn touched_in_repo_files_resolve_to_a_git_base() {
        let repo = Path::new("/repo");
        let resolved = resolve(&index_json(), repo);

        // out-of-repo file dropped; two in-repo files kept, both git-based
        // (no backup → differ reads the base_commit blob).
        assert_eq!(resolved.len(), 2);
        assert!(!resolved.keys().any(|p| p.to_string_lossy().contains("outside")));
        assert_eq!(resolved[Path::new("src/a.rs")].backup_path, None);
        assert_eq!(resolved[Path::new("src/b.rs")].backup_path, None);
    }

    #[test]
    fn no_base_commit_yields_empty_so_caller_falls_back() {
        let json = r#"{
            "snapshots": [{"timestamp":"t0","files":{}}],
            "filePathMap": {"k":"/repo/src/a.rs"}
        }"#;
        let index: Index = serde_json::from_str(json).unwrap();
        assert!(index.base_commit().is_none());
        assert!(resolve(&index, Path::new("/repo")).is_empty());
    }

    #[test]
    fn missing_index_is_empty() {
        let resolved = resolve(&Index::default(), Path::new("/repo"));
        assert!(resolved.is_empty());
    }
}
