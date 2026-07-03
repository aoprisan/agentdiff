//! Locating Claude Code sessions for a working directory.
//!
//! Claude Code stores transcripts at `<claude>/projects/<slug>/<uuid>.jsonl`,
//! where `slug` is the absolute cwd with every `/` and `.` rewritten to `-`.
//! The slug is both lossy (`/a/b.c` and `/a/b/c` collide) and spelled from the
//! cwd the agent saw (which may reach the repo through a symlink), so listing
//! checks the slug for every root spelling and verifies each candidate against
//! the `cwd` recorded inside its transcript.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::domain::session::SessionId;

use super::paths::RepoPaths;

/// The default `~/.claude` directory, if a home directory can be resolved.
pub fn default_claude_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().join(".claude"))
}

/// Project slug for an absolute path: `/` and `.` become `-`. A trailing
/// separator is stripped first (git2's `workdir()` includes one), so the slug
/// matches Claude Code's, which is computed from the bare cwd.
pub fn slug_for(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .trim_end_matches('/')
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect()
}

/// A discovered session transcript.
#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub id: SessionId,
    pub path: PathBuf,
    pub modified: SystemTime,
}

/// All sessions for `cwd`'s project, newest-first by file mtime. Candidates
/// come from the slug of every root spelling (as-given and symlink-resolved);
/// a session whose transcript records a `cwd` outside the repo is dropped
/// (slug collision with a different project).
pub fn list_sessions(claude_dir: &Path, cwd: &Path) -> Vec<SessionEntry> {
    let paths = RepoPaths::new(cwd);
    let mut seen = HashSet::new();
    let mut sessions: Vec<SessionEntry> = Vec::new();

    for root in paths.roots() {
        let dir = claude_dir.join("projects").join(slug_for(root));
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.filter_map(|e| e.ok()) {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
                continue;
            };
            if !seen.insert(id.clone()) {
                continue; // same session reachable via both slugs
            }
            let Some(modified) = e.metadata().ok().and_then(|m| m.modified().ok()) else {
                continue;
            };
            sessions.push(SessionEntry {
                id: SessionId(id),
                path,
                modified,
            });
        }
    }

    sessions.retain(|s| match &peek_meta(&s.path).cwd {
        // The slug is lossy; a recorded cwd outside the repo means the session
        // belongs to a colliding project. No recorded cwd → benefit of the doubt.
        Some(recorded) => paths.contains(recorded),
        None => true,
    });

    sessions.sort_by(|a, b| b.modified.cmp(&a.modified));
    sessions
}

/// Locate a specific session by id within `cwd`'s project.
pub fn find_session(claude_dir: &Path, cwd: &Path, id: &str) -> Option<SessionEntry> {
    list_sessions(claude_dir, cwd)
        .into_iter()
        .find(|s| s.id.0 == id)
}

/// Cheap session metadata for the picker and project verification, read
/// without fully parsing the transcript.
#[derive(Debug, Clone, Default)]
pub struct SessionMeta {
    pub title: Option<String>,
    pub last_prompt: Option<String>,
    /// The first `cwd` recorded on a transcript line.
    pub cwd: Option<String>,
}

/// Scan a transcript for its metadata, cached by mtime — live re-diffs re-list
/// sessions every debounce, and re-reading every transcript end-to-end each
/// time is the difference between O(one live file) and O(all history).
pub fn peek_meta(path: &Path) -> SessionMeta {
    static CACHE: OnceLock<Mutex<std::collections::HashMap<PathBuf, (SystemTime, SessionMeta)>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);

    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Some(mtime) = mtime
        && let Ok(map) = cache.lock()
        && let Some((cached_mtime, meta)) = map.get(path)
        && *cached_mtime == mtime
    {
        return meta.clone();
    }

    let meta = scan_meta(path);
    if let Some(mtime) = mtime
        && let Ok(mut map) = cache.lock()
    {
        map.insert(path.to_path_buf(), (mtime, meta.clone()));
    }
    meta
}

