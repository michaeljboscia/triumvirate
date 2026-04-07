# Stenographer Replacement — Twin Opinions (2026-04-07)

## Gemini's Take

**Verdict: Build your own — the "Triumvirate Ledger"**

### Why NOT claude-mem as-is:
- Inherently coupled to Claude Code's plugin/hook lifecycle
- We run a TRIUMVIRATE (Claude, Gemini, Codex) — a Claude-specific memory system leaves Gemini and Codex blind
- Introduces Bun dependency

### Why NOT the others:
- memory-mcp / mcp-memory-keeper: Too simple. Relies on agents proactively calling tools — data loss when system prompt drifts
- Mem0: Massive overkill. Not building a global enterprise RAG pipeline
- claude-session-tracker: One-way data black hole

### Gemini's Proposed Architecture: "Triumvirate Ledger"

**Core principle:** Stop orchestrating memory with shell scripts. Move state management into SQLite, embed capture in the Rust daemon, expose retrieval via MCP.

1. **Storage: SQLite (WAL mode)**
   - Drop JSON state files entirely
   - WAL handles concurrent reads/writes natively
   - ACID compliant — the deadlock we experienced is impossible
   - FTS5 for full-text search without vector DB overhead

2. **Ingestion: Hybrid Capture**
   - **Passive capture (safety net):** Rust daemon already routes messages between user and agents. Async dump every turn (prompt + response) into raw `transcripts` table. If daemon is running, data is saving. Zero hooks required.
   - **Active capture (high signal):** MCP tool `triumvirate_record_decision` for sprints, bug root causes, architectural decisions. Writes to `summaries` table.

3. **Retrieval: MCP Interface**
   - `query_ledger(query)`: FTS5 search over summaries and transcripts
   - `get_session_context(session_id)`: Pull compressed state of previous session
   - Available to ALL THREE agents equally

4. **Health: localhost:9090/health**
   - Read-only HTTP endpoint from daemon
   - Basic HTML table: last 50 writes, DB file size
   - Rule: If no write in last 5 minutes during active session = system is dead

### Key Gemini Quote:
> "Fail-silent data infrastructure is worse than no infrastructure. If a database goes down, the app should crash so you know it's down."

---

## Codex's Take

**Verdict: Build your own, but fork/adopt claude-mem as ingestion core. Don't start from scratch.**

### Why NOT claude-mem as-is:
- Not production-hardened for our failure modes
- But good enough for immediate pilot

### Why NOT greenfield:
- "Pure greenfield is how you lose another month"

### Codex's Proposed Architecture: Hardened claude-mem Fork

1. **Storage: SQLite in WAL mode**
   - Tables: `events` (append-only raw), `jobs` (compression tasks), `artifacts` (summaries), `sessions`, `health_heartbeats`
   - FTS5 first. Add vector only if recall is measurably bad
   - Optional: local embedding model (Ollama) to avoid API tax

2. **Trigger Model**
   - Hooks: SessionStart, PostToolUse, Stop, SessionEnd
   - Hook responsibility: validate payload → append raw event → return
   - NO summarization inside hook path
   - Async worker consumes `jobs` to compress/index later

3. **Deadlock Prevention**
   - Delete ALL global mutable flags (precompact.active-style)
   - Per-job state machine in DB: pending → running → done/failed
   - Running jobs require heartbeat timestamp
   - Lease TTL reclaim: running + heartbeat stale (>90s) = auto-reset to pending
   - Idempotency key: session_id + event_type + sequence
   - Crash-safe spool fallback: if DB write fails, append NDJSON to local spool; replay daemon drains on recovery

4. **Fail-Loud**
   - Hook can't persist to DB or spool → explicit stderr + non-silent log
   - Daily canary: synthetic event injection; alert if not visible in 30s
   - "No writes in N minutes while active session" = critical alarm

5. **Health Visibility**
   - Local health UI (from claude-mem): queue depth, last write time, stale jobs, spool size, compression lag
   - CLI `steno doctor`: one-shot diagnostics with pass/fail
   - `session-logs/steno-health.md` updated periodically

6. **Multi-Project Isolation**
   - Per-project DB: `<project>/.stenographer/steno.db`
   - Optional global index DB for cross-project search (read-only aggregate)
   - No shared mutable state across projects

7. **Cost Control**
   - Tier 0: always local extractive summary (cheap, deterministic)
   - Tier 1: optional LLM abstraction for high-value windows only
   - Budget guardrails: max tokens/day, max calls/session, hard cutoff to local-only

### What to take from each candidate:
- **claude-mem:** hook coverage, SQLite pipeline, UI pattern
- **memory-mcp:** simple retrieval contract for agents
- **Mem0:** skip unless cross-domain long-horizon reasoning needed
- **mcp-memory-keeper:** explicit query tools pattern
- **worktrace/session-tracker:** markdown/git export as secondary artifact, never primary store

### Key Codex Quote:
> "If ingestion and summarization are coupled, you will lose data again."
> "If health is not visible in one command, it will silently die again."

---

## Convergence Analysis (Claude)

### Where the twins AGREE:
1. **Build, don't adopt** — neither says use claude-mem as-is
2. **SQLite in WAL mode** — both independently chose this as storage
3. **FTS5 for search** — vector search is unnecessary for this problem
4. **Kill all JSON flag coordination** — the root cause of every failure
5. **Decouple ingestion from summarization** — raw write first, compress async
6. **Health endpoint is mandatory** — if you can't see it's working, it's not working
7. **Fail-loud, not fail-silent** — the #1 design principle

### Where they DIVERGE:
| Topic | Gemini | Codex |
|-------|--------|-------|
| Starting point | Build from scratch, embed in Rust daemon | Fork claude-mem, harden it |
| Ingestion | Passive (daemon intercepts traffic) + Active (MCP tool) | Hook-based (like claude-mem) + spool fallback |
| Multi-agent | MCP tools available to all 3 agents | Claude-first hooks, extend later |
| Cost | No mention | Two-tier compression with budget guardrails |
| Spool fallback | Not mentioned | NDJSON crash-safe spool if DB write fails |

### My Synthesis:
Codex's approach is more pragmatic and shippable. Gemini's insight about multi-agent access via MCP is critical and should be merged in. The architecture should be:

1. **Storage:** SQLite WAL + FTS5 (both agree)
2. **Ingestion:** Claude Code hooks for raw event capture (Codex) + MCP tool for all agents to record decisions (Gemini)
3. **Compression:** Async worker, decoupled from ingestion (both agree)
4. **Fallback:** NDJSON spool if DB write fails (Codex — essential)
5. **Health:** HTTP endpoint + `steno doctor` CLI + periodic markdown (Codex, enhanced by Gemini's daemon embedding)
6. **Multi-project:** Per-project DB (Codex)
7. **Multi-agent:** MCP retrieval tools available to all 3 (Gemini)
