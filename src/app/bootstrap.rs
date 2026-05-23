//! Composition root for a review session: open the repo, choose a diff base
//! (latest agent run vs. working-tree fallback), build the diff, recover intent,
//! and load persisted verdicts into a ready-to-render [`AppState`].

use std::path::Path;

use anyhow::Context;

use crate::config;
use crate::domain::diff::{Diff, DiffBase};
use crate::domain::session::PermissionMode;
use crate::git::{self, Repo};
use crate::session::{self, SessionContext, locate};

use super::state::{AppState, SessionListItem, SessionSummary};

/// What the user asked for on the command line, minus the path.
pub struct Selectors<'a> {
    pub no_session: bool,
    pub session_id: Option<&'a str>,
    pub run_index: Option<u32>,
}

/// Build the initial application state for `repo`.
pub fn build_state(
    repo: &Repo,
    state_dir: &Path,
    claude_dir: Option<&Path>,
    selectors: &Selectors<'_>,
) -> anyhow::Result<AppState> {
    let mut intent = session::intent::IntentMap::new();
    let mut summary = None;
    let mut sessions = Vec::new();

    let diff = if selectors.no_session {
        git::diff_worktree_vs_head(repo).context("building the working-tree diff")?
    } else if let Some(claude) = claude_dir {
        match session::load_session(
            claude,
            repo.workdir(),
            selectors.session_id,
            selectors.run_index,
        ) {
            Some(ctx) => {
                let diff = build_diff_for(repo, &ctx)?;
                tracing::info!(
                    session = %ctx.session.id.0,
                    runs = ctx.session.runs.len(),
                    selected_run = ?ctx.selected_run,
                    intent_files = ctx.intent.len(),
                    base = ?diff.base,
                    "loaded Claude Code session"
                );
                summary = Some(summarize(&ctx, &diff.base));
                sessions = session_items(claude, repo.workdir(), Some(&ctx.session.id.0));
                intent = ctx.intent;
                diff
            }
            None => {
                sessions = session_items(claude, repo.workdir(), None);
                git::diff_worktree_vs_head(repo).context("building the working-tree diff")?
            }
        }
    } else {
        git::diff_worktree_vs_head(repo).context("building the working-tree diff")?
    };

    let state_path = config::review_state_path(state_dir, repo.workdir(), &diff.base);
    let review = config::load_review_state(&state_path);

    let mut state = AppState::new(diff, review, state_path);
    state.intent = intent;
    state.session = summary;
    state.sessions = sessions;
    Ok(state)
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
        _ => git::diff_worktree_vs_head(repo).context("building the working-tree diff"),
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
        title: ctx.session.title.clone(),
        last_prompt: ctx.session.last_prompt.clone(),
        base_label,
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
