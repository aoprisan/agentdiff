//! Assembling a Copilot session's events into a reviewable **run**.
//!
//! Unlike Claude — where pre-run backups are interleaved into the transcript as
//! `file-history-snapshot` records and we segment by permission mode — Copilot
//! stores backups separately (session-global `rewind-snapshots`, see
//! [`super::snapshots`]). So a Copilot session maps to a **single run**
//! aggregating the whole session's edits and commands; its pre-run snapshot is
//! folded from the rewind index by the loader. `mode` is metadata for the label:
//! `Autopilot` if the agent ran autonomously at any point, else `Interactive`.

use std::collections::HashMap;

use crate::domain::Timestamp;
use crate::domain::session::{CommandOutcome, CommandRun, PermissionMode, ToolEditEvent};

use super::super::commands;
use super::super::runs::parse_timestamp;
use super::events::{RawEvent, edit_path, edit_tool};

/// The single aggregated run before its backups are attached.
#[derive(Debug, Clone)]
pub struct RawRun {
    pub mode: PermissionMode,
    pub started: Option<Timestamp>,
    pub ended: Option<Timestamp>,
    pub edits: Vec<ToolEditEvent>,
    pub commands: Vec<CommandRun>,
    /// `toolCallId` → index in `commands`, awaiting its completion event.
    pending: HashMap<String, usize>,
}

impl RawRun {
    fn new() -> Self {
        Self {
            mode: PermissionMode::Interactive,
            started: None,
            ended: None,
            edits: Vec::new(),
            commands: Vec::new(),
            pending: HashMap::new(),
        }
    }

    fn observe(&mut self, ts: Timestamp) {
        if self.started.is_none_or(|s| ts.0 < s.0) {
            self.started = Some(ts);
        }
        if self.ended.is_none_or(|e| ts.0 > e.0) {
            self.ended = Some(ts);
        }
    }

    fn push_command(&mut self, cmd: CommandRun, id: Option<&str>) {
        let idx = self.commands.len();
        self.commands.push(cmd);
        if let Some(id) = id {
            self.pending.insert(id.to_string(), idx);
        }
    }

    fn resolve_result(&mut self, id: Option<&str>, success: bool, output: &str) {
        let Some(idx) = id.and_then(|id| self.pending.remove(id)) else {
            return;
        };
        if let Some(cmd) = self.commands.get_mut(idx) {
            // Copilot reports a reliable boolean; still let the content heuristics
            // flip a "success" with a failure signal, as the Claude path does.
            cmd.outcome = commands::outcome(Some(!success), output);
            cmd.output_excerpt = commands::excerpt(output);
        }
    }
}

/// Output of segmentation: the run (if the session did anything) plus metadata.
#[derive(Debug, Clone, Default)]
pub struct Segmentation {
    pub runs: Vec<RawRun>,
    pub last_prompt: Option<String>,
    /// The first user prompt, used as the session title.
    pub first_prompt: Option<String>,
}

