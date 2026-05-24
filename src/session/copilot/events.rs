//! Streaming parser for Copilot CLI's `events.jsonl`.
//!
//! Confines all knowledge of Copilot's on-disk event format to this module.
//! Each line is one event: a `type` tag, a `data` payload whose shape depends on
//! the type, and `id`/`parentId`/`timestamp`. Rather than a giant tagged enum we
//! keep `data` as a [`serde_json::Value`] and expose typed accessors, so an
//! unknown or newly-added event type simply doesn't match any accessor instead
//! of failing the parse. Each line is parsed independently; a malformed or
//! partially-written trailing line (a live session) is skipped, never fatal.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;

use crate::domain::session::EditTool;
use crate::error::Result;

/// One `events.jsonl` line. `data`'s shape varies by `kind`; read it via the
/// accessors below.
#[derive(Debug, Clone, Deserialize)]
pub struct RawEvent {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub data: serde_json::Value,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "parentId", default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
}

impl RawEvent {
    fn data_str(&self, key: &str) -> Option<&str> {
        self.data.get(key).and_then(|v| v.as_str())
    }

    /// `data.context` of a `session.start` event, as `(cwd, git_root)`.
    pub fn start_context(&self) -> Option<(Option<&str>, Option<&str>)> {
        (self.kind == "session.start").then(|| {
            let ctx = self.data.get("context");
            (
                ctx.and_then(|c| c.get("cwd")).and_then(|v| v.as_str()),
                ctx.and_then(|c| c.get("gitRoot")).and_then(|v| v.as_str()),
            )
        })
    }

    /// The new permission mode of a `session.mode_changed` event.
    pub fn new_mode(&self) -> Option<&str> {
        (self.kind == "session.mode_changed")
            .then(|| self.data_str("newMode"))
            .flatten()
    }

    /// The previous permission mode of a `session.mode_changed` event.
    pub fn previous_mode(&self) -> Option<&str> {
        (self.kind == "session.mode_changed")
            .then(|| self.data_str("previousMode"))
            .flatten()
    }

    /// `(tool_name, arguments)` for a `tool.execution_start` event.
    pub fn tool_start(&self) -> Option<(&str, &serde_json::Value)> {
        (self.kind == "tool.execution_start")
            .then(|| self.data_str("toolName").map(|n| (n, &self.data["arguments"])))
            .flatten()
    }

    /// `(success, result_text)` for a `tool.execution_complete` event.
    pub fn tool_complete(&self) -> Option<(bool, String)> {
        if self.kind != "tool.execution_complete" {
            return None;
        }
        let success = self.data.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
        let text = self
            .data
            .get("result")
            .and_then(|r| r.get("content"))
            .map(value_text)
            .unwrap_or_default();
        Some((success, text))
    }

    /// The `toolCallId` linking a start to its completion, if present.
    pub fn tool_call_id(&self) -> Option<&str> {
        self.data_str("toolCallId")
    }

    /// The agent's prose for an `assistant.message`: its `content`, falling back
    /// to `reasoningText` (often where the "why" lives when content is a bare
    /// tool-call turn). Trimmed; `None` when both are empty.
    pub fn assistant_text(&self) -> Option<String> {
        if self.kind != "assistant.message" {
            return None;
        }
        let content = self.data_str("content").unwrap_or("").trim();
        if !content.is_empty() {
            return Some(content.to_string());
        }
        let reasoning = self.data_str("reasoningText").unwrap_or("").trim();
        (!reasoning.is_empty()).then(|| reasoning.to_string())
    }

