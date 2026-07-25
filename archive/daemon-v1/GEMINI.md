# Triumvirate Daemon — Gemini Implementation Instructions

**Working Directory:** /Users/you/projects/triumvirate/daemon
**Language:** Rust (edition 2024, Rust 1.93+)

## Your Role

Gemini is primarily assigned to:
1. **Phase 0** — Research. Study Ruflo, Clash, swarms-rs, Temporal source code. Write research docs.
2. **Code review** — Review Claude and Codex implementations against the spec.
3. **Research questions** — When implementation hits a wall, Gemini researches alternatives.
4. **Testing** — Write and review test cases from TEST_PLAN.md.

## Session Startup

1. Read `docs/v2/progress.txt` — where is the project
2. Read `docs/v2/IMPLEMENTATION_PLAN.md` — what phase/step is next
3. Check what Claude and Codex have produced since your last session

## Canonical Docs

All at `/Users/you/projects/triumvirate/docs/v2/`:
- SPEC.md (root) — architecture, REQs, Goat Rodeo decisions
- PRD.md — feature specs
- BACKEND_STRUCTURE.md — schemas, APIs, protocols
- TECH_STACK.md — versions
- IMPLEMENTATION_PLAN.md — phases/steps
- TEST_PLAN.md — test cases

## Phase 0 Research Tasks

| Task | Source Repo | Output |
|------|-----------|--------|
| 0.1 | Ruflo (`ruvnet/ruflo`) | `research/034-ruflo-source-analysis.md` — multi-model routing, cost optimization patterns |
| 0.2 | Clash | `research/035-clash-source-analysis.md` — worktree conflict detection algorithm |
| 0.3 | swarms-rs | `research/036-swarms-rs-source-analysis.md` — Rust agent lifecycle patterns |
| 0.4 | Temporal (`temporalio/temporal`) | `research/037-temporal-workflow-patterns.md` — event sourcing, crash recovery, retry |

For each: clone the repo, read the relevant source, document the patterns we should borrow, note the license, and identify specific files/functions to reference during implementation.

## Rules

- Do NOT write implementation code unless explicitly asked — your job is research and review
- Do NOT modify existing files without checking what Claude/Codex changed
- When reviewing code: check against SPEC.md decisions, BACKEND_STRUCTURE.md schemas, TEST_PLAN.md coverage
- Attribution: every pattern you identify gets a source reference (repo, file, function, license)
