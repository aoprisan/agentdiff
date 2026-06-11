# agentdiff

A terminal git-diff reviewer built for one workflow: you run Claude Code in
auto / accept-edits mode, then review the changes it made. `agentdiff` scopes the
diff to a single agent **run**, shows you **what the agent said it was doing** next
to each change (pulled from Claude Code's own session data), and lets you keep a
personal approve / needs-attention checklist over a large diff.

It is **read-only** — it never touches your working tree. Verdicts and notes are a
triage aid, not a way to revert or stage.

## Why

In auto mode the agent edits many files without you watching, so the post-run
review is your only control point. Plain `git diff` treats those edits like any
other commit: flat, intent-free, and not scoped to "what this one run touched."
`agentdiff` is built for exactly that review step:

- **Per-run scoping.** By default it diffs the latest autonomous run against that
  run's *pre-run* file content (Claude Code's file-history backups), so you see
  exactly what the run changed — even across intermediate commits. Falls back to
  working-tree-vs-`HEAD` when no session is found.
- **Agent-intent correlation.** For each changed file it surfaces the agent's
  own reasoning, recovered by walking the transcript back to the assistant turn
  that drove the edit.
- **Verification surfacing — "did it actually work?"** Alongside what the agent
  *said*, it shows what the agent *ran to check itself*: the test/build/lint
  commands in the run and whether they passed (a compact `✓ test · ✗ lint` badge,
  with `v` opening the full command list and output). Outcomes are read from the
  command output, so a failing `cargo test` is flagged even when Claude Code's
  own error flag isn't set.
- **Read-only triage.** Mark hunks approved or needs-attention and attach notes;
  state persists across runs and re-attaches to changes by content fingerprint,
  not line number, so it survives a live re-diff.
- **Close the loop.** `agentdiff --report` prints the review as markdown —
  flagged hunks with their diffs, your notes, the correlated intent, and the
  verification outcomes — ready to pipe back to the agent as a follow-up task.
- **Fast, large-diff rendering.** Virtualized diff pane and lazy, viewport-only
  syntax highlighting — it stays responsive on generated files.

## Status

Phases 0–3 are complete: git working-tree reviewer (including untracked files),
Claude Code per-run scoping + intent correlation, live re-diff while the agent is
running, a session picker, and TOML config. Phase 6 adds verification surfacing
(the commands the agent ran to check its work). An optional risk-scoring engine
(phase 4) and any tree-mutating actions (phase 5, intentionally out of scope) are
not built. See [`docs/plan/`](docs/plan/) for the full roadmap.

## Install

Requires a recent Rust toolchain (the crate uses edition 2024).

```bash
git clone https://github.com/aoprisan/agentdiff
cd agentdiff
cargo build --release
```

To put it on your `PATH`, run the built binary with `--install`. It copies the
binary into the first conventional user bin dir already on your `PATH`
(`~/.local/bin`, `~/bin`, or `~/.cargo/bin`), falling back to `~/.local/bin`:

```bash
./target/release/agentdiff --install
```

(It copies rather than symlinks, so the installed tool keeps working after a
`cargo clean` or if you move the source tree.)

## Usage

Run it inside a git repository — typically right after a Claude Code auto run:

```bash
agentdiff                  # review the latest agent run in the current repo
agentdiff /path/to/repo    # review a specific repo
```

It needs a real terminal (TTY). `agentdiff --help` prints the full CLI surface.

### Flags

| Flag | What it does |
|---|---|
| `[PATH]` | Repository to review (defaults to the current directory). |
| `--session <id>` | Review a specific Claude Code session instead of the latest. |
| `--run <n>` | Review a specific agent run within the session. |
| `--range <A..B>` | Diff an arbitrary git range. |
| `--staged` | Diff staged changes (index vs `HEAD`) instead of the working tree. |
| `--no-session` | Ignore Claude Code session data; diff working tree vs `HEAD`. |
| `--report` | Print the review as markdown to stdout and exit (no TUI). |
| `--install` | Copy this binary onto your `PATH` and exit. |

