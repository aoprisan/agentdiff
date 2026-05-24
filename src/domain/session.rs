//! Claude Code session model. Phase 0 defines the shapes; `session/` populates
//! them from the transcript JSONL and `~/.claude/file-history` in Phase 2.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::Timestamp;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

/// Which coding agent produced a session. Selects where session data is read
/// from and how it's parsed; everything downstream of `SessionContext` is
/// identical across providers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    #[default]
    Claude,
    Copilot,
}

impl Provider {
    /// Human label for the UI / diff-base summary.
    pub fn label(self) -> &'static str {
        match self {
            Provider::Claude => "Claude Code",
            Provider::Copilot => "Copilot",
        }
    }
}

/// The nth autonomous span within a session (`auto`/`acceptEdits` for Claude,
/// `autopilot` for Copilot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionMode {
    Auto,
    Plan,
    Default,
    AcceptEdits,
    /// Copilot's autonomous mode (the analog of Claude's `Auto`).
    Autopilot,
    /// Copilot's interactive mode (asks before each action).
    Interactive,
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

/// Coarse classification of a shell command the agent ran, used to single out
/// verification work (tests/build/lint) from incidental commands. Heuristic and
/// advisory — see `session::commands::classify`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandKind {
    Test,
    Build,
    Lint,
    Format,
    Vcs,
    Run,
    Other,
}

impl CommandKind {
    /// Short lowercase label for the verification badge / overlay.
    pub fn label(self) -> &'static str {
        match self {
            CommandKind::Test => "test",
            CommandKind::Build => "build",
            CommandKind::Lint => "lint",
            CommandKind::Format => "fmt",
            CommandKind::Vcs => "git",
            CommandKind::Run => "run",
            CommandKind::Other => "cmd",
        }
    }

    /// Whether this kind is "verification" — the work that tells a reviewer the
    /// change was checked. Drives the compact header summary.
    pub fn is_verification(self) -> bool {
        matches!(
            self,
            CommandKind::Test | CommandKind::Build | CommandKind::Lint | CommandKind::Format
        )
    }
}

/// The recovered result of a command. `Unknown` means no `tool_result` was seen
/// for it (e.g. a live run still mid-command); outcome is otherwise inferred
/// heuristically from the result content — see `session::commands::outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandOutcome {
    Ok,
    Failed,
    Unknown,
}

/// A shell command the agent ran during a run, paired with the recovered outcome
/// of its result. Advisory: both `kind` and `outcome` are heuristics over the
/// transcript, never authoritative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRun {
    pub command: String,
    pub description: Option<String>,
    pub kind: CommandKind,
    pub outcome: CommandOutcome,
    /// Trimmed tail of the result output, for the verification detail overlay.
    pub output_excerpt: String,
    pub message_uuid: String,
    pub timestamp: Option<Timestamp>,
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
    /// Shell commands the agent ran during the span, in transcript order.
    pub commands: Vec<CommandRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: SessionId,
    pub provider: Provider,
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
