# Triumvirate v2 — Roadmap

**Date:** 2026-04-05
**Source of truth:** SPEC_FINAL.md
**Nothing is cut. Everything is sequenced.**

---

## NOW — Conversational Parity (Increments 1-10)

The product: "ask the twins" works with lifecycle visibility.

| Increment | What | User Story |
|-----------|------|-----------|
| 1a | Scaffold + rmcp ping tool | Foundation |
| 1b | ask_agent happy path (Gemini) | US-2 |
| 2 | Explicit session fan-out (`spawn_session` + parallel `ask_session`) | US-1 |
| 3 | Timeout + retry + loud failure | US-4 |
| 4 | Sessions (spawn/ask/dismiss) | US-8 |
| 5 | Alive sessions + context | US-1, US-8 |
| 6 | Outbox logging + context detection | US-4 |
| 7 | Remaining TS parity | All |
| 8 | Dead drop fallback | US-4 |
| 9 | Diagnostics + auth + concurrency | US-4, US-7 |
| 10 | Install CLI + migration swap | US-7 |

**Gate:** TS parity checklist all pass. Reliability baseline measured. Migration swap complete.

---

## NEXT — Dashboard + Observability

The dashboard comes back once the daemon API is stable.

| Feature | What | User Story |
|---------|------|-----------|
| Dashboard rebuild | Svelte 5 + Tailwind against new daemon API | US-6 |
| MCP-path events in dashboard | Correlated by request_id | US-6 |
| Agent health grid | Real-time status, per-agent | US-6 |
| Quota visibility | Per-agent subscription quota usage | US-6 |
| 4K layout | Full-width responsive grid | US-6 |
| WebSocket auto-reconnect | BUG-002 fix in new architecture | US-6 |

**Gate:** US-6 acceptance test passes. Dashboard works on 4K monitor.

---

## THEN — Fleet + Debate

Power tools layer on top of working conversational parity.

| Feature | What | User Story |
|---------|------|-----------|
| Fleet spawn via MCP | `/fleet 3 claude, 2 codex: build X` | US-5 |
| Git worktree orchestration | Isolated workspaces per fleet agent | US-5 |
| Task dependencies | Blocked/unblocked lifecycle | US-5 |
| Sequential merge | Dependency-ordered, conflict detection | US-5 |
| Fleet headline events | Milestones in Claude, detail in dashboard | US-5 |
| Debate workflow via MCP | `/debate Redis vs Postgres` | US-5 variant |
| Debate structured rounds | Challenge → vote → resolution | US-5 variant |

**Gate:** US-5 acceptance tests pass. Fleet produces merged code from N agents.

---

## FUTURE — Extensibility + Intelligence

| Feature | What | User Story |
|---------|------|-----------|
| Agent extensibility | New connectors: Ollama, Scout 10M, API-backed agents | New US |
| Configurable "twins" | Choose which 2 agents are the default pair | US-1 config |
| Intent-based routing | Claude detects "twins" intent without explicit trigger | US-1 enhancement |
| Proactive agent contribution | Agents surface insights unprompted | US-9 |
| Governance / Cedar policies | Agent action approval gates | Tier 4 |
| Prometheus metrics | System-level observability | Tier 4 |
| Langfuse traces | LLM-level observability | Tier 4 |
| Stenographer | Raw event recording (no LLM summaries) | Tier 4 |
| Cost tracking (if API mode added) | Per-token cost attribution | Only if subscriptions → API |
| Distributable product | npm package, brew formula, real installer | Product |

---

## Existing v1 Code Status

| Component | Status | Location |
|-----------|--------|----------|
| Agent connectors (claude.rs, gemini.rs, codex.rs) | Reference for rewrite | `daemon/crates/agentd/src/agent/` |
| Proto crate (event parsers, BUG-001 fixes) | Carry over | `daemon/crates/proto/` |
| Mock CLIs | Carry over | `daemon/crates/mock-*/` |
| Message fabric (tokio broadcast) | Reference | `daemon/crates/agentd/src/fabric/` |
| Memory store (SQLite WAL) | Schema reference | `daemon/crates/agentd/src/memory/` |
| Workflow engine | Reference for fleet/debate phase | `daemon/crates/workflow/` |
| Fleet coordinator | Reference for fleet phase | `daemon/crates/agentd/src/fleet/` |
| Debate scaffold | Reference for debate phase | `daemon/crates/agentd/src/web/server.rs` |
| Governance (Cedar) | Reference for governance phase | `daemon/crates/agentd/src/governance.rs` |
| Dashboard (Svelte 5 + Tailwind) | Reference for dashboard phase | `daemon/frontend/` |
| Config (TOML) | Reference | `daemon/crates/agentd/src/config.rs` |
| Stenographer | Reference for stenographer phase | `daemon/crates/agentd/src/steno/` |
| 37 research artifacts | Always available | `research/` |
| TS inter-agent MCP server | Safety net during migration | `~/.claude/mcp-servers/inter-agent/` |

---

## Decision Log

Every feature placement is traceable to a goatrodeo decision:

- Dashboard deferred: Tier 1-2 before Tier 3-4 (SPEC_FINAL.md)
- Fleet/debate deferred: same rule
- Agent extensibility: emerged R5 (US-9 proactive contribution enabled by alive sessions)
- Nothing is removed. Pricing tracking removed because subscriptions, not API (R3 decision #13).
