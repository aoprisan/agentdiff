# Phase 6 — agent verification surfacing ("did it actually work?")

> Prereq: Phase 2 (works on any loaded session/run). Goal: alongside *what the
> agent said it was doing* (intent), show *what the agent actually ran to check
> its work* — the test/build/lint commands in the run and whether they passed.
> Read-only; the `Diff` spine is unchanged. This is the natural twin of intent:
> intent is the claim, verification is the evidence.

## Why
In auto mode the agent often runs `cargo test` / `clippy` / a build between edits.
That signal already sits in the transcript next to the edits we parse, but we
throw it away. Surfacing it answers the first question a reviewer actually has —
"did it work, and did the agent even check?" — and flags the dangerous case: a
run that edited code but never ran the tests, or ended on a failure.

## Verified on-disk format (checked against real transcripts)
- A shell command is a `tool_use` block with `name:"Bash"`, `id`, and
  `input:{command, description}`.
- Its result arrives in a **following `user` entry** as a `tool_result` content
  block `{tool_use_id, content, is_error}` (linkable by `id`). CC also writes a
  richer top-level `toolUseResult:{stdout,stderr,interrupted,…}` — we ignore it
  for now and read the standard block (more stable, tolerant of drift).
- **`is_error` is unreliable**: in real data it is mostly `null`, occasionally
  `false`, rarely `true` (it fires for tool-level errors / rejections, not for a
  command that merely exited non-zero). A genuine command failure instead shows
  up as an `Exit code N` line **inside the result content**. Outcome detection
  must read the content, not just the flag.

## In scope
- `domain/session.rs`: `CommandRun{command, description, kind, outcome,
  output_excerpt, message_uuid, timestamp}`, `CommandKind{Test, Build, Lint,
  Format, Vcs, Run, Other}`, `CommandOutcome{Ok, Failed, Unknown}`; add
  `commands: Vec<CommandRun>` to `AgentRun` (run-scoped, parallel to `edits`).
- `session/transcript.rs`: add `id` to the `ToolUse` block; add a `ToolResult`
  block variant with tolerant content extraction (string or block array).
- `session/commands.rs` (new, pure + unit-tested): `classify(cmd) -> CommandKind`
  (substring heuristic, advisory), `outcome(is_error, output) -> CommandOutcome`
  (non-zero `Exit code`, `is_error`, and a small high-precision failure-signal
  set), `excerpt(output)` (trimmed tail for the detail view).
- `session/runs.rs`: within an autonomous span, collect `Bash` commands and link
  each to its `tool_result` by id; a command whose result was never seen (live
  run mid-command) stays `Unknown`.
- UI: a compact verification line in the Intent panel header
  (`✓ test · ✓ build · ✗ lint`, latest of each verification kind, colored), and a
  `v` overlay (`tui/widgets/verification.rs`) listing every command in the run
  with its outcome and a short output tail. Help + README + keymap updated.

## Out of scope (later)
- Correlating a specific command to specific hunks (run-level only here).
- A risk rule ("edited code, never ran tests" / "run ended on a failure") — lands
  naturally in Phase 4 once it exists, reading `AgentRun.commands`.
- Parsing structured `toolUseResult` stdout/stderr separation.

## Crates to add
None.

## Tasks (ordered)
1. Domain types + `AgentRun.commands`.
2. Transcript: `ToolUse.id`, `ToolResult` variant + accessors.
3. `session/commands.rs`: `classify` / `outcome` / `excerpt`, unit-tested.
4. `runs.rs`: in-span Bash capture + result linking → `RawRun.commands`; map to
   `AgentRun` in `session/mod.rs`.
5. `SessionSummary.commands` (selected run) in `app/bootstrap.rs`.
6. `verification.rs` widget + header line in `intent_panel.rs`; `show_verify`
   state, `ToggleVerification` command bound to `v`, reducer + CloseOverlay.
7. Extend the session fixture with a Bash run + result; update snapshots.
8. README (keys table, why, config command name) + this doc.

## Acceptance
- A run that ran `cargo test` (passed), `cargo build` (passed), and `cargo clippy`
  (failed, `Exit code 1`) yields three `CommandRun`s with kinds Test/Build/Lint
  and outcomes Ok/Ok/Failed.
- A `Bash` command whose result line is absent (live run) stays `Unknown`.
- Header shows the colored compact line; `v` opens the overlay; both stay correct
  across a live re-diff (recomputed from the re-parsed transcript).
- The tool stays fully usable with no commands / no session (advisory).
- `insta`: `commands.rs` classify/outcome, `runs.rs` capture, fixture run-structure
  snapshot, rendered verification overlay + intent header.

## Definition of done
Reviewers see, per run, what the agent ran to verify itself and whether it
passed — next to what it said it was doing. Commit. Build once Phase 2 is solid.
