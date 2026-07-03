//! GitHub Copilot CLI session integration.
//!
//! Copilot stores each session under `~/.copilot/session-state/<uuid>/`:
//! - `events.jsonl` — the transcript (see [`events`]),
//! - `rewind-snapshots/{index.json,backups/<hash>}` — pre-edit file content
//!   (see [`snapshots`]).
//!
//! We discover the session(s) whose `session.start` context matches the repo,
//! aggregate the events into one reviewable run (see [`runs`]), attach the
//! folded rewind backups as the pre-run snapshot, and recover per-file intent
//! (see [`intent`]). The result is the same [`SessionContext`](super::SessionContext)
//! the Claude path produces, so everything downstream is shared. As with Claude,
//! all of this is advisory: any missing/unparseable data yields `None` and the
//! app falls back to a plain working-tree diff.

pub mod events;
pub mod intent;
pub mod runs;
pub mod snapshots;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::domain::Timestamp;
use crate::domain::session::{AgentRun, AgentSession, Provider, RunId, SessionId};

use super::paths::RepoPaths;
use super::{SessionContext, locate, select_run};

/// The default `~/.copilot` directory, if a home directory can be resolved.
pub fn default_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().join(".copilot"))
}

/// Lines of `events.jsonl` scanned when discovering a session's repo identity.
const META_SCAN_LINES: usize = 64;

/// A discovered Copilot session directory.
#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub id: SessionId,
    /// `<copilot>/session-state/<uuid>`.
    pub dir: PathBuf,
    /// `<dir>/events.jsonl`.
    pub events: PathBuf,
    pub modified: SystemTime,
}

/// All Copilot sessions whose `session.start` context matches `repo_root`,
/// newest-first by the transcript's mtime.
pub fn list_sessions(copilot_dir: &Path, repo_root: &Path) -> Vec<SessionEntry> {
    let root = copilot_dir.join("session-state");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let target = RepoPaths::new(repo_root);

    let mut sessions: Vec<SessionEntry> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let dir = e.path();
            if !dir.is_dir() {
                return None;
            }
            let events = dir.join("events.jsonl");
            let modified = std::fs::metadata(&events).ok()?.modified().ok()?;
            let (cwd, git_root) = read_repo_identity(&events)?;
            if !matches_repo(&target, cwd.as_deref(), git_root.as_deref()) {
                return None;
            }
            let id = dir.file_name()?.to_str()?.to_string();
            Some(SessionEntry {
                id: SessionId(id),
                dir,
                events,
                modified,
            })
        })
        .collect();

    sessions.sort_by(|a, b| b.modified.cmp(&a.modified));
    sessions
}

/// Locate a specific Copilot session by id for `repo_root`.
pub fn find_session(copilot_dir: &Path, repo_root: &Path, id: &str) -> Option<SessionEntry> {
    list_sessions(copilot_dir, repo_root)
        .into_iter()
        .find(|s| s.id.0 == id)
}

/// A cheap picker label: the session's first user prompt. Reads only the head of
/// the transcript rather than fully parsing it.
pub fn peek_title(events_path: &Path) -> Option<String> {
    let file = std::fs::File::open(events_path).ok()?;
    for line in BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .take(META_SCAN_LINES)
    {
        if let Ok(event) = serde_json::from_str::<events::RawEvent>(line.trim())
            && let Some(text) = event.user_text()
        {
            return Some(text);
        }
    }
    None
}

