# Phase 0 — Skeleton & domain contract

> Prereq: read `00-overview.md`. Goal of this phase: a runnable, panic-safe ratatui shell and the frozen `domain` types every later phase builds on. No git, no diffs yet.

## In scope
- Cargo binary project, git-init'd.
- `clap` arg parsing (stub the flags; only `[PATH]` needs to work).
- `tracing` logging to a file under the state dir (never stdout/stderr while the alt-screen is active).
- Terminal lifecycle: enter raw mode + alternate screen on start; restore on exit **and on panic** (panic hook).
- Empty 3-pane layout (file tree | diff | intent) + status bar; `q` quits, `?` shows an empty help overlay.
- The `domain` types from the overview, compiling, with `#[derive(Debug, Clone)]` and serde where persisted.

## Out of scope (deferred)
Real git diffs (Phase 1), session parsing (Phase 2), watching (Phase 3), risk (Phase 4).

## Crates to add
`ratatui`, `crossterm`, `clap` (derive), `anyhow`, `thiserror`, `tracing`, `tracing-subscriber`, `directories`, `crossbeam-channel`. Dev: `insta`.

## Tasks (ordered)
1. `cargo new --bin agentdiff` in `/Users/ao/src/github/agentdiff`; `git init`; add `.gitignore` (`/target`).
2. Add the crates above; commit the lockfile.
3. `src/error.rs`: crate `Error` (thiserror) + `Result` alias; `anyhow` at the binary edge.
4. `src/config.rs`: resolve state dir via `directories` (`ProjectDirs`); helper for log/state file paths.
5. `src/domain/{diff,review,session,ids}.rs`: define all overview types. Keep pure (no IO). Stub `ids::fingerprint(hunk) -> u64` (real impl Phase 1).
6. `src/app/{state,event,update,commands,keymap}.rs`: minimal `AppState` (current `View`, quit flag), `AppEvent::{Input,Tick}`, a reducer that handles quit/help, vim-ish keymap stub.
7. `src/tui/mod.rs`: `run()` — terminal setup, panic hook restoring the terminal, input thread → `crossbeam` channel, main loop (`select!` with ~16–33ms tick), teardown. `src/tui/layout.rs`: 3-pane + status bar split. `src/tui/widgets/{file_tree,diff_pane,intent_panel,statusbar,help}.rs`: render placeholders.
8. `src/main.rs`: parse args, init logging, call `tui::run()`.

## Files created
`Cargo.toml`, `.gitignore`, `src/{main,cli,config,error}.rs`, `src/domain/*`, `src/app/*`, `src/tui/*`.

## Acceptance
- `cargo build` and `cargo clippy` clean.
- `cargo run` opens the alt-screen with three empty panes + status bar; `?` toggles help; `q` quits and the terminal is fully restored.
- Force a panic mid-render (temporary `panic!`) → terminal still restores cleanly (panic hook works). Remove the temp panic.
- `domain` types compile and round-trip through serde where marked persistable (one `insta` snapshot test of a hand-built `Diff`).

## Definition of done
A do-nothing-but-correct TUI shell that never corrupts the terminal, plus a frozen domain contract. Commit.
