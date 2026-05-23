//! agentdiff — a TUI git-diff tool for reviewing what an AI agent did.
//!
//! Phase 0: a panic-safe ratatui shell over the frozen `domain` contract. No git
//! or session integration yet — see `~/.claude/plans/agentdiff/` for the roadmap.

mod app;
mod cli;
mod config;
// Domain types (and their convenience re-exports) are defined ahead of their
// first use — the Claude-session producers land in Phase 2 — so silence
// dead-code/unused-import warnings for the contract module only.
#[allow(dead_code, unused_imports)]
mod domain;
mod error;
mod git;
mod session;
mod tui;

use std::path::Path;

use anyhow::Context;
use clap::Parser;

fn main() -> anyhow::Result<()> {
    let args = cli::Args::parse();
    let paths = config::paths().context("resolving application paths")?;
    init_logging(&paths.log_file).context("initializing logging")?;

    tracing::info!(
        state_dir = %paths.state_dir.display(),
        log_file = %paths.log_file.display(),
        "agentdiff starting"
    );
    tracing::debug!(?args, "parsed args");

    tui::run(args, paths.state_dir)?;

    tracing::info!("agentdiff exiting cleanly");
    Ok(())
}

/// Send `tracing` output to an append-only log file. We must never write to the
/// terminal while the alternate screen is active.
fn init_logging(log_file: &Path) -> anyhow::Result<()> {
    use std::fs::OpenOptions;

    let file = OpenOptions::new().create(true).append(true).open(log_file)?;
    // `Fn() -> impl Write` satisfies `MakeWriter`; `try_clone` shares the fd so
    // each event appends to the same file.
    let make_writer = move || file.try_clone().expect("clone log file handle");

    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(make_writer)
        .init();
    Ok(())
}
