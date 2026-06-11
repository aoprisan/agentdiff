//! Markdown export of the review (`--report`): summary counts, verification
//! results, and every flagged or noted hunk with its diff text and correlated
//! intent. Read-only and terminal-free — the output is meant to be piped back
//! to the agent ("here's what I flagged, fix it") or kept as a review record.

use std::fmt::Write as _;

use crate::domain::diff::{DiffBase, FileChange, Hunk, LineKind};
use crate::domain::review::HunkVerdict;
use crate::domain::session::{CommandOutcome, CommandRun};

use super::state::AppState;

pub fn render_markdown(state: &AppState) -> String {
    let mut out = String::new();

    let label = state
        .session
        .as_ref()
        .map(|s| s.base_label.clone())
        .unwrap_or_else(|| base_label(&state.diff.base));
    let _ = writeln!(out, "# agentdiff review — {label}\n");

    if let Some(session) = &state.session {
        let title = session.title.as_deref().unwrap_or("untitled session");
        let _ = writeln!(
            out,
            "Session: {title} (`{}`, {})\n",
            session.id,
            session.provider.label()
        );
    }

    summary(&mut out, state);
    verification(&mut out, state);
    hunks(&mut out, state);
    out
}

fn summary(out: &mut String, state: &AppState) {
    let counts = state.counts();
    let approved = counts.reviewed - counts.needs_attention;
    let unreviewed = counts.total - counts.reviewed;
    let noted = state
        .diff
        .files
        .iter()
        .flat_map(|f| &f.hunks)
        .filter(|h| state.review.notes.contains_key(&h.href))
        .count();

    let mut parts = vec![format!("{approved} approved")];
    parts.push(format!("{} flagged", counts.needs_attention));
    parts.push(format!("{unreviewed} unreviewed"));
    if noted > 0 {
        parts.push(format!("{noted} with notes"));
    }
    let _ = writeln!(out, "**{} hunks** — {}", counts.total, parts.join(" · "));
    if counts.changed_since_reviewed > 0 {
        let _ = writeln!(
            out,
            "\n⚠ {} reviewed hunk(s) changed after the verdict was recorded.",
            counts.changed_since_reviewed
        );
    }
    out.push('\n');
}

/// The agent's own verification work (tests/build/lint), with failure output.
fn verification(out: &mut String, state: &AppState) {
    let Some(session) = &state.session else {
        return;
    };
    let checks: Vec<&CommandRun> = session
        .commands
        .iter()
        .filter(|c| c.kind.is_verification())
        .collect();
    if checks.is_empty() {
        return;
    }

    let _ = writeln!(out, "## Verification (commands the agent ran)\n");
    for cmd in &checks {
        let mark = match cmd.outcome {
            CommandOutcome::Ok => "✓",
            CommandOutcome::Failed => "✗",
            CommandOutcome::Unknown => "·",
        };
        let _ = writeln!(out, "- {mark} {} — `{}`", cmd.kind.label(), cmd.command);
    }
    out.push('\n');
    for cmd in checks {
        if cmd.outcome == CommandOutcome::Failed && !cmd.output_excerpt.is_empty() {
            let _ = writeln!(out, "Failed {} output (`{}`):\n", cmd.kind.label(), cmd.command);
            fenced(out, "text", &cmd.output_excerpt);
        }
    }
}

/// Every flagged or noted hunk, grouped per file under the file's intent.
fn hunks(out: &mut String, state: &AppState) {
    let mut any = false;
    for file in &state.diff.files {
        let reportable: Vec<&Hunk> = file
            .hunks
            .iter()
            .filter(|h| {
                state.review.verdict(&h.href) == HunkVerdict::NeedsAttention
                    || state.review.notes.contains_key(&h.href)
            })
            .collect();
        if reportable.is_empty() {
            continue;
        }
        if !any {
            let _ = writeln!(out, "## Flagged & noted hunks\n");
            any = true;
        }

        let _ = writeln!(out, "### {}\n", file_label(file));
        if let Some(intent) = state.intent.get(&file.path) {
            // Blockquote every line so multi-line intent stays inside the quote.
            for line in intent.text.lines() {
                let _ = writeln!(out, "> {line}");
            }
            out.push('\n');
        }

        for hunk in reportable {
            let verdict = match state.review.verdict(&hunk.href) {
                HunkVerdict::NeedsAttention => "flagged",
                HunkVerdict::Approved => "approved",
                HunkVerdict::Unreviewed => "unreviewed",
            };
            let _ = writeln!(out, "#### `{}` — {verdict}\n", hunk.header);
            if let Some(note) = state.review.notes.get(&hunk.href) {
                let _ = writeln!(out, "Note: {note}\n");
            }
            fenced(out, "diff", &hunk_text(hunk));
        }
    }
    if !any {
        let _ = writeln!(out, "No flagged hunks or notes.");
    }
}

