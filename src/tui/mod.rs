//! ratatui rendering plus the terminal/event lifecycle. Reads `AppState`;
//! contains no business logic.

pub mod highlight;
pub mod layout;
pub mod theme;
pub mod widgets;

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anyhow::Context;
use crossbeam_channel::{RecvTimeoutError, unbounded};
use ratatui::Frame;
use ratatui::crossterm::event;
use ratatui::layout::Rect;

use crate::app::{AppEvent, AppState, View, update};
use crate::cli::Args;
use crate::config;
use crate::git::{Repo, diff_worktree_vs_head};
use highlight::Highlighter;

/// Build the diff/review state, then run the panic-safe terminal loop, always
/// restoring the terminal — including on panic — and persisting verdicts on exit.
pub fn run(args: Args, state_dir: PathBuf) -> anyhow::Result<()> {
    // Everything that can fail happens before we touch the terminal, so errors
    // print normally rather than from inside the alternate screen.
    let start = args.path.unwrap_or_else(|| PathBuf::from("."));
    let repo = Repo::discover(&start)
        .with_context(|| format!("opening a git repository at {}", start.display()))?;
    let diff = diff_worktree_vs_head(&repo).context("building the working-tree diff")?;
    tracing::info!(files = diff.files.len(), "built working-tree diff");

    let state_path = config::review_state_path(&state_dir, repo.workdir(), &diff.base);
    let review = config::load_review_state(&state_path);
    let mut state = AppState::new(diff, review, state_path);

    install_panic_hook();
    let mut terminal = ratatui::init();
    let mut highlighter = Highlighter::new();
    let result = event_loop(&mut terminal, &mut state, &mut highlighter);
    ratatui::restore();

    // Persist after restoring so any write error surfaces on the real terminal.
    if state.review_dirty
        && let Err(e) = config::save_review_state(&state.state_path, &state.review)
    {
        tracing::warn!(error = %e, "failed to save review state");
    }
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    state: &mut AppState,
    highlighter: &mut Highlighter,
) -> anyhow::Result<()> {
    let (tx, rx) = unbounded::<AppEvent>();

    // Blocking input reads live on their own thread. When the loop drops `rx`,
    // the next `send` fails and this thread exits.
    thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if tx.send(AppEvent::Input(ev)).is_err() {
                break;
            }
        }
    });

    while !state.should_quit {
        // Refresh the diff-pane height so paging/scrolling matches what's drawn.
        if let Ok(size) = terminal.size() {
            let panes = layout::compute(Rect::new(0, 0, size.width, size.height));
            state.viewport_height = (panes.diff.height.saturating_sub(2)).max(1) as usize;
            state.ensure_cursor_visible();
        }
        terminal.draw(|frame| render(frame, state, highlighter))?;
        match rx.recv_timeout(Duration::from_millis(33)) {
            Ok(ev) => update(state, ev),
            Err(RecvTimeoutError::Timeout) => update(state, AppEvent::Tick),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn render(frame: &mut Frame, state: &AppState, highlighter: &mut Highlighter) {
    match state.view {
        View::Review => render_review(frame, state, highlighter),
    }
    if state.show_help {
        widgets::help::render(frame, frame.area());
    }
}

fn render_review(frame: &mut Frame, state: &AppState, highlighter: &mut Highlighter) {
    let panes = layout::compute(frame.area());
    widgets::file_tree::render(frame, panes.file_tree, state);
    widgets::diff_pane::render(frame, panes.diff, state, highlighter);
    widgets::intent_panel::render(frame, panes.intent);
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
}
