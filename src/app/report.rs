//! Markdown export of the review (`--report`): summary counts, verification
//! results, and every flagged or noted hunk with its diff text and correlated
//! intent. Read-only and terminal-free — the output is meant to be piped back
//! to the agent ("here's what I flagged, fix it") or kept as a review record.

use std::fmt::Write as _;

use serde::Serialize;

use crate::domain::diff::{DiffBase, FileChange, Hunk, LineKind};
use crate::domain::review::HunkVerdict;
use crate::domain::session::{CommandOutcome, CommandRun, Provider};

use super::state::AppState;

/// A hunk belongs in the report body when the reviewer flagged it or wrote a
/// note on it — the actionable subset.
fn reportable_hunks<'a>(state: &AppState, file: &'a FileChange) -> Vec<&'a Hunk> {
    file.hunks
        .iter()
        .filter(|h| {
            state.review.verdict(&h.href) == HunkVerdict::NeedsAttention
                || state.review.notes.contains_key(&h.href)
        })
        .collect()
}

fn verdict_label(verdict: HunkVerdict) -> &'static str {
    match verdict {
        HunkVerdict::NeedsAttention => "flagged",
        HunkVerdict::Approved => "approved",
        HunkVerdict::Unreviewed => "unreviewed",
    }
}

/// The hunk's resolved intent: the matched edit's, else the file fallback.
fn resolved_intent(state: &AppState, file: &FileChange, hunk: &Hunk) -> Option<(String, &'static str)> {
    if let Some(intent) = state.hunk_intent.get(&hunk.href) {
        return Some((intent.text.clone(), "hunk"));
    }
    state
        .intent
        .get(&file.path)
        .map(|intent| (intent.text.clone(), "file"))
}

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
    unreviewed_checklist(&mut out, state);
    out
}

/// Structured twin of the markdown report, for piping into tools/agents.
pub fn render_json(state: &AppState) -> String {
    let counts = state.counts();
    let noted = state
        .diff
        .files
        .iter()
        .flat_map(|f| &f.hunks)
        .filter(|h| state.review.notes.contains_key(&h.href))
        .count();

    let findings = state
        .diff
        .files
        .iter()
        .flat_map(|file| {
            reportable_hunks(state, file).into_iter().map(move |hunk| {
                let intent = resolved_intent(state, file, hunk);
                JsonFinding {
                    path: file.path.display().to_string(),
                    header: hunk.header.clone(),
                    fingerprint: hunk.href.fingerprint,
                    verdict: verdict_label(state.review.verdict(&hunk.href)).to_string(),
                    note: state.review.notes.get(&hunk.href).cloned(),
                    intent_scope: intent.as_ref().map(|(_, scope)| (*scope).to_string()),
                    intent: intent.map(|(text, _)| text),
                    diff: hunk_text(hunk),
                }
            })
        })
        .collect();

    let doc = JsonReport {
        base: state
            .session
            .as_ref()
            .map(|s| s.base_label.clone())
            .unwrap_or_else(|| base_label(&state.diff.base)),
        session: state.session.as_ref().map(|s| JsonSession {
            id: s.id.clone(),
            provider: match s.provider {
                Provider::Claude => "claude".to_string(),
                Provider::Copilot => "copilot".to_string(),
            },
            title: s.title.clone(),
            live: s.live,
        }),
        summary: JsonSummary {
            total: counts.total,
            approved: counts.reviewed - counts.needs_attention,
            flagged: counts.needs_attention,
            unreviewed: counts.total - counts.reviewed,
            notes: noted,
            changed_since_reviewed: counts.changed_since_reviewed,
        },
        verification: state
            .session
            .iter()
            .flat_map(|s| &s.commands)
            .filter(|c| c.kind.is_verification())
            .map(|c| JsonCommand {
                kind: c.kind.label().to_string(),
                command: c.command.clone(),
                outcome: match c.outcome {
                    CommandOutcome::Ok => "ok".to_string(),
                    CommandOutcome::Failed => "failed".to_string(),
                    CommandOutcome::Unknown => "unknown".to_string(),
                },
                output_excerpt: c.output_excerpt.clone(),
            })
            .collect(),
        findings,
        unreviewed: unreviewed_refs(state)
            .into_iter()
            .map(|(file, hunk)| JsonHunkId {
                path: file.path.display().to_string(),
                header: hunk.header.clone(),
                fingerprint: hunk.href.fingerprint,
            })
            .collect(),
    };
    let mut out = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_string());
    out.push('\n');
    out
}

