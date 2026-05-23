//! Command-line arguments. Phase 0 only acts on `path`; the remaining flags are
//! parsed and stubbed so the surface is stable for later phases.

use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "agentdiff",
    about = "Review what an AI agent changed in your working tree",
    version
)]
pub struct Args {
    /// Path to the git repository (defaults to the current directory).
    pub path: Option<PathBuf>,

    /// Review a specific Claude Code session id (Phase 2).
    #[arg(long)]
    pub session: Option<String>,

    /// Review a specific agent run within the session (Phase 2).
    #[arg(long)]
    pub run: Option<u32>,

    /// Diff an arbitrary git range `A..B` (Phase 3).
    #[arg(long)]
    pub range: Option<String>,

    /// Diff staged changes (index vs HEAD) instead of the working tree.
    #[arg(long)]
    pub staged: bool,

    /// Ignore Claude Code session data; diff working tree vs HEAD (Phase 2).
    #[arg(long)]
    pub no_session: bool,
}
