//! Segmenting a transcript into autonomous **runs**.
//!
//! A run is a maximal contiguous span during which the agent was in an
//! autonomous permission mode (`auto`/`acceptEdits`). Within a span we collect
//! the agent's edit events and fold in the (cumulative) `file-history-snapshot`
//! records, keeping the latest within the span as the pre-run file map. The raw
//! per-path backups are resolved to on-disk paths later by `backups`.

use std::collections::HashMap;

use crate::domain::Timestamp;
use crate::domain::session::{PermissionMode, ToolEditEvent};

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
}

impl RawRun {
    fn new(mode: PermissionMode) -> Self {
        Self {
            mode,
            started: None,
            ended: None,
            edits: Vec::new(),
            raw_backups: HashMap::new(),
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
            if record.is_assistant() {
                for (name, input) in entry.blocks().iter().filter_map(|b| b.as_tool_use()) {
                    let Some(tool) = edit_tool(name) else { continue };
                    let Some(path) = input.get("file_path").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    run.edits.push(ToolEditEvent {
                        file_path: path.into(),
                        tool,
                        message_uuid: entry.uuid.clone().unwrap_or_default(),
                        parent_uuid: entry.parent_uuid.clone(),
                    });
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

    if let Some(run) = open.take() {
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
}
