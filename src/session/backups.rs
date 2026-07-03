//! Resolving a run's raw `trackedFileBackups` into a usable pre-run map.
//!
//! Path keys can be absolute or relative and may point outside the repo
//! (`~/.claude/plans/*`, a top-level `CLAUDE.md`, …). We normalize each to an
//! absolute path, re-relativize it to the repo root, and **drop anything that
//! falls outside the repo**. A `null` backup file name marks an agent-created
//! file (no prior version). Pure path mapping — no filesystem access — so the
//! differ owns reading the backup blobs and tolerating any that are missing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::session::Backup;

use super::paths::RepoPaths;
use super::transcript::TrackedBackup;

/// Resolve raw per-path backups to a repo-relative pre-run map.
///
/// `file_history_dir` is `<claude>/file-history/<session-id>`; backup file names
/// resolve against it. `repo_root` is the absolute working-tree root.
pub fn resolve(
    raw: &HashMap<String, TrackedBackup>,
    file_history_dir: &Path,
    repo_root: &Path,
) -> HashMap<PathBuf, Backup> {
    let paths = RepoPaths::new(repo_root);
    let mut out = HashMap::new();
    for (key, tracked) in raw {
        let Some(rel) = paths.relativize(key) else {
            continue; // out-of-repo entry → drop
        };
        let backup_path = tracked
            .backup_file_name
            .as_ref()
            .map(|name| file_history_dir.join(name));
        out.insert(
            rel,
            Backup {
                backup_path,
                version: tracked.version,
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(name: Option<&str>, version: u32) -> TrackedBackup {
        TrackedBackup {
            backup_file_name: name.map(str::to_string),
            version,
        }
    }

    #[test]
    fn relativizes_in_repo_drops_out_of_repo_and_marks_created() {
        let repo = Path::new("/repo");
        let fh = Path::new("/claude/file-history/sid");

        let mut input = HashMap::new();
        input.insert("/repo/src/a.rs".into(), raw(Some("a.rs.bak"), 2));
        input.insert("/repo/new.rs".into(), raw(None, 0)); // created
        input.insert("/home/user/CLAUDE.md".into(), raw(Some("c.bak"), 1)); // out of repo
        input.insert("relative/b.rs".into(), raw(Some("b.rs.bak"), 1)); // relative key

        let resolved = resolve(&input, fh, repo);

        assert_eq!(resolved.len(), 3); // CLAUDE.md dropped
        assert!(!resolved.contains_key(Path::new("CLAUDE.md")));

        let a = &resolved[Path::new("src/a.rs")];
        assert_eq!(a.backup_path.as_deref(), Some(fh.join("a.rs.bak").as_path()));
        assert_eq!(a.version, 2);

        // null backup file name ⇒ agent-created (no pre-run content).
        assert!(resolved[Path::new("new.rs")].backup_path.is_none());

        // relative keys resolve against the repo root.
        assert!(resolved.contains_key(Path::new("relative/b.rs")));
    }
}
