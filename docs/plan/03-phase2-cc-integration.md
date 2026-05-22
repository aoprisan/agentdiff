# Phase 2 — Claude Code integration & intent correlation (the headline)

> Prereq: Phase 1. Goal: make `agentdiff` default to "exactly what the latest agent run changed" and show the agent's stated intent beside each file/hunk. This completes the MVP. Reuses the entire Phase 1 diff/render/review pipeline — only the diff *base* and an intent overlay are new.

## In scope
- `session/locate.rs`: cwd → slug (`/` and `.` → `-`); list this project's sessions newest-first; pick the latest session and its latest `auto` run.
- `session/transcript.rs`: streaming line-by-line JSONL parser → `Vec<Record>` using `#[serde(tag="type")]` + `#[serde(other)]` catch-all; tolerate a partially-written trailing line.
- `session/runs.rs`: segment records by `permission-mode` (`auto` spans = autonomous runs); fold the **cumulative** `file-history-snapshot` records into each `AgentRun`'s pre-run `snapshot` map (use the latest snapshot within the span).
- `session/backups.rs`: resolve `trackedFileBackups[path].backupFileName` → `~/.claude/file-history/<sid>/<backupFileName>` (verbatim pre-edit content); `backupFileName: null` ⇒ agent-created. Normalize path keys to absolute, re-relativize to repo root, **drop out-of-repo entries**.
- `git/differ.rs`: support `DiffBase::AgentRun` — diff each backup's pre-run content vs the current working-tree file (via `similar`, same `Hunk`/`Line` model); created files render as full additions.
- Default base = latest `auto` run; **fallback to `WorkingTreeVsHead`** (Phase 1) when no session/run/backups are found. `--no-session` forces git-only; `--session <id>` / `--run <n>` select explicitly.
- `session/intent.rs`: for each `Edit`/`Write`/`MultiEdit` `tool_use`, walk `parentUuid` up the message DAG to the nearest preceding `assistant` text turn; attach as `Intent{file_path,text,source_uuid,confidence}`. Degrade to "no intent found" gracefully.
- `tui/widgets/intent_panel.rs`: right pane shows the intent for the current file/hunk, with the session's `last-prompt`/title as a header and a confidence indicator.
- `session/locate.rs` ranking surfaced in a basic `SessionPicker` (open with a key) listing sessions newest-first.

## Out of scope (deferred)
Live re-diff while the agent runs (Phase 3 — this phase reads the transcript once at launch), richer intent ranking/scoring (Phase 3), risk (Phase 4).

## Crates to add
`serde_json`, `jiff`. (`serde` already present.)

## Tasks (ordered)
1. Build a `tests/fixtures/` corpus: copy a real session JSONL + its `~/.claude/file-history/<sid>/` dir (sanitized) into the repo for deterministic tests.
2. `transcript.rs`: the tagged-enum parser + `Other` fallback; unit-test it round-trips the fixture without error and preserves unknown line types.
3. `runs.rs`: permission-mode segmentation + snapshot folding; test that the fixture yields the expected run count and pre-run file set.
4. `backups.rs`: path normalization + backup-file resolution; test absolute-vs-relative keys and `null` (created) handling; verify resolved backup files exist on disk in the fixture.
5. `differ.rs` `AgentRun` base: pre-run-backup vs working-tree diffs; created files as additions.
6. Wire default base selection + fallback in `main`/`app`; honor `--session`/`--run`/`--no-session`.
7. `intent.rs`: parentUuid DAG walk; test the known fixture edit resolves to its 1-hop intent text.
8. `intent_panel.rs` + `session_picker.rs`; keymap entries to open the picker and to toggle intent detail.

## Acceptance
- Run `agentdiff` in a repo right after a real CC auto run → it auto-selects that run, the diff matches the run's `trackedFileBackups` set (intersected with the repo), and each changed file shows the agent's stated intent.
- In a repo with no CC session → silent, correct fallback to working-tree-vs-HEAD (Phase 1 behavior).
- `--session <id> --run <n>` selects a specific run; `SessionPicker` lists sessions newest-first.
- Out-of-repo backup entries (e.g. `~/.claude/plans/*`, `CLAUDE.md`) do not appear in the diff.
- Parser does not crash on an unknown/new line type (regression-guarded by the fixture test).
- `insta` snapshots: parsed run structure; intent map for the fixture; rendered intent panel.

## Definition of done
The MVP: open the tool after an agent run and immediately see *what this run changed* and *why the agent said it did it*, with graceful git-only fallback. Commit.
