//! Segmenting a transcript into autonomous **runs**.
//!
//! A run is a maximal contiguous span during which the agent was in an
//! autonomous permission mode (`auto`/`acceptEdits`). Within a span we collect
//! the agent's edit events and fold in the (cumulative) `file-history-snapshot`
//! records, keeping the latest within the span as the pre-run file map. The raw
//! per-path backups are resolved to on-disk paths later by `backups`.

use std::collections::HashMap;

use crate::domain::Timestamp;
use crate::domain::session::{CommandOutcome, CommandRun, PermissionMode, ToolEditEvent};

use super::commands;
use super::transcript::{Record, TrackedBackup, edit_tool};

/// A run before its backups are resolved to filesystem paths.
#[derive(Debug, Clone)]
pub struct RawRun {
    pub mode: PermissionMode,
    pub started: Option<Timestamp>,
    pub ended: Option<Timestamp>,
    pub edits: Vec<ToolEditEvent>,
    /// Latest `trackedFileBackups` seen within the span (path string → backup).
    pub raw_backups: HashMap<String, TrackedBackup>,
    /// `Bash` commands run within the span, in transcript order.
    pub commands: Vec<CommandRun>,
    /// `tool_use` id → index in `commands`, awaiting its `tool_result`.
    pending: HashMap<String, usize>,
}

impl RawRun {
    fn new(mode: PermissionMode) -> Self {
        Self {
            mode,
            started: None,
            ended: None,
            edits: Vec::new(),
            raw_backups: HashMap::new(),
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

    /// Record a `Bash` tool call, leaving its outcome `Unknown` until the
    /// matching `tool_result` is seen.
    fn push_command(&mut self, cmd: CommandRun, id: Option<&str>) {
        let idx = self.commands.len();
        self.commands.push(cmd);
        if let Some(id) = id {
            self.pending.insert(id.to_string(), idx);
        }
    }

    /// Resolve a pending command's outcome from its `tool_result`.
    fn resolve_result(&mut self, tool_use_id: Option<&str>, output: &str, is_error: Option<bool>) {
        let Some(idx) = tool_use_id.and_then(|id| self.pending.remove(id)) else {
            return;
        };
        if let Some(cmd) = self.commands.get_mut(idx) {
            cmd.outcome = commands::outcome(is_error, output);
            cmd.output_excerpt = commands::excerpt(output);
        }
    }
}

/// Output of segmentation: the autonomous runs plus session-level metadata.
#[derive(Debug, Clone, Default)]
pub struct Segmentation {
    pub runs: Vec<RawRun>,
    pub title: Option<String>,
    pub last_prompt: Option<String>,
    /// The first user prompt, used as a title fallback.
    pub first_prompt: Option<String>,
}

/// Segment records (in transcript order) into autonomous runs.
pub fn segment(records: &[Record]) -> Segmentation {
    let mut out = Segmentation::default();
    let mut mode = PermissionMode::Default;
    let mut open: Option<RawRun> = None;

    for record in records {
        match record {
            Record::AiTitle(t) => out.title = Some(t.ai_title.clone()),
            Record::LastPrompt(p) => out.last_prompt = Some(p.last_prompt.clone()),
            Record::User(e) if out.first_prompt.is_none() => {
                out.first_prompt = e.content_text().map(str::to_string);
            }
            _ => {}
        }

        // A line carrying a permission mode updates the ambient mode.
        if let Some(entry) = record.as_entry()
            && let Some(m) = entry.permission_mode.as_deref()
        {
            mode = parse_mode(m);
        }

        if !is_autonomous(mode) {
            if let Some(run) = open.take() {
                out.runs.push(run);
            }
            continue;
        }

        let run = open.get_or_insert_with(|| RawRun::new(mode));
        run.mode = mode;

        if let Some(entry) = record.as_entry() {
            if let Some(ts) = entry.timestamp.as_deref().and_then(parse_timestamp) {
                run.observe(ts);
            }
            let ts = entry.timestamp.as_deref().and_then(parse_timestamp);
            for block in entry.blocks() {
                // Edits (assistant turns) → the run's edit list.
                if record.is_assistant()
                    && let Some((name, input)) = block.as_tool_use()
                    && let Some(tool) = edit_tool(name)
                    && let Some(path) = input.get("file_path").and_then(|v| v.as_str())
                {
                    run.edits.push(ToolEditEvent {
                        file_path: path.into(),
                        tool,
                        message_uuid: entry.uuid.clone().unwrap_or_default(),
                        parent_uuid: entry.parent_uuid.clone(),
                    });
                }

                // `Bash` calls → commands (outcome filled in when its result lands).
                if let Some(("Bash", input)) = block.as_tool_use()
                    && let Some(command) = input.get("command").and_then(|v| v.as_str())
                {
                    run.push_command(
                        CommandRun {
                            command: command.to_string(),
                            description: input
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                            kind: commands::classify(command),
                            outcome: CommandOutcome::Unknown,
                            output_excerpt: String::new(),
                            message_uuid: entry.uuid.clone().unwrap_or_default(),
                            timestamp: ts,
                        },
                        block.tool_use_id(),
                    );
                }

                // A `tool_result` (next user turn) resolves a pending command.
                if let Some((tool_use_id, output, is_error)) = block.as_tool_result() {
                    run.resolve_result(tool_use_id, &output, is_error);
                }
            }
        }

        if let Record::FileHistorySnapshot(fhs) = record
            && let Some(snapshot) = &fhs.snapshot
        {
            // Snapshots are cumulative; the latest within the span wins.
            run.raw_backups = snapshot.tracked_file_backups.clone();
        }
    }

    // A run still open at end-of-transcript was never closed by a non-autonomous
    // turn, so the agent is (as of this read) still running it: mark it live.
    if let Some(mut run) = open.take() {
        run.ended = None;
        out.runs.push(run);
    }
    out
}

pub fn parse_mode(mode: &str) -> PermissionMode {
    match mode {
        "auto" => PermissionMode::Auto,
        "acceptEdits" => PermissionMode::AcceptEdits,
        "plan" => PermissionMode::Plan,
        _ => PermissionMode::Default,
    }
}

fn is_autonomous(mode: PermissionMode) -> bool {
    matches!(mode, PermissionMode::Auto | PermissionMode::AcceptEdits)
}

/// Parse an ISO-8601 timestamp into epoch-millis.
pub fn parse_timestamp(s: &str) -> Option<Timestamp> {
    s.parse::<jiff::Timestamp>()
        .ok()
        .map(|t| Timestamp(t.as_millisecond()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::session::EditTool;
    use crate::session::transcript::parse_reader;

    #[test]
    fn segments_autonomous_span_and_collects_edits() {
        // default → (acceptEdits span with two edits + a snapshot) → default.
        let jsonl = r#"
{"type":"user","uuid":"u1","permissionMode":"default","timestamp":"2026-01-01T00:00:00Z","message":{"content":"plan it"}}
{"type":"user","uuid":"u2","permissionMode":"acceptEdits","timestamp":"2026-01-01T00:01:00Z","message":{"content":"go"}}
{"type":"assistant","uuid":"a1","parentUuid":"u2","timestamp":"2026-01-01T00:01:05Z","message":{"content":[{"type":"text","text":"editing a"},{"type":"tool_use","name":"Edit","input":{"file_path":"/repo/a.rs"}}]}}
{"type":"file-history-snapshot","snapshot":{"trackedFileBackups":{"/repo/a.rs":{"backupFileName":"a.rs.bak","version":1}},"timestamp":"2026-01-01T00:01:06Z"}}
{"type":"assistant","uuid":"a2","parentUuid":"a1","timestamp":"2026-01-01T00:01:10Z","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/repo/b.rs"}}]}}
{"type":"user","uuid":"u3","permissionMode":"default","timestamp":"2026-01-01T00:02:00Z","message":{"content":"stop"}}
"#;
        let records = parse_reader(jsonl.as_bytes());
        let seg = segment(&records);

        assert_eq!(seg.runs.len(), 1);
        let run = &seg.runs[0];
        assert_eq!(run.mode, PermissionMode::AcceptEdits);
        assert_eq!(run.edits.len(), 2);
        assert_eq!(run.edits[0].tool, EditTool::Edit);
        assert_eq!(run.edits[1].tool, EditTool::Write);
        assert!(run.raw_backups.contains_key("/repo/a.rs"));
        assert!(run.started.unwrap().0 < run.ended.unwrap().0);
    }

