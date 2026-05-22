# agentdiff — implementation plan

A Rust TUI git-diff tool for reviewing what a Claude Code agent did in auto mode. The plan is split into per-phase files in this directory so they can be executed one at a time.

**Read [`00-overview.md`](00-overview.md) first** — it holds the context, decisions, verified facts, crate stack, architecture, module tree, and shared domain types that every phase references. Then execute the phases in order:

| # | File | What it delivers | Status |
|---|---|---|---|
| 0 | [`01-phase0-skeleton.md`](01-phase0-skeleton.md) | Panic-safe ratatui skeleton + frozen `domain` types | ✅ complete |
| 1 | [`02-phase1-worktree-reviewer.md`](02-phase1-worktree-reviewer.md) | Git working-tree reviewer (incl. untracked), virtualized line+syntect render, read-only verdicts | ⬜ next |
| 2 | [`03-phase2-cc-integration.md`](03-phase2-cc-integration.md) | **Headline:** per-agent-run diff scoping + intent correlation from Claude Code session data | ⬜ |
| 3 | [`04-phase3-live-polish.md`](04-phase3-live-polish.md) | Live re-diff while the agent runs, session-picker polish, config | ⬜ |
| 4 | [`05-phase4-risk-engine.md`](05-phase4-risk-engine.md) | Optional risk inbox (deprioritized) | ⬜ |
| 5 | [`06-phase5-future.md`](06-phase5-future.md) | Out-of-scope action layer (revert/export/stage) — kept architecturally compatible | ⬜ (out of scope) |

**MVP = phases 0–2.** Phases 3–5 are post-MVP. Each phase file is self-contained: goal, in/out of scope, crates to add, ordered tasks, files touched, and acceptance criteria.
