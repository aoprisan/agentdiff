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
    /// Jump to the next hunk without a verdict, wrapping past the end — the
    /// "what's left to review" motion.
    NextUnreviewed,
    PrevUnreviewed,
    NextFile,
    PrevFile,
    GotoTop,
    GotoBottom,

    // Review.
    ToggleCollapse,
    Approve,
    NeedsAttention,
    Unset,

    // Session (Phase 2).
    OpenSessionPicker,
    ToggleIntentDetail,
    Select,

    // Verification (Phase 6): commands the agent ran to check its work.
    ToggleVerification,

    // Notes (Phase 3).
    EditNote,

    // Search.
    OpenSearch,
    NextMatch,
    PrevMatch,

    Noop,
}
