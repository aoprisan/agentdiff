//! Composition root for a review session: open the repo, choose a diff base
//! (range / staged / latest agent run / working-tree fallback), build the diff,
//! recover intent, and load persisted verdicts into a ready-to-render
//! [`AppState`]. [`build_bundle`] is reused by the live re-diff worker.

use std::path::Path;

use anyhow::Context;

use crate::config;
use crate::domain::diff::{Diff, DiffBase};
use crate::domain::session::{PermissionMode, Provider};
use crate::git::{self, Repo};
use crate::session::intent::{self as intent_mod, HunkIntentMap, IntentMap};
use crate::session::{self, AgentDirs, SessionContext, copilot, locate};

use super::state::{AppState, SessionListItem, SessionSummary};

/// What the user asked for, owned so it can be cloned to the worker thread.
#[derive(Debug, Clone, Default)]
pub struct Selectors {
    /// Which agent's session data to read (default: Claude Code).
    pub provider: Provider,
    pub no_session: bool,
    pub session_id: Option<String>,
    pub run_index: Option<u32>,
    pub range: Option<(String, String)>,
    pub staged: bool,
}

impl Selectors {
    /// Resolve CLI args into the owned selectors threaded through re-diffs.
    pub fn from_args(args: &crate::cli::Args) -> Self {
        Selectors {
            provider: args.provider(),
            no_session: args.no_session,
            session_id: args.session.clone(),
            run_index: args.run,
            range: args.range.as_deref().map(parse_range),
            staged: args.staged,
        }
    }
}

/// Parse a `--range` argument into `(from, to)`, defaulting the missing side to
/// `HEAD` (so `HEAD~3` means `HEAD~3..HEAD`).
fn parse_range(range: &str) -> (String, String) {
    match range.split_once("..") {
        Some((from, to)) => (
            if from.is_empty() { "HEAD" } else { from }.to_string(),
            if to.is_empty() { "HEAD" } else { to }.to_string(),
        ),
        None => (range.to_string(), "HEAD".to_string()),
    }
}

/// The diff plus its session overlay — everything a re-diff replaces, leaving
/// review verdicts/notes (keyed by fingerprint) and cursor state untouched.
pub struct DiffBundle {
    pub diff: Diff,
    pub intent: IntentMap,
    /// Hunk-level intent, matched by edit content; falls back to `intent`.
    pub hunk_intent: HunkIntentMap,
    pub session: Option<SessionSummary>,
    pub sessions: Vec<SessionListItem>,
}

/// Build the diff bundle for the current selectors. No persisted-state or
/// terminal I/O, so it is safe to run on the worker thread.
pub fn build_bundle(
    repo: &Repo,
    dirs: &AgentDirs,
    selectors: &Selectors,
) -> anyhow::Result<DiffBundle> {
    if let Some((from, to)) = &selectors.range {
        let diff = git::differ::diff_range(repo, from, to)
            .with_context(|| format!("diffing range {from}..{to}"))?;
        return Ok(DiffBundle::git_only(diff));
    }
    if selectors.staged {
        let diff = git::differ::diff_worktree_vs_index(repo).context("diffing staged changes")?;
        return Ok(DiffBundle::git_only(diff));
    }
    if selectors.no_session {
        return Ok(DiffBundle::git_only(worktree(repo)?));
    }

    match session::load(
        selectors.provider,
        dirs,
        repo.workdir(),
        selectors.session_id.as_deref(),
        selectors.run_index,
    ) {
        Some(ctx) => {
            let diff = build_diff_for(repo, &ctx)?;
            let session = Some(summarize(&ctx, &diff.base));
            let sessions =
                session_items(selectors.provider, dirs, repo.workdir(), Some(&ctx.session.id.0));
            // Hunk-level correlation needs both the diff and the edits, so it
            // happens here rather than in `session::load`.
            let hunk_intent = intent_mod::correlate(&diff, &ctx.edit_intents);
            Ok(DiffBundle {
                diff,
                intent: ctx.intent,
                hunk_intent,
                session,
                sessions,
            })
        }
        None => Ok(DiffBundle {
            diff: worktree(repo)?,
            intent: IntentMap::new(),
            hunk_intent: HunkIntentMap::new(),
            session: None,
            sessions: session_items(selectors.provider, dirs, repo.workdir(), None),
        }),
    }
}