    #[test]
    fn no_autonomous_records_yields_no_runs() {
        let jsonl = r#"{"type":"user","uuid":"u1","permissionMode":"default","message":{"content":"hi"}}"#;
        let seg = segment(&parse_reader(jsonl.as_bytes()));
        assert!(seg.runs.is_empty());
    }

    #[test]
    fn captures_bash_commands_and_links_results_by_id() {
        use crate::domain::session::{CommandKind, CommandOutcome};
        // A resolved test command and an unresolved one (no tool_result → live).
        let jsonl = r#"
{"type":"user","uuid":"u1","permissionMode":"acceptEdits","timestamp":"2026-01-01T00:00:00Z","message":{"content":"go"}}
{"type":"assistant","uuid":"a1","parentUuid":"u1","timestamp":"2026-01-01T00:00:05Z","message":{"content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"cargo test","description":"tests"}}]}}
{"type":"user","uuid":"r1","parentUuid":"a1","timestamp":"2026-01-01T00:00:09Z","message":{"content":[{"type":"tool_result","tool_use_id":"b1","content":"test result: ok. 3 passed; 0 failed"}]}}
{"type":"assistant","uuid":"a2","parentUuid":"r1","timestamp":"2026-01-01T00:00:12Z","message":{"content":[{"type":"tool_use","id":"b2","name":"Bash","input":{"command":"cargo build"}}]}}
"#;
        let seg = segment(&parse_reader(jsonl.as_bytes()));
        let run = &seg.runs[0];
        assert_eq!(run.commands.len(), 2);

        assert_eq!(run.commands[0].kind, CommandKind::Test);
        assert_eq!(run.commands[0].outcome, CommandOutcome::Ok);
        assert!(run.commands[0].output_excerpt.contains("3 passed"));
        assert_eq!(run.commands[0].description.as_deref(), Some("tests"));

        // No result was seen for the build command → it stays Unknown.
        assert_eq!(run.commands[1].kind, CommandKind::Build);
        assert_eq!(run.commands[1].outcome, CommandOutcome::Unknown);
    }

    #[test]
    fn run_open_at_end_of_transcript_is_live() {
        // Enters acceptEdits and never returns to a non-autonomous turn → live.
        let jsonl = r#"
{"type":"user","uuid":"u1","permissionMode":"acceptEdits","timestamp":"2026-01-01T00:00:00Z","message":{"content":"go"}}
{"type":"assistant","uuid":"a1","parentUuid":"u1","timestamp":"2026-01-01T00:00:05Z","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/repo/a.rs"}}]}}
"#;
        let seg = segment(&parse_reader(jsonl.as_bytes()));
        assert_eq!(seg.runs.len(), 1);
        assert!(seg.runs[0].ended.is_none(), "open run should be live");
    }
}
