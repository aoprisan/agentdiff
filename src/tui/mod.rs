//! ratatui rendering plus the terminal/event lifecycle. Reads `AppState`;
//! contains no business logic.

pub mod layout;
pub mod widgets;

use std::thread;
use std::time::Duration;

use crossbeam_channel::{RecvTimeoutError, unbounded};
use ratatui::Frame;
use ratatui::crossterm::event;

use crate::app::{AppEvent, AppState, View, update};
use crate::cli::Args;

/// Set up the terminal, run the event loop, and always restore the terminal —
/// including on panic.
pub fn run(_args: Args) -> anyhow::Result<()> {
    install_panic_hook();
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal) -> anyhow::Result<()> {
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

    let mut state = AppState::new();
    while !state.should_quit {
        terminal.draw(|frame| render(frame, &state))?;
        match rx.recv_timeout(Duration::from_millis(33)) {
            Ok(ev) => update(&mut state, ev),
            Err(RecvTimeoutError::Timeout) => update(&mut state, AppEvent::Tick),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn render(frame: &mut Frame, state: &AppState) {
    match state.view {
        View::Review => render_review(frame),
    }
    if state.show_help {
        widgets::help::render(frame, frame.area());
    }
}

fn render_review(frame: &mut Frame) {
    let panes = layout::compute(frame.area());
    widgets::file_tree::render(frame, panes.file_tree);
    widgets::diff_pane::render(frame, panes.diff);
    widgets::intent_panel::render(frame, panes.intent);
    widgets::statusbar::render(frame, panes.status);
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
    use crate::app::AppState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_to_string(state: &AppState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal.draw(|frame| render(frame, state)).unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn renders_review_layout() {
        let state = AppState::new();
        insta::assert_snapshot!(render_to_string(&state));
    }

    #[test]
    fn renders_help_overlay() {
        let mut state = AppState::new();
        state.show_help = true;
        insta::assert_snapshot!(render_to_string(&state));
    }
}