/// Build the full initial [`AppState`]: a bundle plus persisted review verdicts.
pub fn build_state(
    repo: &Repo,
    state_dir: &Path,
    dirs: &AgentDirs,
    selectors: &Selectors,
) -> anyhow::Result<AppState> {
    let bundle = build_bundle(repo, dirs, selectors)?;
    tracing::info!(
        files = bundle.diff.files.len(),
        intent_files = bundle.intent.len(),
        base = ?bundle.diff.base,
        live = bundle.session.as_ref().is_some_and(|s| s.live),
        "built review state"
    );

    let state_path = config::review_state_path(state_dir, repo.workdir(), &bundle.diff.base);
    let review = config::load_review_state(&state_path);

    let mut state = AppState::new(bundle.diff, review, state_path);
    state.intent = bundle.intent;
    state.hunk_intent = bundle.hunk_intent;
    state.session = bundle.session;
    state.sessions = bundle.sessions;
    Ok(state)
}

impl DiffBundle {
    fn git_only(diff: Diff) -> Self {
        DiffBundle {
            diff,
            intent: IntentMap::new(),
            hunk_intent: HunkIntentMap::new(),
            session: None,
            sessions: Vec::new(),
        }
    }
}

fn worktree(repo: &Repo) -> anyhow::Result<Diff> {
    git::diff_worktree_vs_head(repo).context("building the working-tree diff")
}

/// Diff the selected run from its pre-run backups when usable, else fall back to
/// working-tree-vs-HEAD (the run had no resolvable snapshot).
fn build_diff_for(repo: &Repo, ctx: &SessionContext) -> anyhow::Result<Diff> {
    match ctx.diff_base() {
        Some(DiffBase::AgentRun { .. }) => {
            let run = ctx.selected().expect("diff_base implies a selected run");
            git::differ::diff_agent_run(repo, &ctx.session.id, run)
                .context("building the agent-run diff")
        }
        _ => worktree(repo),
    }
}

fn summarize(ctx: &SessionContext, base: &DiffBase) -> SessionSummary {
    let base_label = match base {
        DiffBase::AgentRun { run, .. } => {
            let total = ctx.session.runs.len();
            let mode = ctx.run(*run).map(|r| mode_label(r.mode)).unwrap_or("");
            format!("agent run {}/{} ({mode})", run.0 + 1, total)
        }
        DiffBase::WorkingTreeVsHead => "working tree vs HEAD".to_string(),
        DiffBase::WorkingTreeVsIndex => "working tree vs index".to_string(),
        DiffBase::Range { from, to } => format!("{from}..{to}"),
    };
    SessionSummary {
        provider: ctx.session.provider,
        id: ctx.session.id.0.clone(),
        title: ctx.session.title.clone(),
        last_prompt: ctx.session.last_prompt.clone(),
        base_label,
        // A run that never closed (no following non-autonomous turn) reads as
        // still running — but quitting the agent mid-acceptEdits leaves the
        // span open forever, so "live" additionally requires the transcript to
        // have been written to recently.
        live: ctx.selected().is_some_and(|r| r.ended.is_none())
            && recently_modified(&ctx.session.file),
        // The commands the selected run ran, for the verification badge/overlay.
        commands: ctx
            .selected()
            .map(|r| r.commands.clone())
            .unwrap_or_default(),
        verify_stale: ctx.selected().is_some_and(verification_stale),
    }
}

