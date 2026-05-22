//! Claude Code session model. Phase 0 defines the shapes; `session/` populates
//! them from the transcript JSONL and `~/.claude/file-history` in Phase 2.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::Timestamp;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

/// The nth autonomous (`auto`/`acceptEdits`) span within a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionMode {
    Auto,
    Plan,
    Default,
    AcceptEdits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditTool {
    Edit,
    Write,
    MultiEdit,
}

/// One agent edit, as recorded by a `tool_use` block in the transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEditEvent {
    pub file_path: PathBuf,
    pub tool: EditTool,
    pub message_uuid: String,
    pub parent_uuid: Option<String>,
}

/// A file's pre-run state. `backup_path: None` means the agent created the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    pub backup_path: Option<PathBuf>,
    pub version: u32,
}

/// One autonomous run — our diff-scoping unit. `ended: None` means it's live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: RunId,
    pub mode: PermissionMode,
    pub started: Timestamp,
    pub ended: Option<Timestamp>,
    /// Path -> pre-run backup, folded from the run's `file-history-snapshot`s.
    pub snapshot: HashMap<PathBuf, Backup>,
    pub edits: Vec<ToolEditEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: SessionId,
    pub project_slug: String,
    pub file: PathBuf,
    pub runs: Vec<AgentRun>,
    pub last_prompt: Option<String>,
    pub title: Option<String>,
}

/// The agent's stated reasoning for editing a file, recovered by walking the
/// transcript's `parentUuid` chain (Phase 2). `f32` precludes `Eq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    pub file_path: PathBuf,
    pub text: String,
    pub source_uuid: String,
    pub confidence: f32,
}
