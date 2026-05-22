use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::commands::Command;

/// Outcome of feeding one key to the keymap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    /// A complete command to apply.
    Command(Command),
    /// The first key of a two-key sequence; hold it as the pending leader.
    Pending(char),
}

/// Vim-ish keymap. `pending` carries the leader of a two-key sequence (`g`, `]`,
/// `[`). When a leader doesn't complete a known sequence, the key is re-resolved
/// as a fresh single keypress so a stray leader never swallows the next command.
pub fn resolve(key: KeyEvent, pending: Option<char>) -> Resolved {
    // Ignore key-release / repeat events (Windows reports both).
    if key.kind != KeyEventKind::Press {
        return Resolved::Command(Command::Noop);
    }

    if let Some(leader) = pending
        && let Some(cmd) = complete(leader, key)
    {
        return Resolved::Command(cmd);
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let cmd = match key.code {
        KeyCode::Char('g') => return Resolved::Pending('g'),
        KeyCode::Char(']') => return Resolved::Pending(']'),
        KeyCode::Char('[') => return Resolved::Pending('['),

        KeyCode::Char('q') => Command::Quit,
        KeyCode::Char('?') => Command::ToggleHelp,
        KeyCode::Esc => Command::CloseOverlay,

        KeyCode::Char('j') | KeyCode::Down => Command::CursorDown,
        KeyCode::Char('k') | KeyCode::Up => Command::CursorUp,
        KeyCode::Char('d') if ctrl => Command::HalfPageDown,
        KeyCode::Char('u') if ctrl => Command::HalfPageUp,
        KeyCode::Char('}') => Command::NextFile,
        KeyCode::Char('{') => Command::PrevFile,
        KeyCode::Char('G') => Command::GotoBottom,

        KeyCode::Char(' ') => Command::ToggleCollapse,
        KeyCode::Char('a') => Command::Approve,
        KeyCode::Char('x') => Command::NeedsAttention,
        KeyCode::Char('u') => Command::Unset,

        _ => Command::Noop,
    };
    Resolved::Command(cmd)
}

/// Try to complete a two-key sequence from its leader.
fn complete(leader: char, key: KeyEvent) -> Option<Command> {
    match (leader, key.code) {
        ('g', KeyCode::Char('g')) => Some(Command::GotoTop),
        (']', KeyCode::Char('c')) => Some(Command::NextHunk),
        ('[', KeyCode::Char('c')) => Some(Command::PrevHunk),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn gg_jumps_to_top() {
        assert_eq!(resolve(press('g'), None), Resolved::Pending('g'));
        assert_eq!(
            resolve(press('g'), Some('g')),
            Resolved::Command(Command::GotoTop)
        );
    }

    #[test]
    fn bracket_c_navigates_hunks() {
        assert_eq!(resolve(press(']'), None), Resolved::Pending(']'));
        assert_eq!(
            resolve(press('c'), Some(']')),
            Resolved::Command(Command::NextHunk)
        );
        assert_eq!(
            resolve(press('c'), Some('[')),
            Resolved::Command(Command::PrevHunk)
        );
    }

    #[test]
    fn incomplete_sequence_falls_back_to_single_key() {
        // `g` then `j` should move down, not be swallowed by the dead leader.
        assert_eq!(
            resolve(press('j'), Some('g')),
            Resolved::Command(Command::CursorDown)
        );
    }

    #[test]
    fn ctrl_d_is_half_page() {
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(
            resolve(ctrl_d, None),
            Resolved::Command(Command::HalfPageDown)
        );
    }
}