#[derive(Serialize)]
struct JsonReport {
    base: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session: Option<JsonSession>,
    summary: JsonSummary,
    verification: Vec<JsonCommand>,
    findings: Vec<JsonFinding>,
    unreviewed: Vec<JsonHunkId>,
}

#[derive(Serialize)]
struct JsonSession {
    id: String,
    provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    live: bool,
}

#[derive(Serialize)]
struct JsonSummary {
    total: usize,
    approved: usize,
    flagged: usize,
    unreviewed: usize,
    notes: usize,
    changed_since_reviewed: usize,
}

#[derive(Serialize)]
struct JsonCommand {
    kind: String,
    command: String,
    outcome: String,
    output_excerpt: String,
}

#[derive(Serialize)]
struct JsonFinding {
    path: String,
    header: String,
    fingerprint: u64,
    verdict: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    intent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    intent_scope: Option<String>,
    diff: String,
}

#[derive(Serialize)]
struct JsonHunkId {
    path: String,
    header: String,
    fingerprint: u64,
}

/// Every hunk still without a verdict, in diff order.
fn unreviewed_refs(state: &AppState) -> Vec<(&FileChange, &Hunk)> {
    state
        .diff
        .files
        .iter()
        .flat_map(|file| file.hunks.iter().map(move |hunk| (file, hunk)))
        .filter(|(_, hunk)| state.review.verdict(&hunk.href) == HunkVerdict::Unreviewed)
        .collect()
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
        let reportable = reportable_hunks(state, file);
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
            let verdict = verdict_label(state.review.verdict(&hunk.href));
            let _ = writeln!(out, "#### `{}` — {verdict}\n", hunk.header);
            // The specific edit's reasoning, when it differs from the
            // file-level intent already quoted above.
            if let Some(hunk_intent) = state.hunk_intent.get(&hunk.href)
                && state
                    .intent
                    .get(&file.path)
                    .is_none_or(|fi| fi.text != hunk_intent.text)
            {
                for line in hunk_intent.text.lines() {
                    let _ = writeln!(out, "> {line}");
                }
                out.push('\n');
            }
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

/// What's left: a checkbox per verdict-less hunk, so the report doubles as a
/// "where I left off" record.
fn unreviewed_checklist(out: &mut String, state: &AppState) {
    let remaining = unreviewed_refs(state);
    if remaining.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n## Unreviewed hunks\n");
    for (file, hunk) in remaining {
        let _ = writeln!(out, "- [ ] `{}` — `{}`", file.path.display(), hunk.header);
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
                base_fallback: false,
                language: Some("rust".into()),
                hunks: vec![
                    hunk(1, "@@ -1,2 +1,2 @@ fn main()"),
                    hunk(2, "@@ -9,2 +9,2 @@"),
                    hunk(3, "@@ -20,2 +20,2 @@"),
                ],
                stats: (3, 3),
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
                file_path: path.clone(),
                text: "Bump the constant so the example reflects the new default.".into(),
                source_uuid: "a1".into(),
                confidence: 0.9,
            },
        );
        // The flagged hunk was matched to a specific edit with its own "why".
        let flagged_href = state.diff.files[0].hunks[0].href.clone();
        state.hunk_intent.insert(
            flagged_href,
            Intent {
                file_path: path,
                text: "Bump x specifically for the doctest.".into(),
                source_uuid: "a2".into(),
                confidence: 1.0,
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
    fn json_report_snapshot() {
        insta::assert_snapshot!(render_json(&sample_state()));
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
        // Nothing to review → no dangling checklist heading.
        assert!(!report.contains("Unreviewed hunks"));

        let json = render_json(&state);
        assert!(json.contains("\"findings\": []"));
        assert!(json.contains("\"unreviewed\": []"));
    }
}
