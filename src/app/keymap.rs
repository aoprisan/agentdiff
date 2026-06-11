use std::collections::HashMap;

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

/// Vim-ish keymap with optional per-command single-key overrides from config.
/// Overrides are additive: a remapped key triggers its command in addition to
/// the built-in default. Ctrl-chords, two-key sequences (`gg`, `]c`, `[c`), and
/// special keys (Enter/Esc/arrows) are fixed.
#[derive(Debug, Clone, Default)]
pub struct Keymap {
    overrides: HashMap<char, Command>,
}

impl Keymap {
    /// Build from a config `command-name → key` table, ignoring unknown command
    /// names or multi-character key strings.
    pub fn from_overrides(table: &HashMap<String, String>) -> Self {
        let mut overrides = HashMap::new();
        for (name, key) in table {
            if let (Some(cmd), Some(ch)) = (command_from_name(name), single_char(key)) {
                overrides.insert(ch, cmd);
            }
        }
        Keymap { overrides }
    }

    pub fn resolve(&self, key: KeyEvent, pending: Option<char>) -> Resolved {
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

        // Config overrides apply to plain (non-ctrl) character keys.
        if !ctrl
            && let KeyCode::Char(c) = key.code
            && let Some(&cmd) = self.overrides.get(&c)
        {
            return Resolved::Command(cmd);
        }

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
            KeyCode::Tab => Command::NextUnreviewed,
            KeyCode::BackTab => Command::PrevUnreviewed,
            KeyCode::Char('G') => Command::GotoBottom,

            KeyCode::Char(' ') => Command::ToggleCollapse,
            KeyCode::Char('a') => Command::Approve,
            KeyCode::Char('x') => Command::NeedsAttention,
            KeyCode::Char('u') => Command::Unset,

            KeyCode::Char('s') => Command::OpenSessionPicker,
            KeyCode::Char('i') => Command::ToggleIntentDetail,
            KeyCode::Char('v') => Command::ToggleVerification,
            KeyCode::Char('n') => Command::EditNote,
            KeyCode::Char('/') => Command::OpenSearch,
            KeyCode::Char('m') => Command::NextMatch,
            KeyCode::Char('M') => Command::PrevMatch,
            KeyCode::Enter => Command::Select,

            _ => Command::Noop,
        };
        Resolved::Command(cmd)
    }
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

/// Map a config command name to a `Command` (overridable single-key actions only).
fn command_from_name(name: &str) -> Option<Command> {
    Some(match name {
        "quit" => Command::Quit,
        "help" => Command::ToggleHelp,
        "cursor_down" => Command::CursorDown,
        "cursor_up" => Command::CursorUp,
        "next_file" => Command::NextFile,
        "prev_file" => Command::PrevFile,
        "goto_bottom" => Command::GotoBottom,
        "toggle_collapse" => Command::ToggleCollapse,
        "approve" => Command::Approve,
        "needs_attention" => Command::NeedsAttention,
        "unset" => Command::Unset,
        "session_picker" => Command::OpenSessionPicker,
        "intent_detail" => Command::ToggleIntentDetail,
        "verification" => Command::ToggleVerification,
        "edit_note" => Command::EditNote,
        "search" => Command::OpenSearch,
        "next_match" => Command::NextMatch,
        "prev_match" => Command::PrevMatch,
        _ => return None,
    })
}

fn single_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn keymap() -> Keymap {
        Keymap::default()
    }

    #[test]
    fn gg_jumps_to_top() {
        assert_eq!(keymap().resolve(press('g'), None), Resolved::Pending('g'));
        assert_eq!(
            keymap().resolve(press('g'), Some('g')),
            Resolved::Command(Command::GotoTop)
        );
    }

    #[test]
    fn bracket_c_navigates_hunks() {
        assert_eq!(keymap().resolve(press(']'), None), Resolved::Pending(']'));
        assert_eq!(
            keymap().resolve(press('c'), Some(']')),
            Resolved::Command(Command::NextHunk)
        );
        assert_eq!(
            keymap().resolve(press('c'), Some('[')),
            Resolved::Command(Command::PrevHunk)
        );
    }

    #[test]
    fn incomplete_sequence_falls_back_to_single_key() {
        assert_eq!(
            keymap().resolve(press('j'), Some('g')),
            Resolved::Command(Command::CursorDown)
        );
    }

    #[test]
    fn ctrl_d_is_half_page() {
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(
            keymap().resolve(ctrl_d, None),
            Resolved::Command(Command::HalfPageDown)
        );
    }

    #[test]
    fn config_override_rebinds_a_key() {
        let mut table = HashMap::new();
        table.insert("approve".to_string(), "v".to_string());
        let km = Keymap::from_overrides(&table);
        assert_eq!(
            km.resolve(press('v'), None),
            Resolved::Command(Command::Approve)
        );
        // Defaults still resolve.
        assert_eq!(km.resolve(press('q'), None), Resolved::Command(Command::Quit));
    }
}
