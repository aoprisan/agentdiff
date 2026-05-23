//! Claude Code session integration: parse the transcript, segment autonomous
//! runs, resolve pre-run backups, and recover per-file intent. Depends only on
//! `domain`; all transcript/file-history format knowledge is confined here.
//!
//! Everything is advisory — `load_session` returns `None` (and the app falls
//! back to a plain working-tree diff) whenever data is missing or unparseable.

pub mod backups;
pub mod intent;
pub mod locate;
pub mod runs;
pub mod transcript;

use std::path::Path;

use crate::domain::Timestamp;
use crate::domain::diff::DiffBase;
use crate::domain::session::{AgentRun, AgentSession, RunId};

use intent::IntentMap;

/// Everything Phase 2 derives from one session, ready for the UI.
pub struct SessionContext {
    pub session: AgentSession,
    pub selected_run: Option<RunId>,
    pub intent: IntentMap,
}

impl SessionContext {
    pub fn run(&self, id: RunId) -> Option<&AgentRun> {
        self.session.runs.iter().find(|r| r.id == id)
    }

    pub fn selected(&self) -> Option<&AgentRun> {
        self.selected_run.and_then(|id| self.run(id))
    }

    /// The diff base implied by the selection: an `AgentRun` base only when the
    /// chosen run has a non-empty pre-run snapshot (resolvable backups). When
    /// there's no usable snapshot the caller falls back to working-tree-vs-HEAD.
    pub fn diff_base(&self) -> Option<DiffBase> {
        let run = self.selected()?;
        (!run.snapshot.is_empty()).then(|| DiffBase::AgentRun {
            session: self.session.id.clone(),
            run: run.id,
        })
    }
}

/// Load the session for `repo_root`, honoring an explicit session id / run.
///
/// `claude_dir` is `~/.claude`. Returns `None` when no session exists or the
/// transcript yields no records.
pub fn load_session(
    claude_dir: &Path,
    repo_root: &Path,
    session_id: Option<&str>,
    run_index: Option<u32>,
) -> Option<SessionContext> {
    let entry = match session_id {
        Some(id) => locate::find_session(claude_dir, repo_root, id)?,
        None => locate::list_sessions(claude_dir, repo_root).into_iter().next()?,
    };

    let records = transcript::parse_file(&entry.path).ok()?;
    if records.is_empty() {
        return None;
    }

    let seg = runs::segment(&records);
    let file_history_dir = claude_dir.join("file-history").join(&entry.id.0);

    let runs: Vec<AgentRun> = seg
        .runs
        .iter()
        .enumerate()
        .map(|(i, raw)| AgentRun {
            id: RunId(i as u32),
            mode: raw.mode,
            started: raw.started.unwrap_or(Timestamp(0)),
            ended: raw.ended,
            snapshot: backups::resolve(&raw.raw_backups, &file_history_dir, repo_root),
            edits: raw.edits.clone(),
        })
        .collect();

    let selected_run = select_run(&runs, run_index);

    let session = AgentSession {
        id: entry.id.clone(),
        project_slug: locate::slug_for(repo_root),
        file: entry.path,
        runs,
        last_prompt: seg.last_prompt,
        // Fall back to the first user prompt when there's no AI-generated title.
        title: seg.title.or(seg.first_prompt),
    };

    Some(SessionContext {
        session,
        selected_run,
        intent: intent::build(&records, repo_root),
    })
}

/// Pick the run to review: an explicit index when valid, else the most recent
/// run that has resolvable backups, else the most recent run.
fn select_run(runs: &[AgentRun], run_index: Option<u32>) -> Option<RunId> {
    if let Some(n) = run_index
        && (n as usize) < runs.len()
    {
        return Some(RunId(n));
    }
    runs.iter()
        .rev()
        .find(|r| !r.snapshot.is_empty())
        .or_else(|| runs.last())
        .map(|r| r.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::session::{EditTool, PermissionMode};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/session/sample-session.jsonl"
    );

    fn fixture_records() -> Vec<transcript::Record> {
        transcript::parse_file(Path::new(FIXTURE)).unwrap()
    }

    fn assemble_runs(seg: &runs::Segmentation, file_history: &Path, repo: &Path) -> Vec<AgentRun> {
        seg.runs
            .iter()
            .enumerate()
            .map(|(i, raw)| AgentRun {
                id: RunId(i as u32),
                mode: raw.mode,
                started: raw.started.unwrap_or(Timestamp(0)),
                ended: raw.ended,
                snapshot: backups::resolve(&raw.raw_backups, file_history, repo),
                edits: raw.edits.clone(),
            })
            .collect()
    }

    #[test]
    fn fixture_parses_without_crashing_on_unknown_or_truncated_lines() {
        let records = fixture_records();
        // The 10 well-formed lines parse (one is the `some-future-record` →
        // Other); the truncated trailing line is dropped.
        assert_eq!(records.len(), 11);
        assert!(records.iter().any(|r| matches!(r, transcript::Record::Other)));
    }

    #[test]
    fn fixture_segments_one_accept_edits_run() {
        let seg = runs::segment(&fixture_records());
        assert_eq!(seg.runs.len(), 1);
        let run = &seg.runs[0];
        assert_eq!(run.mode, PermissionMode::AcceptEdits);
        // greet.rs (Write), lib.rs (Edit), and the out-of-repo edit are all in span.
        assert_eq!(run.edits.len(), 3);
        assert_eq!(run.edits[0].tool, EditTool::Write);
        assert!(run.raw_backups.contains_key("/repo/src/greet.rs"));
        assert_eq!(seg.title.as_deref(), Some("Add greeting, fix off-by-one"));
    }

    #[test]
    fn fixture_run_structure_snapshot() {
        let seg = runs::segment(&fixture_records());
        let agent_runs = assemble_runs(&seg, Path::new("/fh"), Path::new("/repo"));
        insta::with_settings!({sort_maps => true}, {
            insta::assert_json_snapshot!(agent_runs);
        });
    }

    #[test]
    fn fixture_intent_map_snapshot_and_scope() {
        let records = fixture_records();
        let map = intent::build(&records, Path::new("/repo"));

        // In-repo edits get intent; the out-of-repo edit is dropped.
        assert!(map.contains_key(Path::new("src/greet.rs")));
        assert!(map.contains_key(Path::new("src/lib.rs")));
        assert!(!map.keys().any(|p| p.to_string_lossy().contains("thing.rs")));

        let sorted: BTreeMap<String, _> = map
            .iter()
            .map(|(k, v)| (k.display().to_string(), v))
            .collect();
        insta::assert_json_snapshot!(sorted);
    }

    #[test]
    fn load_session_end_to_end_selects_run_and_recovers_intent() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = Path::new("/repo");
        let projects = tmp.path().join("projects").join(locate::slug_for(repo));
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::copy(FIXTURE, projects.join("fixture-sid.jsonl")).unwrap();

        let ctx = load_session(tmp.path(), repo, None, None).expect("session loads");

        assert_eq!(ctx.session.runs.len(), 1);
        assert!(ctx.selected_run.is_some());
        assert!(matches!(
            ctx.diff_base(),
            Some(crate::domain::diff::DiffBase::AgentRun { .. })
        ));
        assert!(ctx.intent.contains_key(&PathBuf::from("src/greet.rs")));
        assert!(ctx.intent.contains_key(&PathBuf::from("src/lib.rs")));
        assert_eq!(ctx.session.title.as_deref(), Some("Add greeting, fix off-by-one"));
    }
}
