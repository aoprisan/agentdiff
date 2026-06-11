//! ratatui rendering plus the terminal/event lifecycle. Reads `AppState`;
//! contains no business logic.

pub mod highlight;
pub mod layout;
pub mod theme;
pub mod widgets;

use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::Context;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use ratatui::Frame;
use ratatui::crossterm::event;
use ratatui::layout::Rect;

use crate::app::{self, AppEvent, AppState, Keymap, Selectors, View, update};
use crate::cli::Args;
use crate::config::{self, ThemeConfig};
use crate::domain::session::Provider;
use crate::git::Repo;
use crate::session::{AgentDirs, copilot, locate};
use highlight::Highlighter;

/// A request to the background re-diff worker.
struct DiffRequest {
    generation: u64,
    selectors: Selectors,
}

/// Build the diff/review/session state, then run the panic-safe terminal loop,
/// always restoring the terminal — including on panic — and persisting verdicts.
pub fn run(args: Args, state_dir: PathBuf) -> anyhow::Result<()> {
    // Everything that can fail happens before we touch the terminal, so errors
    // print normally rather than from inside the alternate screen.
    let start = args.path.clone().unwrap_or_else(|| PathBuf::from("."));
    let repo = Repo::discover(&start)
        .with_context(|| format!("opening a git repository at {}", start.display()))?;
    let dirs = AgentDirs::discover();

    // User config: theme is global; the syntax theme and keymap are threaded
    // through so they survive a session switch.
    let config = config::load_config();
    let palette = resolve_palette(&config.theme);
    theme::install(palette);
    let keymap = Keymap::from_overrides(&config.keys);
    let syntax_theme = config
        .theme
        .syntax
        .clone()
        .unwrap_or_else(|| palette.syntax.to_string());

    let selectors = Selectors::from_args(&args);
    let mut state = app::build_state(&repo, &state_dir, &dirs, &selectors)?;
    state.keymap = keymap.clone();

    install_panic_hook();
    let mut terminal = ratatui::init();
    let mut highlighter = Highlighter::with_theme(&syntax_theme);
    let result = event_loop(
        &mut terminal,
        &mut state,
        &mut highlighter,
        &repo,
        &state_dir,
        dirs,
        selectors,
        keymap,
        syntax_theme,
    );
    ratatui::restore();

    // Persist after restoring so any write error surfaces on the real terminal.
    save_review(&state);
    result
}

