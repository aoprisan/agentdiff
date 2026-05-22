# Phase 4 — Risk engine & risk inbox (optional, deprioritized)

> Prereq: Phase 2+ (works on any `Diff`). Goal: auto-flag the dangerous things agents tend to do and let you triage them first. Deprioritized — intent correlation (Phase 2) was chosen as the #1 edge, so this is a "nice to have." Adds a new `risk/` module; the `Diff` spine is unchanged.

## In scope
- `risk/mod.rs`: `Rule` trait (`fn evaluate(&self, diff: &Diff) -> Vec<RiskFlag>`) + a `RiskEngine` running the registered rule set and sorting flags by `(severity, category)`.
- `domain/risk.rs`: `RiskFlag{category,severity,target:HunkRef,message,evidence}`, `Severity`, `RiskCategory` (added in this phase, not Phase 0).
- Rules (ship highest-value first): new dependencies (`Cargo.toml`/lockfile/`package.json`, globset-gated), introduced `unwrap()`/`expect()`/`panic!`/`unreachable!`, secret-ish patterns (entropy + known token prefixes), placeholder markers (TODO/FIXME/"in a real implementation"/stub), whole-file deletions, weakened/skipped/deleted tests, CI/infra/config edits (`.github/**`, Dockerfile). Lower priority: large generated/minified blocks.
- Side panel: show flags for the current file/hunk (alongside intent).
- **Risk Inbox** view: a flat, severity-sorted list of flags across the whole diff; `Enter` jumps to the hunk. The headline triage UX for this phase.

## Out of scope (deferred)
Config-driven user rules without recompiling (Phase 6 stretch), action on flags (Phase 5 / read-only decision).

## Crates to add
None beyond `globset` (already present). Optionally a small entropy helper (hand-rolled).

## Tasks (ordered)
1. `domain/risk.rs` types; anchor `RiskFlag.target` to `HunkRef` so flags survive re-diff like verdicts.
2. `risk/mod.rs`: `Rule` trait + `RiskEngine` + default registry.
3. `risk/rules/*`: implement rules in priority order; each is a small unit-tested function over a `FileChange`/`Hunk`.
4. Run the engine on the worker after each (re-)diff → `RiskReport` in `AppState`.
5. Side-panel integration + badges on the file tree (flag counts by severity).
6. `tui/widgets/risk_inbox.rs` + a `View::RiskInbox` toggle and jump-to-hunk.

## Acceptance
- A diff that adds a dependency, an `unwrap()`, a `TODO`, and deletes a test produces exactly those flags at sensible severities.
- Flags re-anchor across re-diff (don't duplicate or vanish spuriously).
- Risk Inbox lists all flags severity-sorted; `Enter` jumps to the right hunk; file tree shows per-file flag counts.
- `insta` snapshots: rule outputs on fixtures; rendered Risk Inbox buffer.

## Definition of done
Optional triage layer that surfaces risky agent edits first. Commit. Only build this once phases 0–2 are solid.
