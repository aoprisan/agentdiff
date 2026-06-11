//! Command-line arguments. Resolved into [`crate::app::Selectors`] for both the
//! TUI and the `--report` path.

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

/// Output format for `--report`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ReportFormat {
    /// Human-readable markdown, ready to paste back to the agent.
    #[default]
    Markdown,
    /// Stable structured output for piping into tools/agents.
    Json,
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

    /// Print a review report (verdicts, notes, intent, verification) to stdout
    /// and exit without starting the TUI. `--report` alone emits markdown;
    /// `--report=json` emits structured JSON.
    #[arg(
        long,
        value_enum,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "markdown"
    )]
    pub report: Option<ReportFormat>,

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_flag_defaults_to_markdown_and_accepts_json() {
        let args = Args::try_parse_from(["agentdiff", "--report"]).unwrap();
        assert_eq!(args.report, Some(ReportFormat::Markdown));

        let args = Args::try_parse_from(["agentdiff", "--report=json"]).unwrap();
        assert_eq!(args.report, Some(ReportFormat::Json));

        // `require_equals` keeps a following path positional, not a format.
        let args = Args::try_parse_from(["agentdiff", "--report", "/repo"]).unwrap();
        assert_eq!(args.report, Some(ReportFormat::Markdown));
        assert_eq!(args.path.as_deref(), Some(std::path::Path::new("/repo")));

        let args = Args::try_parse_from(["agentdiff"]).unwrap();
        assert_eq!(args.report, None);
    }
}
