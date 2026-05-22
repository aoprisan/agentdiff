/// A resolved user intent, decoupled from the key that produced it. The keymap
/// translates input into `Command`s; the reducer applies them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Quit,
    ToggleHelp,
    CloseOverlay,

    // Navigation.
    CursorDown,
    CursorUp,
    HalfPageDown,
    HalfPageUp,
    NextHunk,
    PrevHunk,
    NextFile,
    PrevFile,
    GotoTop,
    GotoBottom,

    // Review.
    ToggleCollapse,
    Approve,
    NeedsAttention,
    Unset,

    Noop,
}
