//! Resolving Copilot's `rewind-snapshots` into a pre-run backup map.
//!
//! `<session>/rewind-snapshots/index.json` records, in chronological order, a
//! list of snapshots; each captures the verbatim content of the files touched so
//! far under `backups/<hash>`, and a top-level `filePathMap` maps each file key
//! to its absolute path. A file's **earliest** snapshot holds its content before
//! the agent first edited it — so we keep the first backup seen per file (later
//! snapshots re-capture an already-edited file at its current content). Keys are
//! relativized to the repo root and anything outside is dropped, mirroring the
//! Claude [`backups`](super::super::backups) path. Produces the same
//! `HashMap<PathBuf, Backup>` the differ consumes; no file content is read here.

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
    #[serde(default)]
    pub files: HashMap<String, SnapshotFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SnapshotFile {
    #[serde(rename = "backupFile", default)]
    pub backup_file: Option<String>,
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

/// Fold the index into a repo-relative pre-run map: each file's earliest backup,
/// resolved to `<session_dir>/rewind-snapshots/backups/<hash>`.
pub fn resolve(index: &Index, session_dir: &Path, repo_root: &Path) -> HashMap<PathBuf, Backup> {
    let backups_dir = session_dir.join("rewind-snapshots").join("backups");
    let mut out: HashMap<PathBuf, Backup> = HashMap::new();
    // Track the first version index at which each file key was captured.
    let mut version: HashMap<&str, u32> = HashMap::new();

    for (snap_index, snapshot) in index.snapshots.iter().enumerate() {
        for (key, file) in &snapshot.files {
            let Some(backup_name) = &file.backup_file else {
                continue; // no backup captured for this file in this snapshot
            };
            // Earliest snapshot wins: skip a file already seen.
            if version.contains_key(key.as_str()) {
                continue;
            }
            let Some(abs) = index.file_path_map.get(key) else {
                continue; // unknown file key
            };
            let Some(rel) = relativize(abs, repo_root) else {
                continue; // out-of-repo entry → drop
            };
            version.insert(key.as_str(), snap_index as u32);
            out.insert(
                rel,
                Backup {
                    backup_path: Some(backups_dir.join(backup_name)),
                    version: snap_index as u32,
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
                {"timestamp":"t0","files":{
                    "k_a":{"backupFile":"a-v1"}
                }},
                {"timestamp":"t1","files":{
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
    fn keeps_earliest_backup_relativizes_and_drops_out_of_repo() {
        let repo = Path::new("/repo");
        let session = Path::new("/copilot/session-state/sid");
        let resolved = resolve(&index_json(), session, repo);

        // out-of-repo file dropped; two in-repo files kept.
        assert_eq!(resolved.len(), 2);
        assert!(!resolved.keys().any(|p| p.to_string_lossy().contains("outside")));

        // a.rs keeps its v1 backup, not the later re-baselined v2.
        let a = &resolved[Path::new("src/a.rs")];
        let expected = session.join("rewind-snapshots").join("backups").join("a-v1");
        assert_eq!(a.backup_path.as_deref(), Some(expected.as_path()));
        assert_eq!(a.version, 0);

        // b.rs first appears in the second snapshot.
        assert_eq!(resolved[Path::new("src/b.rs")].version, 1);
    }

    #[test]
    fn missing_index_is_empty() {
        let resolved = resolve(&Index::default(), Path::new("/x"), Path::new("/repo"));
        assert!(resolved.is_empty());
    }
}
