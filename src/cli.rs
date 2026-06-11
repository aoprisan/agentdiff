//! Command-line arguments. Phase 0 only acts on `path`; the remaining flags are
//! parsed and stubbed so the surface is stable for later phases.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use crate::domain::session::Provider;

/// Which coding agent's session data to review.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ProviderArg {
    /// Claude Code (`~/.claude`).
    #[default]
    Claude,
    /// GitHub Copilot CLI (`~/.copilot`).
    Copilot,
}

impl From<ProviderArg> for Provider {
    fn from(arg: ProviderArg) -> Self {
        match arg {
            ProviderArg::Claude => Provider::Claude,
            ProviderArg::Copilot => Provider::Copilot,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "agentdiff",
    about = "Review what an AI agent changed in your working tree",
    version
)]
pub struct Args {
    /// Path to the git repository (defaults to the current directory).
    pub path: Option<PathBuf>,

    /// Which agent's session data to review (default: claude).
    #[arg(long, value_enum, default_value_t = ProviderArg::Claude)]
    pub provider: ProviderArg,

    /// Shorthand for `--provider copilot`.
    #[arg(long)]
    pub copilot: bool,

    /// Review a specific agent session id.
    #[arg(long)]
    pub session: Option<String>,

    /// Review a specific agent run within the session.
    #[arg(long)]
    pub run: Option<u32>,

    /// Diff an arbitrary git range `A..B`.
    #[arg(long)]
    pub range: Option<String>,

    /// Diff staged changes (index vs HEAD) instead of the working tree.
    #[arg(long)]
    pub staged: bool,

    /// Ignore agent session data; diff working tree vs HEAD.
    #[arg(long)]
    pub no_session: bool,

    /// Print a markdown review report (verdicts, notes, intent, verification)
    /// to stdout and exit without starting the TUI.
    #[arg(long)]
    pub report: bool,

    /// Copy this binary into a `bin` directory on your PATH and exit.
    #[arg(long)]
    pub install: bool,
}

impl Args {
    /// The resolved provider: `--copilot` overrides `--provider`.
    pub fn provider(&self) -> Provider {
        if self.copilot {
            Provider::Copilot
        } else {
            self.provider.into()
        }
    }
}
