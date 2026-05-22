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
