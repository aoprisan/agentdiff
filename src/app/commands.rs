/// A resolved user intent, decoupled from the key that produced it. The keymap
/// translates input into `Command`s; the reducer applies them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Quit,
    ToggleHelp,
    CloseOverlay,
    Noop,
}
