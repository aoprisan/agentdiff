//! Locating Claude Code sessions for a working directory.
//!
//! Claude Code stores transcripts at `<claude>/projects/<slug>/<uuid>.jsonl`,
//! where `slug` is the absolute cwd with every `/` and `.` rewritten to `-`.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::domain::session::SessionId;

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

/// All sessions for `cwd`'s project, newest-first by file mtime.
pub fn list_sessions(claude_dir: &Path, cwd: &Path) -> Vec<SessionEntry> {
    let dir = claude_dir.join("projects").join(slug_for(cwd));
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut sessions: Vec<SessionEntry> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_string();
            let modified = e.metadata().ok()?.modified().ok()?;
            Some(SessionEntry {
                id: SessionId(id),
                path,
                modified,
            })
        })
        .collect();

    sessions.sort_by(|a, b| b.modified.cmp(&a.modified));
    sessions
}

/// Locate a specific session by id within `cwd`'s project.
pub fn find_session(claude_dir: &Path, cwd: &Path, id: &str) -> Option<SessionEntry> {
    list_sessions(claude_dir, cwd)
        .into_iter()
        .find(|s| s.id.0 == id)
}

/// A cheap human label for a session, for the picker. Reads only the `ai-title`
/// and `last-prompt` lines rather than fully parsing the transcript.
#[derive(Debug, Clone, Default)]
pub struct SessionLabel {
    pub title: Option<String>,
    pub last_prompt: Option<String>,
}

pub fn peek_label(path: &Path) -> SessionLabel {
    use std::io::{BufRead, BufReader};

    let Ok(file) = std::fs::File::open(path) else {
        return SessionLabel::default();
    };
    let mut label = SessionLabel::default();
    for line in BufReader::new(file).lines().map_while(std::result::Result::ok) {
        if !line.contains("\"ai-title\"") && !line.contains("\"last-prompt\"") {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            match value.get("type").and_then(|t| t.as_str()) {
                Some("ai-title") => {
                    label.title = value
                        .get("aiTitle")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                }
                Some("last-prompt") => {
                    label.last_prompt = value
                        .get("lastPrompt")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                }
                _ => {}
            }
        }
    }
    label
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
}
