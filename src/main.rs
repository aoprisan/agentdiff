//! agentdiff — a TUI git-diff tool for reviewing what an AI agent did. See
//! `docs/plan/` for the architecture and phase roadmap.

mod app;
mod cli;
mod config;
// A few domain types/fields are still ahead of their consumers (e.g. the
// phase-4 risk engine), so silence dead-code warnings for the contract module.
#[allow(dead_code, unused_imports)]
mod domain;
mod error;
mod git;
mod install;
mod session;
mod tui;
mod watch;

use std::path::Path;

use anyhow::Context;
use clap::Parser;

fn main() -> anyhow::Result<()> {
    let args = cli::Args::parse();

    // `--install` is a one-shot side task: copy the binary onto PATH and exit
    // before we touch the state dir, logging, or the terminal.
    if args.install {
        return install::install();
    }

    let paths = config::paths().context("resolving application paths")?;
    init_logging(&paths.log_file).context("initializing logging")?;

    tracing::info!(
        state_dir = %paths.state_dir.display(),
        log_file = %paths.log_file.display(),
        "agentdiff starting"
    );
    tracing::debug!(?args, "parsed args");

    // `--report` is the non-interactive surface: build the same state the TUI
    // would and print it instead of entering the alternate screen.
    if let Some(format) = args.report {
        return report(&args, &paths.state_dir, format);
    }

    tui::run(args, paths.state_dir)?;

    tracing::info!("agentdiff exiting cleanly");
    Ok(())
}

/// Build the review state exactly as the TUI would and print the report to
/// stdout — verdicts, notes, intent, and verification outcomes.
fn report(args: &cli::Args, state_dir: &Path, format: cli::ReportFormat) -> anyhow::Result<()> {
    use std::path::PathBuf;

    let start = args.path.clone().unwrap_or_else(|| PathBuf::from("."));
    let repo = git::Repo::discover(&start)
        .with_context(|| format!("opening a git repository at {}", start.display()))?;
    let dirs = session::AgentDirs::discover();
    let selectors = app::Selectors::from_args(args);
    let state = app::build_state(&repo, state_dir, &dirs, &selectors)?;
    let output = match format {
        cli::ReportFormat::Markdown => app::report::render_markdown(&state),
        cli::ReportFormat::Json => app::report::render_json(&state),
    };
    print!("{output}");
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
