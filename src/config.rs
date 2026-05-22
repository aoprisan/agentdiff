//! Filesystem locations for state and logs.
//!
//! Phase 0 only needs a state directory and a log file path; later phases add
//! the persisted review-state file and the config (keymap/theme) loader here.

use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;

use crate::error::Result;

/// Resolved on-disk locations for this run.
pub struct Paths {
    /// Directory holding persisted state (review verdicts land here in Phase 1).
    pub state_dir: PathBuf,
    /// Append-only log file (we never log to stdout/stderr while the TUI owns
    /// the terminal).
    pub log_file: PathBuf,
}

/// Resolve and create the application's state directory.
pub fn paths() -> Result<Paths> {
    let state_dir = ProjectDirs::from("dev", "agentdiff", "agentdiff")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".agentdiff"));
    fs::create_dir_all(&state_dir)?;
    let log_file = state_dir.join("agentdiff.log");
    Ok(Paths { state_dir, log_file })
}