/// Whether the run's verification evidence predates its final edits: a ✓ badge
/// from a `cargo test` that ran *before* the last three edits proves nothing
/// about the state under review. Timestamps must exist on both sides to claim
/// staleness — missing data never cries wolf.
fn verification_stale(run: &crate::domain::session::AgentRun) -> bool {
    use crate::domain::session::CommandKind;
    let last_edit = run.edits.iter().filter_map(|e| Some(e.timestamp?.0)).max();
    let last_verify = run
        .commands
        .iter()
        .filter(|c| {
            matches!(
                c.kind,
                CommandKind::Test | CommandKind::Build | CommandKind::Lint | CommandKind::Format
            )
        })
        .filter_map(|c| Some(c.timestamp?.0))
        .max();
    matches!((last_edit, last_verify), (Some(edit), Some(verify)) if verify < edit)
}

/// How stale a transcript may be while its open run still counts as live.
const LIVE_STALENESS: std::time::Duration = std::time::Duration::from_secs(300);

fn recently_modified(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .map(|t| t.elapsed().map_or(true, |age| age < LIVE_STALENESS))
        .unwrap_or(false)
}

fn mode_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Auto => "auto",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Plan => "plan",
        PermissionMode::Default => "default",
        PermissionMode::Autopilot => "autopilot",
        PermissionMode::Interactive => "interactive",
    }
}

/// The active provider's sessions for `cwd`, for the picker.
fn session_items(
    provider: Provider,
    dirs: &AgentDirs,
    cwd: &Path,
    current_id: Option<&str>,
) -> Vec<SessionListItem> {
    match provider {
        Provider::Claude => {
            let Some(dir) = dirs.claude.as_deref() else {
                return Vec::new();
            };
            locate::list_sessions(dir, cwd)
                .into_iter()
                .map(|entry| {
                    let meta = locate::peek_meta(&entry.path);
                    SessionListItem {
                        is_current: current_id == Some(entry.id.0.as_str()),
                        id: entry.id.0,
                        title: meta.title,
                        last_prompt: meta.last_prompt,
                    }
                })
                .collect()
        }
        Provider::Copilot => {
            let Some(dir) = dirs.copilot.as_deref() else {
                return Vec::new();
            };
            copilot::list_sessions(dir, cwd)
                .into_iter()
                .map(|entry| SessionListItem {
                    is_current: current_id == Some(entry.id.0.as_str()),
                    title: copilot::peek_title(&entry.events),
                    id: entry.id.0,
                    last_prompt: None,
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Timestamp;
    use crate::domain::session::{
        AgentRun, CommandKind, CommandOutcome, CommandRun, EditTool, RunId, ToolEditEvent,
    };
    use std::collections::HashMap;

    fn run_with(edit_ts: Option<i64>, verify_ts: Option<i64>) -> AgentRun {
        AgentRun {
            id: RunId(0),
            mode: PermissionMode::AcceptEdits,
            started: Timestamp(0),
            ended: None,
            snapshot: HashMap::new(),
            base_commit: None,
            edits: vec![ToolEditEvent {
                file_path: "a.rs".into(),
                tool: EditTool::Edit,
                message_uuid: "m".into(),
                parent_uuid: None,
                timestamp: edit_ts.map(Timestamp),
            }],
            commands: vec![CommandRun {
                command: "cargo test".into(),
                description: None,
                kind: CommandKind::Test,
                outcome: CommandOutcome::Ok,
                output_excerpt: String::new(),
                message_uuid: "c".into(),
                timestamp: verify_ts.map(Timestamp),
            }],
        }
    }

    #[test]
    fn verification_is_stale_only_when_edits_postdate_the_last_check() {
        // Test ran after the last edit → fresh.
        assert!(!verification_stale(&run_with(Some(10), Some(20))));
        // Edits landed after the last test → stale.
        assert!(verification_stale(&run_with(Some(30), Some(20))));
        // Missing timestamps on either side must not cry wolf.
        assert!(!verification_stale(&run_with(None, Some(20))));
        assert!(!verification_stale(&run_with(Some(30), None)));
    }
}