fn scan_meta(path: &Path) -> SessionMeta {
    use std::io::{BufRead, BufReader};

    let Ok(file) = std::fs::File::open(path) else {
        return SessionMeta::default();
    };
    let mut meta = SessionMeta::default();
    for line in BufReader::new(file).lines().map_while(std::result::Result::ok) {
        let wants_label = line.contains("\"ai-title\"") || line.contains("\"last-prompt\"");
        let wants_cwd = meta.cwd.is_none() && line.contains("\"cwd\"");
        if !wants_label && !wants_cwd {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if meta.cwd.is_none() {
            meta.cwd = value
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
        match value.get("type").and_then(|t| t.as_str()) {
            Some("ai-title") => {
                meta.title = value
                    .get("aiTitle")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            }
            Some("last-prompt") => {
                meta.last_prompt = value
                    .get("lastPrompt")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            }
            _ => {}
        }
    }
    meta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_rewrites_separators_and_dots() {
        assert_eq!(slug_for(Path::new("/home/user/agentdiff")), "-home-user-agentdiff");
        assert_eq!(slug_for(Path::new("/a/b.c/d")), "-a-b-c-d");
        // git2's workdir() ends with a separator; the slug must not gain a `-`.
        assert_eq!(slug_for(Path::new("/home/user/agentdiff/")), "-home-user-agentdiff");
    }

    #[test]
    fn lists_sessions_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = Path::new("/proj");
        let project = dir.path().join("projects").join(slug_for(cwd));
        std::fs::create_dir_all(&project).unwrap();

        std::fs::write(project.join("a.jsonl"), "{}\n").unwrap();
        std::fs::write(project.join("b.jsonl"), "{}\n").unwrap();
        std::fs::write(project.join("ignore.txt"), "not a session").unwrap();

        let listed = list_sessions(dir.path(), cwd);
        // Only the two `.jsonl` files are sessions, returned newest-first.
        assert_eq!(listed.len(), 2);
        assert!(listed[0].modified >= listed[1].modified);
        assert!(find_session(dir.path(), cwd, "a").is_some());
        assert!(find_session(dir.path(), cwd, "missing").is_none());
    }

    #[test]
    fn drops_sessions_whose_recorded_cwd_is_another_project() {
        // `/a/b.c` and `/a/b/c` share the slug `-a-b-c`; the recorded cwd
        // disambiguates.
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("projects").join("-a-b-c");
        std::fs::create_dir_all(&project).unwrap();

        std::fs::write(
            project.join("ours.jsonl"),
            r#"{"type":"user","uuid":"u1","cwd":"/a/b.c","message":{"content":"hi"}}"#,
        )
        .unwrap();
        std::fs::write(
            project.join("theirs.jsonl"),
            r#"{"type":"user","uuid":"u1","cwd":"/a/b/c","message":{"content":"hi"}}"#,
        )
        .unwrap();
        std::fs::write(project.join("nocwd.jsonl"), "{}\n").unwrap();

        let listed = list_sessions(dir.path(), Path::new("/a/b.c"));
        let ids: Vec<_> = listed.iter().map(|s| s.id.0.as_str()).collect();
        assert!(ids.contains(&"ours"));
        assert!(!ids.contains(&"theirs"), "colliding project must be dropped");
        // No recorded cwd → kept (benefit of the doubt).
        assert!(ids.contains(&"nocwd"));
    }

    #[cfg(unix)]
    #[test]
    fn finds_sessions_recorded_under_the_symlink_resolved_root() {
        // The repo is opened via a symlink, but Claude Code recorded its cwd
        // (and thus the project slug) via the real path.
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real-repo");
        std::fs::create_dir_all(&real).unwrap();
        let link = tmp.path().join("link-repo");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let claude = tmp.path().join("claude");
        let project = claude.join("projects").join(slug_for(&real));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("s1.jsonl"), "{}\n").unwrap();

        let listed = list_sessions(&claude, &link);
        assert_eq!(listed.len(), 1, "session found via the canonical root's slug");
        assert_eq!(listed[0].id.0, "s1");
    }
}