    /// The user's prompt text for a `user.message` (`content`, then the
    /// expanded `transformedContent`).
    pub fn user_text(&self) -> Option<String> {
        if self.kind != "user.message" {
            return None;
        }
        for key in ["content", "transformedContent"] {
            if let Some(s) = self.data_str(key) {
                let t = s.trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
        None
    }

    /// The `turnId` this event belongs to, when present.
    pub fn turn_id(&self) -> Option<&str> {
        self.data_str("turnId")
    }
}

/// Map a Copilot tool name to our edit-tool enum, or `None` for non-edits.
pub fn edit_tool(name: &str) -> Option<EditTool> {
    match name {
        "create" => Some(EditTool::Write),
        "edit" | "str_replace" => Some(EditTool::Edit),
        "apply_patch" => Some(EditTool::MultiEdit),
        _ => None,
    }
}

/// The `path` argument of an edit tool call (`create`/`edit`), if present.
pub fn edit_path(args: &serde_json::Value) -> Option<&str> {
    args.get("path").and_then(|v| v.as_str())
}

/// Flatten a tool-result `content` into plain text: a bare string, or a list of
/// `{text}` blocks. Anything else yields an empty string.
fn value_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .or_else(|| item.get("text").and_then(|t| t.as_str()).map(str::to_string))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Parse an `events.jsonl` file, skipping malformed lines. An empty vec is
/// returned for a missing/unreadable file (session data is advisory).
pub fn parse_file(path: &Path) -> Result<Vec<RawEvent>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    Ok(parse_reader(BufReader::new(file)))
}

/// Parse events from any line reader; tolerant of malformed/partial lines.
pub fn parse_reader<R: BufRead>(reader: R) -> Vec<RawEvent> {
    let mut events = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<RawEvent>(trimmed) {
            Ok(event) => events.push(event),
            Err(e) => tracing::debug!(error = %e, "skipping unparseable copilot event line"),
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_and_unknown_event_types() {
        let jsonl = concat!(
            r#"{"type":"session.start","id":"e0","timestamp":"2026-05-20T10:00:00Z","data":{"context":{"cwd":"/repo/sub","gitRoot":"/repo"}}}"#,
            "\n",
            r#"{"type":"session.mode_changed","id":"e1","parentId":"e0","data":{"previousMode":"interactive","newMode":"autopilot"}}"#,
            "\n",
            r#"{"type":"assistant.message","id":"e2","parentId":"e1","data":{"turnId":"1","content":"","reasoningText":"I'll create the file."}}"#,
            "\n",
            r#"{"type":"tool.execution_start","id":"e3","parentId":"e2","data":{"toolCallId":"t1","toolName":"create","turnId":"1","arguments":{"path":"/repo/a.rs","file_text":"x"}}}"#,
            "\n",
            r#"{"type":"tool.execution_complete","id":"e4","parentId":"e3","data":{"toolCallId":"t1","success":true,"result":{"content":"ok"}}}"#,
            "\n",
            r#"{"type":"some.future.thing","id":"e5","data":{"whatever":1}}"#,
            "\n",
            r#"{"type":"assistant.message","id":"e6","data":{"content":"partial"#,
        );
        let events = parse_reader(jsonl.as_bytes());
        // Six well-formed lines parse; the truncated trailing line is dropped.
        assert_eq!(events.len(), 6);

        assert_eq!(events[0].start_context(), Some((Some("/repo/sub"), Some("/repo"))));
        assert_eq!(events[1].new_mode(), Some("autopilot"));
        assert_eq!(events[1].previous_mode(), Some("interactive"));
        assert_eq!(events[2].assistant_text().as_deref(), Some("I'll create the file."));

        let (name, args) = events[3].tool_start().unwrap();
        assert_eq!(name, "create");
        assert_eq!(edit_tool(name), Some(EditTool::Write));
        assert_eq!(edit_path(args), Some("/repo/a.rs"));
        assert_eq!(events[3].tool_call_id(), Some("t1"));

        assert_eq!(events[4].tool_complete(), Some((true, "ok".to_string())));
        // An unknown event type matches no accessor but still parses.
        assert!(events[5].tool_start().is_none());
        assert!(events[5].assistant_text().is_none());
    }
}