#[allow(clippy::too_many_arguments)]
fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    state: &mut AppState,
    highlighter: &mut Highlighter,
    repo: &Repo,
    state_dir: &Path,
    dirs: AgentDirs,
    initial_selectors: Selectors,
    keymap: Keymap,
    syntax_theme: String,
) -> anyhow::Result<()> {
    let (tx, rx) = unbounded::<AppEvent>();

    // Blocking input reads live on their own thread. When the loop drops `rx`,
    // the next `send` fails and this thread exits.
    {
        let tx = tx.clone();
        thread::spawn(move || {
            while let Ok(ev) = event::read() {
                if tx.send(AppEvent::Input(ev)).is_err() {
                    break;
                }
            }
        });
    }

    // Background re-diff worker: opens its own (non-Send) repo handle and rebuilds
    // the diff bundle on demand, tagging each result with its request generation.
    let (req_tx, req_rx) = unbounded::<DiffRequest>();
    spawn_worker(repo.workdir().to_path_buf(), dirs.clone(), req_rx, tx.clone());

    let mut selectors = initial_selectors;
    // Watch the tree + active transcript; the guard must outlive the loop.
    let mut _watch = spawn_watch(state, &dirs, repo.workdir(), tx.clone());

    while !state.should_quit {
        // Refresh the diff-pane height so paging/scrolling matches what's drawn.
        if let Ok(size) = terminal.size() {
            let panes = layout::compute(Rect::new(0, 0, size.width, size.height));
            state.viewport_height = (panes.diff.height.saturating_sub(2)).max(1) as usize;
            state.ensure_cursor_visible();
        }
        terminal.draw(|frame| render(frame, state, highlighter))?;

        match rx.recv_timeout(Duration::from_millis(33)) {
            Ok(AppEvent::FsChanged) => {
                state.generation += 1;
                let _ = req_tx.send(DiffRequest {
                    generation: state.generation,
                    selectors: selectors.clone(),
                });
            }
            Ok(AppEvent::DiffReady { generation, bundle }) => {
                if generation == state.generation {
                    let bundle = *bundle;
                    state.apply_rediff(bundle.diff, bundle.intent, bundle.session, bundle.sessions);
                    // Indices changed, so cached highlights may be stale.
                    highlighter.clear();
                }
            }
            Ok(ev) => update(state, ev),
            Err(RecvTimeoutError::Timeout) => update(state, AppEvent::Tick),
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if let Some(id) = state.pending_switch.take() {
            switch_session(state, repo, state_dir, &dirs, &mut selectors, id);
            // Reapply config that lives outside the rebuilt AppState / Highlighter.
            state.keymap = keymap.clone();
            *highlighter = Highlighter::with_theme(&syntax_theme);
            _watch = spawn_watch(state, &dirs, repo.workdir(), tx.clone());
        }
    }
    Ok(())
}

/// Resolve the active palette: a built-in base (by name) with `#rrggbb`
/// per-color overrides layered on top.
fn resolve_palette(theme_config: &ThemeConfig) -> theme::Palette {
    let base = match &theme_config.name {
        Some(name) => theme::Palette::by_name(name).unwrap_or_else(|| {
            tracing::warn!(name, "unknown theme name; using default palette");
            theme::Palette::default()
        }),
        None => theme::Palette::default(),
    };
    base.with_overrides(
        theme_config.added.as_deref().and_then(theme::parse_color),
        theme_config.removed.as_deref().and_then(theme::parse_color),
        theme_config.intent.as_deref().and_then(theme::parse_color),
    )
}

fn spawn_worker(
    workdir: PathBuf,
    dirs: AgentDirs,
    req_rx: Receiver<DiffRequest>,
    tx: Sender<AppEvent>,
) {
    thread::spawn(move || {
        let repo = match Repo::discover(&workdir) {
            Ok(repo) => repo,
            Err(e) => {
                tracing::warn!(error = %e, "re-diff worker could not open repo; live updates disabled");
                return;
            }
        };
        while let Ok(req) = req_rx.recv() {
            match app::build_bundle(&repo, &dirs, &req.selectors) {
                Ok(bundle) => {
                    let event = AppEvent::DiffReady {
                        generation: req.generation,
                        bundle: Box::new(bundle),
                    };
                    if tx.send(event).is_err() {
                        break;
                    }
                }
                Err(e) => tracing::warn!(error = %e, "background re-diff failed"),
            }
        }
    });
}

fn spawn_watch(
    state: &AppState,
    dirs: &AgentDirs,
    workdir: &Path,
    tx: Sender<AppEvent>,
) -> Option<crate::watch::Watch> {
    let session_file = active_session_file(state, dirs, workdir);
    crate::watch::spawn(workdir, session_file, tx)
}

/// The transcript file of the loaded session, watched for live updates. Resolved
/// for whichever provider produced it.
fn active_session_file(state: &AppState, dirs: &AgentDirs, workdir: &Path) -> Option<PathBuf> {
    let session = state.session.as_ref()?;
    let id = session.id.as_str();
    match session.provider {
        Provider::Claude => locate::find_session(dirs.claude.as_deref()?, workdir, id).map(|e| e.path),
        Provider::Copilot => {
            copilot::find_session(dirs.copilot.as_deref()?, workdir, id).map(|e| e.events)
        }
    }
}

/// Reload review state for a different session picked in the picker, and point
/// future re-diffs at it. The outgoing verdicts are persisted first; the caller
/// reapplies keymap/highlighter (which live outside the rebuilt `AppState`). The
/// active provider is preserved across the switch.
fn switch_session(
    state: &mut AppState,
    repo: &Repo,
    state_dir: &Path,
    dirs: &AgentDirs,
    selectors: &mut Selectors,
    session_id: String,
) {
    save_review(state);
    *selectors = Selectors {
        provider: selectors.provider,
        no_session: false,
        session_id: Some(session_id.clone()),
        run_index: None,
        range: None,
        staged: false,
    };
    match app::build_state(repo, state_dir, dirs, selectors) {
        Ok(new_state) => *state = new_state,
        Err(e) => tracing::warn!(error = %e, session = session_id, "failed to switch session"),
    }
}

fn save_review(state: &AppState) {
    if state.review_dirty
        && let Err(e) = config::save_review_state(&state.state_path, &state.review)
    {
        tracing::warn!(error = %e, "failed to save review state");
    }
}

fn render(frame: &mut Frame, state: &AppState, highlighter: &mut Highlighter) {
    // Paint the palette's canvas first. Widgets render on top with transparent
    // (unset) backgrounds, so context lines, borders and gaps pick up the theme
    // background instead of the terminal's own. `Color::Reset` is a no-op that
    // defers to the terminal, preserving the default theme's look.
    use ratatui::style::Style;
    use ratatui::widgets::Block;
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::bg()).fg(theme::fg())),
        frame.area(),
    );

    match state.view {
        View::Review => render_review(frame, state, highlighter),
    }
    if state.note_edit.is_some() {
        widgets::notes::render(frame, frame.area(), state);
    } else if state.search_edit.is_some() {
        widgets::search::render(frame, frame.area(), state);
    } else if state.show_picker {
        widgets::session_picker::render(frame, frame.area(), state);
    } else if state.show_verify {
        widgets::verification::render(frame, frame.area(), state);
    } else if state.show_help {
        widgets::help::render(frame, frame.area());
    }
}

