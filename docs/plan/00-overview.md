# agentdiff — overview & shared reference

## Context

You run Claude Code in auto/accept-edits mode, so the agent makes many edits across many files without you watching. The post-run **review of the working tree is your only control point** — but `git diff` (even with delta) treats those changes like any other commit: flat, intent-free, and not scoped to "what this one agent run touched." This tool is built for exactly that review step.

### Decisions that shape everything

- **#1 differentiator: agent-intent correlation.** For each changed file/hunk, show *what the agent said it was doing*, pulled from the Claude Code session transcript. Headline feature, in the MVP.
- **Deep Claude Code integration.** By default, scope the diff to a single agent **run** and diff against that run's *pre-run* file content (Claude's own file-history backups), so the view is "exactly what this autonomous run changed," even across intermediate commits. Falls back to plain git when no session is found.
- **Read-only.** No reverting, staging, or exporting. Verdicts (approved / needs-attention) are a *personal triage checklist* over a large diff, persisted across runs — no tree mutation. (Deliberately drops the highest-correctness-risk component, hunk-level `git apply --reverse`.)
- **Line-based rendering** with syntect highlighting + intra-line word diff. Structural/AST diff is out of scope.

### Verified facts (checked on this machine — not assumptions)

- Transcripts: `~/.claude/projects/<slug>/<session-uuid>.jsonl`; `slug` = absolute cwd with `/` and `.` → `-`.
- `~/.claude/file-history/<session-uuid>/<backupFileName>` holds verbatim **pre-edit** content. Dir exists with **213** session backup folders.
- `permission-mode` records carry `auto` / `plan` / `default` — `auto` is the autonomous span to segment on.
- `file-history-snapshot` records: `{messageId, snapshot:{messageId, trackedFileBackups, timestamp}}`. Files accumulate as they're first touched (early snapshots empty), each at its **pre-edit** `@v1`; a later snapshot (`isSnapshotUpdate:false`) **re-baselines** edited files at their *current* content under a higher `@vN`. So a file's pre-run content is its **earliest (lowest-version)** backup in the span — not the latest. `trackedFileBackups[path] = {backupFileName, version, backupTime}`; `backupFileName: null` ⇒ agent-created file. Path keys are sometimes absolute and may point outside the repo.
- Intent is **not** colocated with the edit: an `Edit`/`Write`/`MultiEdit` `tool_use` reaches its reasoning by walking `parentUuid` up to the nearest preceding `assistant` text turn (confirmed: 1 hop → `"Now let me start writing files…"`).

## Stack

| Concern | Crate | Role |
|---|---|---|
| TUI | `ratatui` + `crossterm` | immediate-mode widgets; rebuild frame from `AppState` each tick |
| Git read | `git2` (libgit2) | status incl. untracked, read blobs at HEAD/index/ref, base diffs |
| Diffing | `similar` | line diff for untracked/backup files git2 won't diff natively; intra-line **word diff** on every hunk |
| Highlight | `syntect` | lazy, viewport-only syntax highlighting (cached) |
| Session | `serde` + `serde_json` | stream-parse JSONL; `#[serde(tag="type")]` enum + `#[serde(other)]` so format drift never crashes |
| Time | `jiff` | parse ISO-8601 timestamps; order runs / pick latest |
| Watch | `notify` + `notify-debouncer-full` | live re-diff of tree + active JSONL (Phase 3) |
| Walk | `ignore` | gitignore-aware enumeration of untracked files |
| Match | `globset` | path patterns (untracked walk; risk rules later) |
| State | `directories` + `toml` | resolve state dir; persist review verdicts (human-diffable) |
| CLI/log | `clap` (derive), `tracing` + file subscriber | args; logs to file (never stdout while alt-screen is up) |
| Channels | `crossbeam-channel` | `select!` across input / fs / worker events |
| Errors | `thiserror` (libs), `anyhow` (edges) | typed inside modules, boxed at the boundary |
| Tests | `insta`, `tempfile`, `assert_cmd` | snapshot diff model + rendered `TestBackend` buffers; temp git repos |

