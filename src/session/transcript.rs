//! Streaming JSONL transcript parser.
//!
//! Confines all knowledge of Claude Code's on-disk transcript format to this
//! module. Records are an internally-tagged enum with a `#[serde(other)]`
//! catch-all, so an unknown or newly-added line type deserializes to
//! [`Record::Other`] rather than failing the whole parse. Each line is parsed
//! independently and a malformed line (including a partially-written trailing
//! line on a live session) is skipped, never fatal.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;

use crate::domain::session::EditTool;
use crate::error::Result;

/// One transcript line. The tag is the JSON `type` field.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Record {
    User(Entry),
    Assistant(Entry),
    LastPrompt(LastPrompt),
    AiTitle(AiTitle),
    /// Documented (planner-verified) backup record; absent in some CC versions.
    FileHistorySnapshot(FileHistorySnapshot),
    /// Standalone permission-mode record (`{"type":"permission-mode",...}`),
    /// the documented segmentation signal on CC versions that don't put a
    /// `permissionMode` field on every entry.
    PermissionMode(ModeRecord),
    /// Current CC versions write `{"type":"mode","mode":...}`. Its value
    /// vocabulary overlaps UI modes (e.g. `"normal"`), so segmentation only
    /// acts on values it positively recognizes as permission modes.
    Mode(ModeRecord),
    /// Any other line type (attachments, queue ops, future additions).
    #[serde(other)]
    Other,
}

/// Body of a standalone mode record; the mode string has been seen under both
/// `mode` and `permissionMode` keys.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModeRecord {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(rename = "permissionMode", default)]
    pub permission_mode: Option<String>,
}

impl ModeRecord {
    pub fn mode_str(&self) -> Option<&str> {
        self.mode.as_deref().or(self.permission_mode.as_deref())
    }
}

impl Record {
    pub fn as_entry(&self) -> Option<&Entry> {
        match self {
            Record::User(e) | Record::Assistant(e) => Some(e),
            _ => None,
        }
    }

    pub fn is_assistant(&self) -> bool {
        matches!(self, Record::Assistant(_))
    }
}

/// A `user` or `assistant` line. Fields absent in a given line stay `None`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Entry {
    pub uuid: Option<String>,
    #[serde(rename = "parentUuid")]
    pub parent_uuid: Option<String>,
    pub timestamp: Option<String>,
    /// The autonomy mode in effect, recorded on the line (`default`/`plan`/
    /// `acceptEdits`/`auto` depending on CC version).
    #[serde(rename = "permissionMode")]
    pub permission_mode: Option<String>,
    pub message: Option<Message>,
}

impl Entry {
    /// Content blocks (assistant turns), or empty when the content is a plain
    /// string (user prompts) or absent.
    pub fn blocks(&self) -> &[Block] {
        match self.message.as_ref().map(|m| &m.content) {
            Some(Content::Blocks(b)) => b,
            _ => &[],
        }
    }

