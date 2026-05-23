# Phase 3 — Live re-diff & polish

> Prereq: Phase 2 (MVP complete). Goal: make the tool pleasant during an in-progress agent run and configurable. Pure additive layer.

## In scope
- `watch/mod.rs`: `notify` + `notify-debouncer-full` watchers on (a) the working tree and (b) the active session JSONL; debounce bursty agent writes (~150–300ms) → `AppEvent::FsChanged`.
- Live re-diff on the worker thread; swap `AppState.diff` on `GitDone`, re-anchoring verdicts/intent by `HunkRef` fingerprint and **preserving the cursor on the same logical hunk**. Generation counter drops superseded re-diffs.
- Detect a still-running (`ended: None`) run and show a "live" indicator; re-read the transcript tail to pick up new edits/intent.
- Notes UI: edit the per-hunk `notes` field (stored since Phase 1) in a small input; persisted with `ReviewState`.
- Config: `config.rs` loads `~/.config/agentdiff/config.toml` for keymap overrides + theme (syntax theme, add/del/intent colors).
- Arbitrary ranges: `--range A..B` (`DiffBase::Range`) and `--staged` (`WorkingTreeVsIndex`).
- Help overlay fully populated; optional mouse scroll/click selection.

## Out of scope (deferred)
Risk engine (Phase 4), action layer (Phase 5).

## Crates to add
`notify`, `notify-debouncer-full`.

## Tasks (ordered)
1. `watch/mod.rs`: watcher threads + debouncer → channel events; scope the tree watch with `ignore` rules; watch the active JSONL path.
2. `app/update.rs`: handle `FsChanged` → enqueue re-diff (+ transcript re-read if JSONL changed); apply generation-counter cancellation.
3. Re-anchor logic: after a re-diff, remap verdicts/notes/intent by fingerprint; keep the selected hunk if it still exists, else nearest.
4. Live-run indicator in the status bar; transcript tail re-parse for new edits/intent.
5. Notes editor widget + keybinding; persist with `ReviewState`.
6. `config.rs`: TOML keymap/theme load with sensible defaults; thread through `tui/theme.rs` and `app/keymap.rs`.
7. `--range`/`--staged` plumbed through `git/differ.rs` and base selection.

## Acceptance
- Start `agentdiff` while a CC auto run is in progress → as the agent edits, the diff and intent update within a moment without losing your scroll position or verdicts.
- Editing a file under review re-diffs and keeps verdicts attached (or flags "changed since reviewed").
- `--range main..HEAD` and `--staged` produce correct diffs reusing the same UI.
- Custom keymap/theme from `config.toml` takes effect.
- Notes persist across relaunch.

## Definition of done
The tool is comfortable to leave open during an agent run and adapts to each user's keys/colors. Commit.

## Implementation notes

- **Worker + watch concurrency.** `watch/mod.rs` runs a `notify-debouncer-full` watcher (250ms) on the tree + active transcript, filtering through the repo gitignore and always skipping `.git`, emitting `AppEvent::FsChanged`. A worker thread opens its **own** libgit2 handle (git2's `Repository` is `!Send`) and rebuilds the diff bundle (`app::build_bundle`) on demand; each result carries a generation, and `AppState.generation` drops superseded ones. The main thread only renders + applies results.
- **Re-anchoring is automatic.** Verdicts/notes are keyed by `HunkRef` fingerprint, so they re-attach across a re-diff with no remapping; a vanished fingerprint surfaces as "changed since reviewed". The cursor re-anchors to the same hunk by fingerprint (`AppState::apply_rediff`), and the highlight LRU is cleared since `(file,hunk,line)` indices shift.
- **Live indicator.** A run still open at end-of-transcript (no closing non-autonomous turn) has `ended: None` and shows `● live` in the status bar + intent header. (Not observable in this environment, which records no autonomous runs — validated by unit test.)
- **Config.** `~/.config/agentdiff/config.toml` → `[theme] syntax/added/removed/intent` (syntect theme name + `#rrggbb` overrides) and `[keys] command = "char"` (additive single-key rebindings). Threaded through `Highlighter::with_theme`, a global `theme::Overrides`, and a `Keymap` reapplied across session switches.
- **Ranges.** `--range A..B` (`diff_range`, tree-to-tree) and `--staged` (`diff_worktree_vs_index`, HEAD-to-index) reuse the same render/review UI; missing `--range` side defaults to `HEAD`.
- **Deferred:** mouse support (marked optional in scope).
- The live watch/worker path can't be exercised at runtime in this sandbox (no TTY); its pieces — gitignore filtering, cursor/verdict re-anchoring, range/staged diffs, live detection, config parsing/keymap override — are covered by unit tests.
