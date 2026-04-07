# CLAUDE.md — Triumvirate v2.2 Build Context

## What This Is

v2.2 "The Accountability Release" — 9 features, 4 new crates, ~98 REQs. Spec survived an 8-round goat rodeo with twin review every round.

## Canonical Docs

| Doc | Path | Purpose |
|-----|------|---------|
| SPEC.md | `daemon/docs/v2.2/SPEC.md` | Final spec (source of truth for all REQs) |
| PRD.md | `daemon/docs/v2.2/PRD.md` | Features with FEAT-IDs and acceptance criteria |
| APP_FLOW.md | `daemon/docs/v2.2/APP_FLOW.md` | User flows with failure paths |
| TECH_STACK.md | `daemon/docs/v2.2/TECH_STACK.md` | Dependencies, env vars, build pipeline |
| BACKEND_STRUCTURE.md | `daemon/docs/v2.2/BACKEND_STRUCTURE.md` | SQLite schema, traits, HTTP endpoints |
| IMPLEMENTATION_PLAN.md | `daemon/docs/v2.2/IMPLEMENTATION_PLAN.md` | Phased plan with wave ordering |
| TEST_PLAN.md | `daemon/docs/v2.2/TEST_PLAN.md` | Acceptance + reality tests per REQ |
| FRONTEND_GUIDELINES.md | `daemon/docs/v2.2/FRONTEND_GUIDELINES.md` | Svelte 5 architecture, file structure |
| DESIGN_SYSTEM.md | `dashboard/DESIGN_SYSTEM.md` | Colors, typography, spacing, components |

## Crate Map (13 total)

Existing (9): shared-types, daemon-core, mcp-bridge, mcp-tools, agent-adapter, agent-worker, daemon-http, fallback-outbox, triumvirate

New (4): **ledger**, **fleet**, **peer-review**, **dashboard**

## Key Architecture Decisions

- Spool-first ingestion — hooks write to directory, daemon drains async
- Daemon-owned SQLite — single writer, no direct DB access from hooks
- Daemon-proxy-only MCP bridge — no local execution fallback
- Full agents for peer review — spawn on demand
- Parallel reviews, sequential merge
- Untracked fleet-task.md (gitignored, not committed)
- Confidence decay at query time

## Build Order

Phase 1 (Data) → Phase 2 (Knowledge) → Phase 3+5 parallel (Fleet + Dashboard) → Phase 4 (Review) → Phase 6+7 (Codex + GC)

## Rules

- `ledger` crate takes absolute PathBuf only — no git resolution inside
- `fleet` crate uses GitOps trait — no direct git shell-outs
- All new MCP tools defined in shared-types DTOs
- DESIGN_SYSTEM.md must be approved before dashboard dev
- Hooks use POSIX builtins only — no uuidgen, no sqlite3, no python3
