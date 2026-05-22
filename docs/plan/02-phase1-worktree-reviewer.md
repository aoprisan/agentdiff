# Phase 1 — Working-tree reviewer

> Prereq: Phase 0. Goal: a genuinely useful "review what changed in my working tree" viewer — virtualized, syntax-highlighted, vim-navigable, with a persisted read-only triage checklist. This is the spine; Phase 2 swaps the diff *base* but reuses everything here.

## In scope
- `DiffBase::WorkingTreeVsHead` built with `git2`, **including untracked files** (agents create many).
- Real `HunkRef` fingerprinting; flattened row index + jump tables.
- Diff pane: virtualized rendering, lazy `syntect` highlighting, intra-line word diff via `similar`.
- File-tree pane: changed files with status badge + `(+adds/-dels)`; collapse/expand.
- Read-only verdicts: `a` = Approved, `x` = NeedsAttention, anchored to `HunkRef`; status bar shows reviewed/total.
- Persist `ReviewState` (TOML, keyed by repo path + base) under the state dir; reload on launch and re-anchor by fingerprint.

## Out of scope (deferred)
Claude session/intent (Phase 2), live re-diff (Phase 3), risk (Phase 4), any tree mutation (Phase 5 / never per read-only decision), arbitrary ranges (Phase 3), notes UI (Phase 3 — store field exists, no editor yet).

## Crates to add
`git2`, `similar`, `syntect`, `ignore`, `globset`, `toml`, `serde` (+ derive). Dev: `tempfile`.

## Tasks (ordered)
1. `git/repo.rs`: open repo from `[PATH]`/cwd (walk up for `.git`); resolve HEAD; `status` incl. untracked using `ignore` for enumeration.
2. `git/differ.rs`: produce `domain::Diff` for `WorkingTreeVsHead` — map git2 deltas/hunks/lines into `FileChange`/`Hunk`/`Line`; detect binary, renames (`old_path`), language (by extension).
3. `git/untracked.rs`: for created files, synth an "empty → content" diff via `similar` into the same `Hunk`/`Line` model; set `is_created`.
4. `domain/ids.rs`: real `fingerprint` (hash of normalized hunk old/new line content + path) — stable across re-diff, insensitive to surrounding line-number shifts.
5. Diff flattening: `Vec<RowRef>` (file-header | hunk-header | line) + next/prev hunk + next/prev file jump tables, rebuilt on (re-)diff.
6. `tui/highlight.rs`: syntect set loaded once (`once_cell`); highlight only visible rows + overscan; LRU cache keyed by (FileId, range, theme).
7. `tui/widgets/diff_pane.rs`: render the visible window only; add/del/context styling + word-diff spans from `Line.intra`; collapse created/binary/huge files by default with a summary row.
8. `tui/widgets/file_tree.rs`: list with badges + per-file stats; selection drives the diff pane.
9. Navigation in `app/keymap.rs` + `update.rs`: `j/k`, `C-d/C-u`, `]c`/`[c` (hunk), `}`/`{` or `Tab`+`n/p` (file), `gg/G`, `Space` (collapse), `a`/`x` (verdict), `u` (unset).
10. `domain/review.rs` + persistence: load/save `ReviewState` (TOML); re-anchor verdicts by fingerprint on load and flag "changed since reviewed" when a fingerprint is gone.

## Files created/modified
`src/git/{repo,differ,untracked}.rs`, `src/domain/ids.rs` (real), `src/domain/review.rs` (persistence), `src/tui/highlight.rs`, `src/tui/widgets/{diff_pane,file_tree,statusbar}.rs`, `src/app/{update,commands,keymap}.rs`.

## Acceptance
- In a repo with mixed changes (modified, added/untracked, deleted, renamed, a binary, a CRLF file, a no-trailing-newline file), `agentdiff` shows all of them; file set matches `git status` + untracked.
- Word diff highlights the changed substrings within modified lines; syntax colors match file language.
- A 10k+-line generated file is collapsed by default; expanding + scrolling stays responsive (no full-file highlight).
- `a`/`x` set verdicts; quit and relaunch → verdicts persist and re-attach to the same hunks; an edited hunk shows "changed since reviewed."
- `insta` snapshots: `Diff` model for the temp-repo fixture; rendered diff-pane buffer via `TestBackend`.

## Definition of done
A fast, correct, persistent working-tree reviewer usable daily even before any Claude-specific feature exists. Commit.