fn hunk_text(hunk: &Hunk) -> String {
    let mut text = String::new();
    for line in &hunk.lines {
        let prefix = match line.kind {
            LineKind::Context => ' ',
            LineKind::Added => '+',
            LineKind::Removed => '-',
        };
        let _ = writeln!(text, "{prefix}{}", line.text);
    }
    text
}

/// Fence with four backticks so diff text containing ``` can't break out.
fn fenced(out: &mut String, lang: &str, body: &str) {
    let _ = writeln!(out, "````{lang}");
    out.push_str(body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    let _ = writeln!(out, "````\n");
}

fn file_label(file: &FileChange) -> String {
    match &file.old_path {
        Some(old) => format!("{} → {}", old.display(), file.path.display()),
        None => file.path.display().to_string(),
    }
}

fn base_label(base: &DiffBase) -> String {
    match base {
        DiffBase::WorkingTreeVsHead => "working tree vs HEAD".to_string(),
        DiffBase::WorkingTreeVsIndex => "staged changes".to_string(),
        DiffBase::Range { from, to } => format!("{from}..{to}"),
        DiffBase::AgentRun { run, .. } => format!("agent run {}", run.0 + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::SessionSummary;
    use crate::domain::Timestamp;
    use crate::domain::diff::{ChangeKind, Diff, FileId, Line, LineRange};
    use crate::domain::review::{HunkRef, ReviewState};
    use crate::domain::session::{CommandKind, Intent, Provider};
    use std::path::PathBuf;

    fn line(kind: LineKind, text: &str) -> Line {
        Line {
            kind,
            old_no: None,
            new_no: None,
            text: text.into(),
            intra: Vec::new(),
        }
    }

    fn sample_state() -> AppState {
        let path = PathBuf::from("src/main.rs");
        let hunk = |fp: u64, header: &str| Hunk {
            href: HunkRef {
                path: path.clone(),
                fingerprint: fp,
            },
            old: LineRange { start: 1, count: 2 },
            new: LineRange { start: 1, count: 2 },
            header: header.into(),
            lines: vec![
                line(LineKind::Context, "fn main() {"),
                line(LineKind::Removed, "    let x = 1;"),
                line(LineKind::Added, "    let x = 2;"),
            ],
        };
        let diff = Diff {
            base: DiffBase::WorkingTreeVsHead,
            generated_at: Timestamp::from_millis(0),
            files: vec![FileChange {
                id: FileId(0),
                path: path.clone(),
                old_path: None,
                change: ChangeKind::Modified,
                is_binary: false,
                is_created: false,
                language: Some("rust".into()),
                hunks: vec![hunk(1, "@@ -1,2 +1,2 @@ fn main()"), hunk(2, "@@ -9,2 +9,2 @@")],
                stats: (2, 2),
            }],
        };

        let mut state = AppState::new(diff, ReviewState::default(), PathBuf::from("/tmp/r.toml"));
        let flagged = state.diff.files[0].hunks[0].href.clone();
        let approved = state.diff.files[0].hunks[1].href.clone();
        state
            .review
            .set_verdict(flagged.clone(), HunkVerdict::NeedsAttention);
        state.review.set_verdict(approved, HunkVerdict::Approved);
        state
            .review
            .notes
            .insert(flagged, "off-by-one suspicion".into());
        state.intent.insert(
            path.clone(),
            Intent {
                file_path: path,
                text: "Bump the constant so the example reflects the new default.".into(),
                source_uuid: "a1".into(),
                confidence: 0.9,
            },
        );
        state.session = Some(SessionSummary {
            provider: Provider::Claude,
            id: "11111111-aaaa".into(),
            title: Some("Add greeting, fix off-by-one".into()),
            last_prompt: None,
            base_label: "agent run 1/1 (auto)".into(),
            live: false,
            commands: vec![
                CommandRun {
                    command: "cargo test --all".into(),
                    description: None,
                    kind: CommandKind::Test,
                    outcome: CommandOutcome::Ok,
                    output_excerpt: "test result: ok.".into(),
                    message_uuid: "c1".into(),
                    timestamp: None,
                },
                CommandRun {
                    command: "cargo clippy --all-targets".into(),
                    description: None,
                    kind: CommandKind::Lint,
                    outcome: CommandOutcome::Failed,
                    output_excerpt: "error: unused variable `x`".into(),
                    message_uuid: "c2".into(),
                    timestamp: None,
                },
            ],
        });
        state
    }

    #[test]
    fn report_snapshot() {
        insta::assert_snapshot!(render_markdown(&sample_state()));
    }

    #[test]
    fn report_without_session_or_verdicts() {
        let state = AppState::new(
            Diff {
                base: DiffBase::WorkingTreeVsHead,
                generated_at: Timestamp::from_millis(0),
                files: Vec::new(),
            },
            ReviewState::default(),
            PathBuf::from("/tmp/r.toml"),
        );
        let report = render_markdown(&state);
        assert!(report.contains("working tree vs HEAD"));
        assert!(report.contains("No flagged hunks or notes."));
    }
}