**No `tokio`.** Input thread + watch thread + worker thread over `crossbeam-channel`, with a **generation counter** to drop stale results. Revisit only if remote/API calls appear.

## Architecture

**Diff is the spine.** Everything (intent, review state) anchors onto content-addressed `HunkRef`s. The "before" source only changes how the `Diff` is *built*; all downstream code is identical.

```
git repo ─► git::differ (git2) + git::untracked (similar) ─┐
                                                            ├─► domain::Diff ─► AppState ─► tui (render only)
~/.claude/projects/*.jsonl ─► session::transcript ─► runs ─►│        ▲                         │ commands
~/.claude/file-history/*           │ (backups = "before")  ─┘        │ AppEvent (input/fs/job)  ▼
                                   └─► session::intent (parentUuid DAG walk) ─► intent map ► intent panel
```

Default `DiffBase = AgentRun{session,run}` (latest `auto` run) → diff its pre-run backups vs current working tree. Fallback `DiffBase = WorkingTreeVsHead` when no session/run is found.

### Module layout (`src/`)

```
main.rs            clap parse, logging, panic-safe terminal, launch tui::run()
cli.rs             args: [PATH] --session <id> --run <n> --range A..B --no-session
config.rs          state-dir resolution, keymap/theme load (TOML)
error.rs           crate Error (thiserror) + Result

domain/            pure data + pure transforms, no IO, heavily unit-tested
  diff.rs          Diff, FileChange, Hunk, Line, LineKind, ChangeKind, DiffBase, FileId
  review.rs        ReviewState, HunkVerdict, HunkRef (content fingerprint)
  session.rs       AgentSession, AgentRun, Intent, ToolEditEvent, Backup, RunId
  ids.rs           content-addressed hashing for HunkRef/FileId (re-diff-stable)

git/               only place libgit2 lives (read-only)
  repo.rs          open repo, resolve refs, status incl. untracked (ignore-aware)
  differ.rs        git2 → domain::Diff for any DiffBase
  untracked.rs     synth "empty → content" diffs for agent-created files via `similar`

session/           Claude Code transcript + file-history (the differentiator)
  locate.rs        cwd → slug; list/rank sessions; latest-run detection
  transcript.rs    streaming JSONL parser → Vec<Record> (tagged enum + Other)
  runs.rs          segment by permission-mode; cumulative snapshots → AgentRun spans
  backups.rs       resolve trackedFileBackups → ~/.claude/file-history paths (None = created)
  intent.rs        walk parentUuid DAG: edit file_path → nearest preceding assistant text

watch/mod.rs       notify watchers (tree + active jsonl) + debouncer → AppEvent (Phase 3)

app/               UI-framework-agnostic core
  state.rs         AppState, View enum, cursors/scroll, job/generation tracking
  event.rs         AppEvent (Input, FsChanged, GitDone, SessionDone, Tick)
  update.rs        reducer (AppState, AppEvent) → mutation + side-effect requests
  commands.rs      Approve, NeedsAttention, NextHunk, NextFile, OpenSessionPicker, …
  keymap.rs        vim-like bindings → Command (configurable)

tui/               ratatui render only (reads AppState, no business logic)
  mod.rs           run(): terminal setup, threads, channels, loop, teardown
  layout.rs        3-pane + picker layout
  highlight.rs     syntect wrapper: cached, lazy, line-range highlighting
  theme.rs         add/del/intent color mapping
  widgets/  file_tree.rs  diff_pane.rs  intent_panel.rs  statusbar.rs
            session_picker.rs  help.rs
```

Dependency direction is acyclic: `domain` depends on nothing internal; `git`/`session`/`watch` depend on `domain`; `app` orchestrates IO modules via channels/traits; `tui` reads `app` + `domain`. (`risk/` and `action/` are intentionally absent until their phases.)

### Core domain types (frozen in Phase 0)