    /// The plain-string content of a user prompt, if that's its shape.
    pub fn content_text(&self) -> Option<&str> {
        match self.message.as_ref().map(|m| &m.content) {
            Some(Content::Text(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Concatenated assistant text-block content (the agent's prose), trimmed.
    pub fn assistant_text(&self) -> Option<String> {
        let joined = self
            .blocks()
            .iter()
            .filter_map(Block::as_text)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        (!joined.is_empty()).then_some(joined)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub content: Content,
}

/// Message content is either a bare string (user prompt) or a block list. The
/// catch-all arm keeps a line with a drifted content shape (`null`, an object)
/// parseable — its other fields, like `permissionMode`, still matter for
/// segmentation even when the content is unusable.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<Block>),
    Other(#[allow(dead_code)] serde_json::Value),
}

impl Default for Content {
    fn default() -> Self {
        Content::Blocks(Vec::new())
    }
}

/// A content block. `thinking` and other future block types fall through to
/// `Other`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    ToolUse {
        /// Tool-call id used to link a later `tool_result` back to this call.
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    ToolResult {
        #[serde(default)]
        tool_use_id: Option<String>,
        /// Either a plain string or a list of content blocks; extracted via
        /// [`result_text`].
        #[serde(default)]
        content: serde_json::Value,
        /// Tool-level error flag. Unreliable for command exit status (often
        /// absent); failures usually surface as `Exit code N` in `content`.
        #[serde(default)]
        is_error: Option<bool>,
    },
    #[serde(other)]
    Other,
}

impl Block {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Block::Text { text } => Some(text.as_str()),
            _ => None,
        }
    }

    /// `(tool_name, input)` when this block is a `tool_use`.
    pub fn as_tool_use(&self) -> Option<(&str, &serde_json::Value)> {
        match self {
            Block::ToolUse { name, input, .. } => Some((name.as_str(), input)),
            _ => None,
        }
    }

    /// The `tool_use` id of this call, when present.
    pub fn tool_use_id(&self) -> Option<&str> {
        match self {
            Block::ToolUse { id, .. } => id.as_deref(),
            _ => None,
        }
    }

    /// `(tool_use_id, output_text, is_error)` when this block is a `tool_result`.
    pub fn as_tool_result(&self) -> Option<(Option<&str>, String, Option<bool>)> {
        match self {
            Block::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => Some((tool_use_id.as_deref(), result_text(content), *is_error)),
            _ => None,
        }
    }
}

/// Flatten a `tool_result`'s `content` into plain text. CC writes either a bare
/// string or a list of content blocks; anything else yields an empty string.
pub fn result_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LastPrompt {
    #[serde(rename = "lastPrompt")]
    pub last_prompt: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiTitle {
    #[serde(rename = "aiTitle")]
    pub ai_title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileHistorySnapshot {
    pub snapshot: Option<Snapshot>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Snapshot {
    #[serde(rename = "trackedFileBackups", default)]
    pub tracked_file_backups: HashMap<String, TrackedBackup>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrackedBackup {
    /// `None` (JSON `null`) means the agent created the file (no prior version).
    #[serde(rename = "backupFileName")]
    pub backup_file_name: Option<String>,
    #[serde(default)]
    pub version: u32,
}

/// Map a transcript tool name to our edit-tool enum, or `None` for non-edits.
pub fn edit_tool(name: &str) -> Option<EditTool> {
    match name {
        "Edit" => Some(EditTool::Edit),
        "Write" => Some(EditTool::Write),
        "MultiEdit" => Some(EditTool::MultiEdit),
        _ => None,
    }
}

/// Parse a transcript file into records, skipping any malformed line. Returns an
/// empty vec for a missing/unreadable file (session data is advisory).
pub fn parse_file(path: &Path) -> Result<Vec<Record>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    Ok(parse_reader(BufReader::new(file)))
}

/// Parse records from any line reader; tolerant of malformed/partial lines.
pub fn parse_reader<R: BufRead>(reader: R) -> Vec<Record> {
    let mut records = Vec::new();
    for line in reader.lines() {
        // An I/O error mid-read (or a partial trailing line) ends parsing.
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<Record>(trimmed) {
            Ok(record) => records.push(record),
            Err(e) => tracing::debug!(error = %e, "skipping unparseable transcript line"),
        }
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_and_unknown_types() {
        let jsonl = r#"
{"type":"user","uuid":"u1","parentUuid":null,"permissionMode":"default","message":{"role":"user","content":"do the thing"}}
{"type":"assistant","uuid":"a1","parentUuid":"u1","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"I'll edit the file."},{"type":"tool_use","name":"Edit","input":{"file_path":"/repo/src/a.rs"}}]}}
{"type":"ai-title","aiTitle":"My title","sessionId":"s1"}
{"type":"last-prompt","lastPrompt":"do the thing","leafUuid":"a1","sessionId":"s1"}
{"type":"some-future-thing","whatever":42}
{"type":"assistant","uuid":"a2","message":{"role":"assi"#;
        let records = parse_reader(jsonl.as_bytes());
        // The 5 well-formed lines parse; the truncated trailing line is dropped.
        assert_eq!(records.len(), 5);
        assert!(matches!(records[4], Record::Other));

        let assistant = records[1].as_entry().unwrap();
        assert_eq!(assistant.assistant_text().as_deref(), Some("I'll edit the file."));
        let (name, input) = assistant.blocks()[2].as_tool_use().unwrap();
        assert_eq!(name, "Edit");
        assert_eq!(input.get("file_path").and_then(|v| v.as_str()), Some("/repo/src/a.rs"));

        let user = records[0].as_entry().unwrap();
        assert_eq!(user.content_text(), Some("do the thing"));
        assert_eq!(user.permission_mode.as_deref(), Some("default"));
    }

    #[test]
    fn parses_bash_tool_use_and_tool_result() {
        let jsonl = concat!(
            r#"{"type":"assistant","uuid":"a1","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"cargo test","description":"run tests"}}]}}"#,
            "\n",
            r#"{"type":"user","uuid":"r1","parentUuid":"a1","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"test result: ok","is_error":false}]}}"#,
        );
        let records = parse_reader(jsonl.as_bytes());

        let use_block = &records[0].as_entry().unwrap().blocks()[0];
        assert_eq!(use_block.as_tool_use().unwrap().0, "Bash");
        assert_eq!(use_block.tool_use_id(), Some("toolu_1"));

        let result_block = &records[1].as_entry().unwrap().blocks()[0];
        let (id, text, is_error) = result_block.as_tool_result().unwrap();
        assert_eq!(id, Some("toolu_1"));
        assert_eq!(text, "test result: ok");
        assert_eq!(is_error, Some(false));
    }

    #[test]
    fn tool_result_content_array_flattens_to_text() {
        let line = r#"{"type":"user","uuid":"r1","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":[{"type":"text","text":"line one"},{"type":"text","text":"line two"}]}]}}"#;
        let records = parse_reader(line.as_bytes());
        let (_, text, is_error) = records[0].as_entry().unwrap().blocks()[0]
            .as_tool_result()
            .unwrap();
        assert_eq!(text, "line one\nline two");
        assert_eq!(is_error, None);
    }

    #[test]
    fn mode_records_parse_under_both_tags_and_key_spellings() {
        let jsonl = concat!(
            r#"{"type":"permission-mode","mode":"auto","sessionId":"s"}"#,
            "\n",
            r#"{"type":"mode","mode":"normal","sessionId":"s"}"#,
            "\n",
            r#"{"type":"permission-mode","permissionMode":"acceptEdits"}"#,
        );
        let records = parse_reader(jsonl.as_bytes());
        assert_eq!(records.len(), 3);
        let modes: Vec<_> = records
            .iter()
            .map(|r| match r {
                Record::PermissionMode(m) | Record::Mode(m) => m.mode_str(),
                _ => panic!("expected mode records"),
            })
            .collect();
        assert_eq!(modes, vec![Some("auto"), Some("normal"), Some("acceptEdits")]);
    }

    #[test]
    fn drifted_content_shapes_do_not_drop_the_line() {
        // `content: null` (or an object) must not lose the line — its other
        // fields (permissionMode) still drive run segmentation.
        let jsonl = concat!(
            r#"{"type":"user","uuid":"u1","permissionMode":"acceptEdits","message":{"content":null}}"#,
            "\n",
            r#"{"type":"user","uuid":"u2","message":{"content":{"weird":true}}}"#,
        );
        let records = parse_reader(jsonl.as_bytes());
        assert_eq!(records.len(), 2);
        let entry = records[0].as_entry().unwrap();
        assert_eq!(entry.permission_mode.as_deref(), Some("acceptEdits"));
        assert!(entry.blocks().is_empty());
        assert!(entry.content_text().is_none());
    }

    #[test]
    fn unknown_block_types_are_ignored() {
        let line = r#"{"type":"assistant","uuid":"a","message":{"content":[{"type":"redacted_thinking","data":"x"}]}}"#;
        let records = parse_reader(line.as_bytes());
        assert_eq!(records.len(), 1);
        assert!(records[0].as_entry().unwrap().assistant_text().is_none());
    }
}