## Keys

| Key | Action |
|---|---|
| `j` / `k`, `↓` / `↑` | Line down / up |
| `Ctrl-d` / `Ctrl-u` | Half page down / up |
| `]c` / `[c` | Next / previous hunk |
| `Tab` / `Shift-Tab` | Next / previous **unreviewed** hunk (wraps) |
| `}` / `{` | Next / previous file |
| `gg` / `G` | Top / bottom |
| `Space` | Collapse / expand file |
| `/` | Search (paths, hunk headers, line text) |
| `m` / `M` | Next / previous search match (wraps) |
| `a` | Approve hunk |
| `x` | Flag hunk (needs attention) |
| `u` | Clear verdict |
| `n` | Add / edit note |
| `s` | Session picker |
| `i` | Toggle intent detail |
| `v` | Verification overlay (commands the agent ran + outcomes) |
| `?` | Toggle help |
| `Esc` | Close overlay |
| `q` | Quit |

## Configuration

Optional config lives at `~/.config/agentdiff/config.toml` (path resolved per
platform). Everything is optional; a missing or invalid file falls back to
built-in defaults.

```toml
[theme]
name   = "solarized-dark"      # "default", "solarized-dark", or "solarized-light"
syntax = "base16-ocean.dark"   # a syntect theme; defaults to the palette's pairing
added   = "#859900"            # #rrggbb overrides for add / remove / intent foregrounds
removed = "#dc322f"
intent  = "#268bd2"

# Rebind any single-key action. Overrides are additive — the default key still works.
[keys]
approve = "v"
```

Overridable command names: `quit`, `help`, `cursor_down`, `cursor_up`,
`next_file`, `prev_file`, `goto_bottom`, `toggle_collapse`, `approve`,
`needs_attention`, `unset`, `session_picker`, `intent_detail`, `verification`,
`edit_note`, `search`, `next_match`, `prev_match`. (Ctrl-chords, two-key
sequences, and special keys are fixed.)

## How it works

The **`Diff` is the spine**: git and session producers build a `domain::Diff`,
and risk, intent, and review state all anchor onto a content-addressed `HunkRef`
inside each hunk — a content fingerprint, not a line number — so they re-attach
across a live re-diff even as the tree changes underneath. Changing the "before"
source (the `DiffBase`) only changes how a `Diff` is built; everything downstream
is identical.

Claude Code session data is read from the standard on-disk locations
(`~/.claude/projects/<slug>/<session>.jsonl` transcripts and
`~/.claude/file-history/<session>/` pre-edit backups). All of it is treated as
**advisory** — git is always the source of truth, and the tool stays fully useful
even when no session can be parsed.

There is no async runtime: an input thread, a filesystem-watch thread, and a
worker thread communicate over `crossbeam-channel`, and a generation counter
drops results superseded by a newer re-diff. The main thread owns `AppState` and
renders.

```
src/
  domain/    pure types + transforms, no I/O — the contract everything anchors to
  git/       libgit2 reads: status, blobs, base diffs, synthetic untracked diffs
  session/   Claude Code transcript + file-history parsing, run segmentation, intent
  watch/     notify-based live re-diff of the tree and active transcript
  app/       UI-agnostic core: AppState + the (state, event) reducer
  tui/       ratatui rendering only — reads AppState, no business logic
```

See [`CLAUDE.md`](CLAUDE.md) and [`docs/plan/00-overview.md`](docs/plan/00-overview.md)
for the full architecture, domain types, and design decisions.

## Development

```bash
cargo build                  # compile
cargo clippy --all-targets   # lint — kept clean as the CI bar
cargo test                   # all tests
cargo run                    # launch the TUI (needs a TTY)
```

Snapshot tests use [`insta`](https://insta.rs). When a snapshot is new or
intentionally changed, accept it with `INSTA_UPDATE=always cargo test` (or
`cargo insta review`). Pending `*.snap.new` files are gitignored — commit the
accepted `.snap`.