```rust
struct Diff { base: DiffBase, files: Vec<FileChange>, generated_at: Timestamp }
enum DiffBase { WorkingTreeVsHead, WorkingTreeVsIndex,
                Range{from:String,to:String}, AgentRun{session:SessionId, run:RunId} }
struct FileChange { id:FileId, path:PathBuf, old_path:Option<PathBuf>, change:ChangeKind,
                    is_binary:bool, is_created:bool, language:Option<String>,
                    hunks:Vec<Hunk>, stats:(usize,usize) }            // is_created = agent-created
struct Hunk { href:HunkRef, old:LineRange, new:LineRange, header:String, lines:Vec<Line> }
struct Line { kind:LineKind, old_no:Option<u32>, new_no:Option<u32>,
              text:String, intra:Vec<InlineSpan> }                    // intra = word-diff spans
struct HunkRef { path:PathBuf, fingerprint:u64 }                      // content-addressed
enum  HunkVerdict { Unreviewed, Approved, NeedsAttention }            // read-only triage marker
struct ReviewState { verdicts:HashMap<HunkRef,HunkVerdict>, notes:HashMap<HunkRef,String>,
                     collapsed:HashMap<PathBuf,bool>, last_session:Option<SessionId> }
struct AgentRun { id:RunId, mode:PermissionMode, started:Timestamp, ended:Option<Timestamp>,
                  snapshot:HashMap<PathBuf,Backup>, edits:Vec<ToolEditEvent> }  // ended None = live
struct Backup { backup_path:Option<PathBuf>, version:u32 }            // None = agent-created
struct Intent { file_path:PathBuf, text:String, source_uuid:String, confidence:f32 }
```

**Why content-addressed `HunkRef`:** the tree changes live (agent still editing; live re-diff). Anchoring verdicts/notes/intent to a content fingerprint re-attaches them across re-diffs; verdicts whose fingerprint vanished are surfaced as "changed since you reviewed."

## Cross-cutting invariants (hold from Phase 1 onward)

- **Virtualized diff pane.** Flatten `Diff` once into a `Vec<RowRef>` (file-header | hunk-header | line) + jump table (next/prev hunk, next/prev file). Render only the visible window; scrolling is O(1).
- **Lazy highlighting.** syntect only on visible rows + small overscan, LRU-cached by (FileId, line range, theme). Fully highlighting a 10k-line file is forbidden.
- **Word diff at model-build time** (stored in `Line.intra`); per frame just map spans → styled `Span`s.
- **Created/binary/huge files collapsed by default** with a summary; expand on demand.
- **Main thread renders only**; long ops run on the worker; a generation counter drops superseded results.
- **Session data is advisory.** git is always the source of truth; the tool stays useful (Phase 1 behavior) if parsing yields nothing.

## Top risks & mitigations

1. **CC format drift / private format.** Confine knowledge to `session/` behind types; serde `Other` fallback; build a `tests/fixtures/` corpus from real JSONL + file-history and snapshot-test the parser so a CC update fails a test, not the app.
2. **`trackedFileBackups` path keys** may be absolute or point outside the repo. Normalize to absolute, re-relativize to repo root, drop out-of-repo entries (optionally note "agent also touched" separately).
3. **Cumulative/empty early snapshots.** Use the *latest* snapshot within a run span as the authoritative pre-run map.
4. **Untracked diffs aren't native to git2.** `git::untracked` synthesizes them via `similar` into the same `Hunk`/`Line` model.
5. **Render perf on huge/generated diffs.** Virtualization + lazy highlight from Phase 1; benchmark a 50-file / 50k-line synthetic diff.
6. **Terminal corruption on panic.** Restoring panic hook installed in Phase 0.

## Global verification approach

- **Unit:** `domain` transforms + `git::untracked` via `insta`; `session::{runs,backups,intent}` against `tests/fixtures/`.
- **Integration:** `git::differ` against `tempfile` repos (modified/added/deleted/renamed/untracked/binary/CRLF/no-trailing-newline); snapshot the `Diff`.
- **Render:** ratatui `TestBackend` → buffer → `insta` snapshots for diff pane + intent panel.
- **Manual:** see each phase's acceptance criteria; ultimately, run `agentdiff` right after a real CC auto run and confirm per-run scoping + intent display.