fn render_review(frame: &mut Frame, state: &AppState, highlighter: &mut Highlighter) {
    let panes = layout::compute(frame.area());
    widgets::file_tree::render(frame, panes.file_tree, state);
    widgets::diff_pane::render(frame, panes.diff, state, highlighter);
    widgets::intent_panel::render(frame, panes.intent, state);
    widgets::statusbar::render(frame, panes.status, state);
}

/// Restore the terminal before the default panic handler prints, so a panic in
/// any thread never leaves the user stuck in a broken alternate screen.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::diff::{
        ChangeKind, Diff, DiffBase, FileChange, FileId, Hunk, InlineSpan, Line, LineKind, LineRange,
    };
    use crate::domain::review::{HunkRef, HunkVerdict, ReviewState};
    use crate::domain::{Timestamp, ids::fingerprint};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn sample_diff() -> Diff {
        let lines = vec![
            Line {
                kind: LineKind::Context,
                old_no: Some(1),
                new_no: Some(1),
                text: "fn main() {".into(),
                intra: Vec::new(),
            },
            Line {
                kind: LineKind::Removed,
                old_no: Some(2),
                new_no: None,
                text: "    let x = 1;".into(),
                intra: vec![InlineSpan {
                    start: 12,
                    end: 13,
                    changed: true,
                }],
            },
            Line {
                kind: LineKind::Added,
                old_no: None,
                new_no: Some(2),
                text: "    let x = 2;".into(),
                intra: vec![InlineSpan {
                    start: 12,
                    end: 13,
                    changed: true,
                }],
            },
        ];
        let path = PathBuf::from("src/main.rs");
        let hunk = Hunk {
            href: HunkRef {
                path: path.clone(),
                fingerprint: fingerprint(&path, &lines),
            },
            old: LineRange { start: 1, count: 2 },
            new: LineRange { start: 1, count: 2 },
            header: "@@ -1,2 +1,2 @@ fn main()".into(),
            lines,
        };
        Diff {
            base: DiffBase::WorkingTreeVsHead,
            generated_at: Timestamp::from_millis(0),
            files: vec![FileChange {
                id: FileId(0),
                path,
                old_path: None,
                change: ChangeKind::Modified,
                is_binary: false,
                is_created: false,
                language: Some("rust".into()),
                hunks: vec![hunk],
                stats: (1, 1),
            }],
        }
    }

    fn sample_state() -> AppState {
        AppState::new(
            sample_diff(),
            ReviewState::default(),
            PathBuf::from("/tmp/review.toml"),
        )
    }

    fn sample_commands() -> Vec<crate::domain::session::CommandRun> {
        use crate::domain::session::{CommandKind, CommandOutcome, CommandRun};
        vec![
            CommandRun {
                command: "cargo test --all".into(),
                description: Some("run the test suite".into()),
                kind: CommandKind::Test,
                outcome: CommandOutcome::Ok,
                output_excerpt: "test result: ok. 42 passed; 0 failed".into(),
                message_uuid: "c1".into(),
                timestamp: None,
            },
            CommandRun {
                command: "cargo clippy --all-targets".into(),
                description: None,
                kind: CommandKind::Lint,
                outcome: CommandOutcome::Failed,
                output_excerpt: "error: unused variable `x`\nExit code 1".into(),
                message_uuid: "c2".into(),
                timestamp: None,
            },
        ]
    }

    fn render_to_string(state: &AppState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let mut hl = Highlighter::new();
        terminal
            .draw(|frame| render(frame, state, &mut hl))
            .unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn renders_empty_review_layout() {
        let state = AppState::new(
            Diff {
                base: DiffBase::WorkingTreeVsHead,
                generated_at: Timestamp::from_millis(0),
                files: Vec::new(),
            },
            ReviewState::default(),
            PathBuf::from("/tmp/review.toml"),
        );
        insta::assert_snapshot!(render_to_string(&state));
    }

    #[test]
    fn renders_diff_pane() {
        insta::assert_snapshot!(render_to_string(&sample_state()));
    }

    #[test]
    fn solarized_paints_the_whole_canvas_background() {
        // The only test that installs a palette, so this OnceLock set always wins
        // and the assertion is deterministic regardless of test ordering.
        theme::install(theme::Palette::solarized_dark());
        assert_eq!(theme::bg(), theme::Palette::solarized_dark().bg);

        let state = sample_state();
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let mut hl = Highlighter::new();
        terminal
            .draw(|frame| render(frame, &state, &mut hl))
            .unwrap();

        let base03 = ratatui::style::Color::Rgb(0x00, 0x2b, 0x36);
        let buf = terminal.backend().buffer();
        let painted = buf.content().iter().filter(|c| c.bg == base03).count();
        let total = buf.content().len();
        // The canvas is painted edge to edge; only the cursor row and word-diff
        // emphasis use a different bg, so the vast majority of cells are base03.
        assert!(
            painted > total * 3 / 4,
            "expected most cells painted base03, got {painted}/{total}"
        );
    }

    #[test]
    fn renders_verdict_markers() {
        let mut state = sample_state();
        let href = state.diff.files[0].hunks[0].href.clone();
        state.review.set_verdict(href, HunkVerdict::Approved);
        insta::assert_snapshot!(render_to_string(&state));
    }

    #[test]
    fn renders_help_overlay() {
        let mut state = sample_state();
        state.show_help = true;
        insta::assert_snapshot!(render_to_string(&state));
    }

    #[test]
    fn renders_intent_panel() {
        use crate::app::state::SessionSummary;
        use crate::domain::session::Intent;

        let mut state = sample_state();
        state.session = Some(SessionSummary {
            provider: Provider::Claude,
            id: "11111111-aaaa".into(),
            title: Some("Add greeting, fix off-by-one".into()),
            last_prompt: Some("thanks".into()),
            base_label: "agent run 1/1 (acceptEdits)".into(),
            live: true,
            commands: sample_commands(),
        });
        state.intent.insert(
            PathBuf::from("src/main.rs"),
            Intent {
                file_path: PathBuf::from("src/main.rs"),
                text: "Bump the constant so the example reflects the new default.".into(),
                source_uuid: "a1".into(),
                confidence: 0.9,
            },
        );
        insta::assert_snapshot!(render_to_string(&state));
    }

    #[test]
    fn renders_verification_overlay() {
        use crate::app::state::SessionSummary;

        let mut state = sample_state();
        state.session = Some(SessionSummary {
            provider: Provider::Claude,
            id: "11111111-aaaa".into(),
            title: Some("Add greeting, fix off-by-one".into()),
            last_prompt: Some("thanks".into()),
            base_label: "agent run 1/1 (acceptEdits)".into(),
            live: false,
            commands: sample_commands(),
        });
        state.show_verify = true;
        insta::assert_snapshot!(render_to_string(&state));
    }

    #[test]
    fn renders_note_editor() {
        use crate::app::state::NoteEdit;

        let mut state = sample_state();
        let href = state.diff.files[0].hunks[0].href.clone();
        state.note_edit = Some(NoteEdit {
            href,
            buffer: "double-check the off-by-one".into(),
        });
        insta::assert_snapshot!(render_to_string(&state));
    }

    #[test]
    fn renders_search_prompt() {
        let mut state = sample_state();
        state.search_edit = Some("let x".into());
        insta::assert_snapshot!(render_to_string(&state));
    }

    #[test]
    fn renders_session_picker() {
        use crate::app::SessionListItem;

        let mut state = sample_state();
        state.sessions = vec![
            SessionListItem {
                id: "11111111-aaaa".into(),
                title: Some("Add greeting, fix off-by-one".into()),
                last_prompt: None,
                is_current: true,
            },
            SessionListItem {
                id: "22222222-bbbb".into(),
                title: None,
                last_prompt: Some("earlier exploratory task".into()),
                is_current: false,
            },
        ];
        state.show_picker = true;
        insta::assert_snapshot!(render_to_string(&state));
    }
}
