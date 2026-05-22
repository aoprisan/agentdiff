# Phase 5 — Action layer (OUT OF CURRENT SCOPE)

> Explicitly excluded by the read-only decision. This file records *why* and how to add it later without re-architecting, so phases 0–4 stay compatible. Do not build unless the read-only constraint is lifted.

## Why deferred
You chose read-only: the tool's job is to help you *understand and triage* an agent run, not to mutate the tree. Acting on the tree (especially hunk-level reverse-apply) is the highest-correctness-risk component; keeping it out shrinks the surface and the bug budget. Verdicts already give a personal checklist without touching files.

## What it would add (when/if unlocked)
- **Hunk-level revert** of rejected hunks: generate a reverse unified patch from the model (we already own headers + `+/-`/context lines) and apply via the `git` CLI — `git apply --reverse --recount --unidiff-zero`, gated by a mandatory `git apply --check` pre-flight; whole-file via `git restore` (tracked) or delete-with-undo (created).
- **Stage/unstage hunks** (`git add`/`restore --staged`) to commit the approved subset from the TUI.
- **Export feedback to the agent**: write rejected hunks + correlated intent + notes as a structured prompt (Markdown for humans / JSON on stdout for agents), to paste/pipe back into Claude Code as a follow-up task (matches the `revdiff`/`difit` annotations-to-stdout convention).

## Why the current architecture stays compatible
- Verdicts/notes/flags are anchored to **content-addressed `HunkRef`s**, so "rejected" is already a first-class, addressable set.
- We **own the diff model** (not pre-rendered ANSI), so a unified patch can be synthesized for both revert and export from one code path (`git/patch.rs`, to be added).
- A `GitWriter` trait would isolate the only mutating code; everything else stays read-only and unit-testable against `tempfile` repos.

## If building it
1. `git/patch.rs`: `&[HunkRef] -> unified diff text` (re-number synthesized headers).
2. `git/writer.rs`: `GitWriter` trait + `CliGitWriter` (`git apply --reverse --check`); pause the watcher during apply; snapshot targets to a trash dir for one-keystroke undo; all-or-nothing per invocation; on conflict, re-diff and surface — never force.
3. `action/{revert,export}.rs`: gather rejected hunks → writer / structured prompt.
4. New keybindings + confirm dialogs; integration tests with adversarial hunks (adjacent, EOF-newline, CRLF, renames).

## Stretch beyond this
Structural/AST diff behind a `StructuralDiffer` trait (tree-sitter or shell `difftastic`); config-driven user-defined risk rules.