/// Load the Copilot session for `repo_root`, honoring an explicit id / run.
pub fn load_session(
    copilot_dir: &Path,
    repo_root: &Path,
    session_id: Option<&str>,
    run_index: Option<u32>,
) -> Option<SessionContext> {
    let entry = match session_id {
        Some(id) => find_session(copilot_dir, repo_root, id)?,
        None => list_sessions(copilot_dir, repo_root).into_iter().next()?,
    };

    let events = events::parse_file(&entry.events).ok()?;
    if events.is_empty() {
        return None;
    }

    let seg = runs::segment(&events);
    let index = snapshots::parse(&entry.dir);
    let base_commit = index.base_commit().map(str::to_string);

    let runs: Vec<AgentRun> = seg
        .runs
        .iter()
        .enumerate()
        .map(|(i, raw)| AgentRun {
            id: RunId(i as u32),
            mode: raw.mode,
            started: raw.started.unwrap_or(Timestamp(0)),
            ended: raw.ended,
            snapshot: snapshots::resolve(&index, repo_root),
            base_commit: base_commit.clone(),
            edits: raw.edits.clone(),
            commands: raw.commands.clone(),
        })
        .collect();

    let selected_run = select_run(&runs, run_index);

    let session = AgentSession {
        id: entry.id.clone(),
        provider: Provider::Copilot,
        project_slug: locate::slug_for(repo_root),
        file: entry.events,
        runs,
        last_prompt: seg.last_prompt,
        title: seg.first_prompt,
    };

    Some(SessionContext {
        session,
        selected_run,
        intent: intent::build(&events, repo_root),
        // Copilot rewind snapshots don't carry per-edit content, so hunk-level
        // correlation has nothing to match on; per-file intent still applies.
        edit_intents: Vec::new(),
    })
}

/// `(cwd, git_root)` from the head `session.start` event of a transcript.
fn read_repo_identity(events_path: &Path) -> Option<(Option<String>, Option<String>)> {
    let file = std::fs::File::open(events_path).ok()?;
    for line in BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .take(META_SCAN_LINES)
    {
        let Ok(event) = serde_json::from_str::<events::RawEvent>(line.trim()) else {
            continue;
        };
        if let Some((cwd, git_root)) = event.start_context() {
            return Some((cwd.map(str::to_string), git_root.map(str::to_string)));
        }
    }
    None
}

/// Whether a session's recorded `cwd`/`git_root` belongs to the repo: an exact
/// `git_root`/`cwd` match (symlink-tolerant), or a `cwd` nested inside it.
fn matches_repo(target: &RepoPaths, cwd: Option<&str>, git_root: Option<&str>) -> bool {
    if git_root.is_some_and(|g| target.matches_root(g)) {
        return true;
    }
    cwd.is_some_and(|c| target.contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::diff::DiffBase;
    use crate::domain::session::{CommandOutcome, PermissionMode};
    use std::path::PathBuf;

    const FIXTURE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/copilot");
    /// The repo root the fixture's `session.start` records.
    const REPO: &str = "/repo";

    fn copilot_dir() -> PathBuf {
        PathBuf::from(FIXTURE_ROOT)
    }

    #[test]
    fn matches_repo_by_git_root_or_nested_cwd() {
        let target = RepoPaths::new(Path::new("/repo"));
        assert!(matches_repo(&target, Some("/repo"), Some("/repo")));
        assert!(matches_repo(&target, Some("/repo/src"), Some("/repo")));
        assert!(matches_repo(&target, Some("/repo"), None));
        assert!(!matches_repo(&target, Some("/other"), Some("/other")));
        assert!(!matches_repo(&target, Some("/repository"), None));
    }

    #[test]
    fn discovers_and_loads_the_fixture_session() {
        let repo = Path::new(REPO);
        let listed = list_sessions(&copilot_dir(), repo);
        assert_eq!(listed.len(), 1, "the one fixture session matches /repo");
        assert_eq!(listed[0].id.0, "sid-1111");

        let ctx = load_session(&copilot_dir(), repo, None, None).expect("session loads");
        assert_eq!(ctx.session.provider, Provider::Copilot);
        assert_eq!(ctx.session.runs.len(), 1);
        assert_eq!(ctx.session.runs[0].mode, PermissionMode::Autopilot);

        // The rewind snapshot resolves to a usable agent-run base.
        assert!(matches!(ctx.diff_base(), Some(DiffBase::AgentRun { .. })));

        // Per-file intent recovered for the in-repo edit.
        assert!(ctx.intent.contains_key(&PathBuf::from("src/lib.rs")));

        // Verification command survived with its outcome.
        let run = ctx.selected().expect("a run is selected");
        assert_eq!(run.commands.len(), 1);
        assert_eq!(run.commands[0].outcome, CommandOutcome::Ok);
    }

    #[test]
    fn peek_title_reads_first_user_prompt() {
        let events = copilot_dir()
            .join("session-state")
            .join("sid-1111")
            .join("events.jsonl");
        assert_eq!(peek_title(&events).as_deref(), Some("tidy up lib.rs"));
    }
}
