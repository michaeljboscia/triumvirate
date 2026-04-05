# Goat Rodeo Decision Ledger — Triumvirate v2 Conversational Parity

**Date:** 2026-04-05
**Rounds:** 9
**Questions:** 62
**Decisions:** 52 (25 user, 27 clanker consensus)

---

## User Decisions

| # | Round | REQs | Decision | Why |
|---|-------|------|----------|-----|
| 1 | R1 | US-8 | Persistent sessions are first-class user story | 118 Gemini daemon sessions prove demand |
| 2 | R1 | REQ-002 | Explicit trigger routing only, expandable later | No false positives, predictable behavior |
| 3 | R1 | REQ-004 | Fail fast, 3x visible retry, non-blocking, dead drop fallback | User wants to see EVERY failure in the moment |
| 4 | R1 | REQ-004 | Dead drop transport via osascript Terminal spawn | User never leaves Claude, agents always reachable |
| 5 | R1 | US-5 | Fleet: headlines in Claude, details in dashboard | Dashboard exists for deep dives |
| 6 | R1 | REQ-005 | Daemon down = RED STATE, degraded mode, TS safety net | 2/3 of brain is gone, must be loud |
| 7 | R2 | REQ-005 | Loud retries on daemon restart, no silent buffering | "I have a history of swallowing errors — a lot" |
| 8 | R2 | REQ-006 | 37-item checklist + reliability baseline (measure first, set SLO after) | 16% failure rate was invisible until we measured |
| 9 | R2 | REQ-015 | Dead drop: handoff tracking + diagnostic logging + pattern detection | Evidence collection automated, diagnosis stays human |
| 10 | R4 | REQ-028 | JIT spawn, stay ALIVE, no kill, no hibernate. Ever. | Tokens aren't infinite (no hibernate), agents must be ready (no kill) |
| 11 | R5 | REQ-028 | Pre-warm with context per request, not dump on spawn | Fast AND useful |
| 12 | R3 | Migration | Swap config day one, TS is safety net | "RUST NOW. If it's a clusterfuck we turn the old one back on" |
| 13 | R3 | REQ-030 | Pricing removed. Subscriptions, not API. | "There is no pricing — these are CLIs" |
| 14 | R2 | Dead Drop | PID tracked, Claude catches orphaned processes | State visibility is non-negotiable |
| 15 | R2 | Dead Drop | Adapter checks for results on every tool call, canonical naming, 7-day GC | System doesn't rot |
| 16 | R4 | REQ-006 | FULL parity. All 37 items. No MVP. No cuts. | "I DON'T WANT AN MVP — I WANT IT TO FUCKING WORK" |
| 17 | R5 | US-7 | Real install CLI, distributable, not scripts | "Stop trying to make it smaller — this is a real fucking thing" |
| 18 | R5 | Build | 10 testable increments | Smaller blast radius, test as you go |
| 19 | R7 | Build | Codex owns crate structure, granular preferred | Codex is building it |
| 20 | R7 | Architecture | Single binary with MCP stdio + HTTP dashboard | Research confirmed: single process for single-user local tools |
| 21 | R8 | Architecture | REVERTED: single-binary architecture; adopted two-mode binary (daemon + mcp bridge) | Claude dying would kill agents — unacceptable |
| 22 | R7 | Build | Clean rewrite, old code is reference | "Core functionality missed on first go-round" |
| 23 | R7 | Repo | feat/mcp-first, daemon-v2/, old code untouched | Clean separation |
| 24 | R4 | Scope | Not gold-plating — this IS the product | "MOTHERFUCKER" |
| 25 | R4 | Pre-warm | Machine-wide sessions, 3-4 projects, no cap | "I'm only 1 person" |

## Clanker Consensus

| # | Round | Decision |
|---|-------|----------|
| 1 | R1 | Skills become MCP tool aliases |
| 2 | R1 | Dashboard shows MCP-path + fabric events (correlated by request_id) |
| 3 | R1 | Codex thread IDs in daemon SQLite |
| 4 | R1 | MCP presence <=10ms overhead (release blocker) |
| 5 | R3 | 100ms adapter overhead acceptable |
| 6 | R3 | Claude passes cwd/repo/branch explicitly on every call |
| 7 | R3 | Timestamped unique dead drop filenames |
| 8 | R4 | FIFO queue per project for concurrent requests |
| 9 | R4 | Event tiers: headlines to Claude, firehose to dashboard |
| 10 | R4 | Detect $TERM_PROGRAM, fall back to Terminal.app |
| 11 | R4 | Adapt prompts per agent with role templates |
| 12 | R5 | Claude evaluates + synthesizes twin responses (not blind) |
| 13 | R5 | Coarse activity stages (planning/reading/drafting/finalizing) |
| 14 | R5 | Failure pattern alerts injected into MCP responses |
| 15 | R5 | Hardcoded default templates, user-overridable |
| 16 | R6 | Codex decides wrap vs rewrite per file |
| 17 | R6+R8 | Use rmcp crate (confirmed: progress + logging support) |
| 18 | R6 | Lightweight bootstrap prompt for pre-warm |
| 19 | R6 | Layered test harness: unit + mock CLI + e2e |
| 20 | R2 | Outbox logs in SQLite, indexed |
| 21 | R2 | Hardcoded triggers for now |
| 22 | R2 | Accept session loss on TS retirement |
| 23 | R8+R9 | Final spec rewritten from scratch before code |
| 24 | R9 | Local auth token (daemon.token, Bearer) |
| 25 | R9 | Thread-safe concurrent access |
| 26 | R9 | Real e2e test at every increment |
| 27 | R9 | Daemon stays HTTP-only, no native MCP |

## Contradictions Resolved

| Topic | Evolution | Final |
|-------|-----------|-------|
| Pre-warming | R1: warm on boot + 10min TTL → R3: kill sessions → R4: no kill, no hibernate → R5: alive + contextualized | JIT spawn, stay alive, context per request |
| Architecture | R7: single binary → R8: two-mode binary | Two-mode (Claude dying can't kill agents) |
| Pricing | Original spec: cost per turn → R3: removed | Quota only, not cost |
| Scope | Twins said "gold-plating" in R4 → User overruled | Full parity, no cuts |