/// Aggregate all events into one run (plus session metadata).
pub fn segment(events: &[RawEvent]) -> Segmentation {
    let mut out = Segmentation::default();
    if events.is_empty() {
        return out;
    }

    let mut run = RawRun::new();
    let mut saw_autopilot = false;

    for event in events {
        if let Some(ts) = event.timestamp.as_deref().and_then(parse_timestamp) {
            run.observe(ts);
        }

        // Any autopilot mode (entered or left) marks the run autonomous.
        if event.new_mode() == Some("autopilot") || event.previous_mode() == Some("autopilot") {
            saw_autopilot = true;
        }

        if let Some(text) = event.user_text() {
            if out.first_prompt.is_none() {
                out.first_prompt = Some(text.clone());
            }
            out.last_prompt = Some(text);
        }

        if let Some((name, args)) = event.tool_start() {
            if let Some(tool) = edit_tool(name)
                && let Some(path) = edit_path(args)
            {
                run.edits.push(ToolEditEvent {
                    file_path: path.into(),
                    tool,
                    message_uuid: event.id.clone().unwrap_or_default(),
                    parent_uuid: event.parent_id.clone(),
                });
            }
            if name == "bash"
                && let Some(command) = args.get("command").and_then(|v| v.as_str())
            {
                let ts = event.timestamp.as_deref().and_then(parse_timestamp);
                run.push_command(
                    CommandRun {
                        command: command.to_string(),
                        description: args
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        kind: commands::classify(command),
                        outcome: CommandOutcome::Unknown,
                        output_excerpt: String::new(),
                        message_uuid: event.id.clone().unwrap_or_default(),
                        timestamp: ts,
                    },
                    event.tool_call_id(),
                );
            }
        }

        if let Some((success, output)) = event.tool_complete() {
            run.resolve_result(event.tool_call_id(), success, &output);
        }
    }

    run.mode = if saw_autopilot {
        PermissionMode::Autopilot
    } else {
        PermissionMode::Interactive
    };
    // A session closed by a shutdown/task-complete event has ended; otherwise the
    // CLI may still be running it, so leave it live.
    let last = events.last().map(|e| e.kind.as_str());
    if matches!(last, Some("session.shutdown") | Some("session.task_complete")) {
        // keep the observed end time
    } else {
        run.ended = None;
    }

    out.runs.push(run);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::events::parse_reader;
    use crate::domain::session::{CommandKind, EditTool};

    #[test]
    fn aggregates_edits_commands_and_marks_autopilot() {
        let jsonl = concat!(
            r#"{"type":"user.message","id":"u1","timestamp":"2026-05-20T10:00:00Z","data":{"content":"build it"}}"#,
            "\n",
            r#"{"type":"session.mode_changed","id":"m1","data":{"previousMode":"interactive","newMode":"autopilot"}}"#,
            "\n",
            r#"{"type":"tool.execution_start","id":"a1","data":{"toolCallId":"t1","toolName":"edit","turnId":"1","arguments":{"path":"/repo/a.rs"}}}"#,
            "\n",
            r#"{"type":"tool.execution_start","id":"a2","data":{"toolCallId":"t2","toolName":"bash","arguments":{"command":"cargo test","description":"tests"}}}"#,
            "\n",
            r#"{"type":"tool.execution_complete","id":"a3","data":{"toolCallId":"t2","success":true,"result":{"content":"test result: ok. 3 passed"}}}"#,
            "\n",
            r#"{"type":"tool.execution_start","id":"a4","data":{"toolCallId":"t3","toolName":"bash","arguments":{"command":"cargo clippy"}}}"#,
            "\n",
            r#"{"type":"tool.execution_complete","id":"a5","timestamp":"2026-05-20T10:01:00Z","data":{"toolCallId":"t3","success":false,"result":{"content":"error: unused\nExit code 1"}}}"#,
        );
        let seg = segment(&parse_reader(jsonl.as_bytes()));
        assert_eq!(seg.runs.len(), 1);
        let run = &seg.runs[0];

        assert_eq!(run.mode, PermissionMode::Autopilot);
        assert_eq!(run.edits.len(), 1);
        assert_eq!(run.edits[0].tool, EditTool::Edit);

        assert_eq!(run.commands.len(), 2);
        assert_eq!(run.commands[0].kind, CommandKind::Test);
        assert_eq!(run.commands[0].outcome, CommandOutcome::Ok);
        assert_eq!(run.commands[1].kind, CommandKind::Lint);
        assert_eq!(run.commands[1].outcome, CommandOutcome::Failed);

        assert_eq!(seg.first_prompt.as_deref(), Some("build it"));
        // No shutdown event ⇒ the run is live.
        assert!(run.ended.is_none());
    }

    #[test]
    fn interactive_only_session_is_not_autopilot_and_ends_on_shutdown() {
        let jsonl = concat!(
            r#"{"type":"user.message","id":"u1","timestamp":"2026-05-20T10:00:00Z","data":{"content":"hi"}}"#,
            "\n",
            r#"{"type":"session.shutdown","id":"s1","timestamp":"2026-05-20T10:00:05Z","data":{}}"#,
        );
        let seg = segment(&parse_reader(jsonl.as_bytes()));
        let run = &seg.runs[0];
        assert_eq!(run.mode, PermissionMode::Interactive);
        assert!(run.ended.is_some(), "shutdown closes the run");
    }
}
