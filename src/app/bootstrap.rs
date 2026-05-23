//! Composition root for a review session: open the repo, choose a diff base
//! (range / staged / latest agent run / working-tree fallback), build the diff,
//! recover intent, and load persisted verdicts into a ready-to-render
//! [`AppState`]. [`build_bundle`] is reused by the live re-diff worker.

use std::path::Path;

use anyhow::Context;

use crate::config;
use crate::domain::diff::{Diff, DiffBase};
use crate::domain::session::PermissionMode;
use crate::git::{self, Repo};
use crate::session::intent::IntentMap;
use crate::session::{self, SessionContext, locate};

use super::state::{AppState, SessionListItem, SessionSummary};

/// What the user asked for, owned so it can be cloned to the worker thread.
#[derive(Debug, Clone, Default)]
pub struct Selectors {
    pub no_session: bool,
    pub session_id: Option<String>,
    pub run_index: Option<u32>,
    pub range: Option<(String, String)>,
    pub staged: bool,
}

/// The diff plus its session overlay — everything a re-diff replaces, leaving
/// review verdicts/notes (keyed by fingerprint) and cursor state untouched.
pub struct DiffBundle {
    pub diff: Diff,
    pub intent: IntentMap,
    pub session: Option<SessionSummary>,
    pub sessions: Vec<SessionListItem>,
}

/// Build the diff bundle for the current selectors. No persisted-state or
/// terminal I/O, so it is safe to run on the worker thread.
pub fn build_bundle(
    repo: &Repo,
    claude_dir: Option<&Path>,
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

    let Some(claude) = claude_dir else {
        return Ok(DiffBundle::git_only(worktree(repo)?));
    };

    match session::load_session(
        claude,
        repo.workdir(),
        selectors.session_id.as_deref(),
        selectors.run_index,
    ) {
        Some(ctx) => {
            let diff = build_diff_for(repo, &ctx)?;
            let session = Some(summarize(&ctx, &diff.base));
            let sessions = session_items(claude, repo.workdir(), Some(&ctx.session.id.0));
            Ok(DiffBundle {
                diff,
                intent: ctx.intent,
                session,
                sessions,
            })
        }
        None => Ok(DiffBundle {
            diff: worktree(repo)?,
            intent: IntentMap::new(),
            session: None,
            sessions: session_items(claude, repo.workdir(), None),
        }),
    }
}

/// Build the full initial [`AppState`]: a bundle plus persisted review verdicts.
pub fn build_state(
    repo: &Repo,
    state_dir: &Path,
    claude_dir: Option<&Path>,
    selectors: &Selectors,
) -> anyhow::Result<AppState> {
    let bundle = build_bundle(repo, claude_dir, selectors)?;
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
    state.session = bundle.session;
    state.sessions = bundle.sessions;
    Ok(state)
}

impl DiffBundle {
    fn git_only(diff: Diff) -> Self {
        DiffBundle {
            diff,
            intent: IntentMap::new(),
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
        id: ctx.session.id.0.clone(),
        title: ctx.session.title.clone(),
        last_prompt: ctx.session.last_prompt.clone(),
        base_label,
        // A run that never closed (no following non-autonomous turn) is still running.
        live: ctx.selected().is_some_and(|r| r.ended.is_none()),
        // The commands the selected run ran, for the verification badge/overlay.
        commands: ctx
            .selected()
            .map(|r| r.commands.clone())
            .unwrap_or_default(),
    }
}

fn mode_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Auto => "auto",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Plan => "plan",
        PermissionMode::Default => "default",
    }
}

fn session_items(claude: &Path, cwd: &Path, current_id: Option<&str>) -> Vec<SessionListItem> {
    locate::list_sessions(claude, cwd)
        .into_iter()
        .map(|entry| {
            let label = locate::peek_label(&entry.path);
            SessionListItem {
                is_current: current_id == Some(entry.id.0.as_str()),
                id: entry.id.0,
                title: label.title,
                last_prompt: label.last_prompt,
            }
        })
        .collect()
}
