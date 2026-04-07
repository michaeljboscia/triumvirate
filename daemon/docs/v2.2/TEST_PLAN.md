# Test Plan — Triumvirate v2.2

---

## Phase 1: Ledger Ingestion + Health

| REQ | Acceptance Criteria | Test Type | Pass Condition | Reality Test |
|-----|-------------------|-----------|----------------|-------------|
| REQ-006 | SQLite in WAL mode | Unit | `PRAGMA journal_mode` returns `wal` | Open DB, check pragma. Non-WAL = fail. |
| REQ-007 | Per-project DB | Unit | Different projects get different DB files | Create two stores with different roots, verify distinct file paths on disk |
| REQ-007a | Project root resolution | Unit | All 4 fallback steps work | Test: env var set → uses it. Git repo → uses git root. .triumvirate/ dir → uses ancestor. None → uses scratch/sha256 |
| REQ-008 | Tables created | Unit | All 8 tables exist after open() | `SELECT name FROM sqlite_master WHERE type='table'` returns all 8 |
| REQ-009 | FTS5 enabled | Unit | FTS5 search returns results | Insert summary, search with `MATCH`, verify hit |
| REQ-009a | Write-path priority | Integration | Ingestion not starved under fleet load | Simulate 50 task updates + 50 ingestion events concurrently. Ingestion queue lag < 5s. |
| REQ-010 | Spool atomic write | Integration | Concurrent hooks don't corrupt | Spawn 10 parallel bash hooks writing to same spool dir. All files valid JSON. Zero corruption. |
| REQ-010a | No direct SQLite | Integration | Hook never opens DB | Strace/dtrace hook execution. No `open()` calls to ledger.db. |
| REQ-010b | LRU + 60s sweep | Integration | Daemon discovers new projects | Write spool file without wake ping. Wait 60s. File ingested. |
| REQ-010c | Daemon truncation | Unit | Fields >64KB truncated with marker | Insert event with 100KB tool_output. Verify DB value contains `[...truncated]` and is valid JSON. |
| REQ-011 | Spool overflow | Integration | Spool >100MB → degraded health | Fill spool to 101MB. Health returns `degraded`. Events still written. |
| REQ-011a | Spool drain on startup | Integration | Daemon restart drains spool | Write 5 spool files, start daemon, verify all 5 in SQLite, spool dir empty. |
| REQ-011b | Idempotency dedupe | Unit | Duplicate events rejected | Ingest same event twice (same idempotency key). Second returns success but no duplicate row. |
| REQ-012 | Tier 0 compression | Integration | Summary produced from events | Ingest 10 events. Wait for worker. Verify ≥1 summary in summaries table. |
| REQ-013b | Heartbeat TTL | Unit | Stale jobs reset | Set job to `running` with heartbeat 2 minutes ago. Run reclaim. Job is `pending`. |
| REQ-015 | Health endpoint | E2E | HTTP returns valid JSON | `curl localhost:8080/ledger/health` → JSON with all required fields |
| REQ-015a | Health MCP tool | E2E | MCP tool returns same data as HTTP | Call `ledger_health()` via MCP, compare with HTTP endpoint output |
| REQ-016 | Degraded detection | Integration | No events in 5 min + active session → degraded | Start session, wait 5 min without events, check health status |
| REQ-017-L | Doctor diagnostics | E2E | `triumvirate doctor` includes Ledger checks | Run doctor, verify output contains: DB exists, WAL mode, spool status, job status |
| REQ-018a | .gitignore init | Integration | .triumvirate/ added to .gitignore | Init Ledger in git repo without .triumvirate/ in .gitignore. Verify it's appended. |

---

## Phase 2: Knowledge

| REQ | Acceptance Criteria | Test Type | Pass Condition | Reality Test |
|-----|-------------------|-----------|----------------|-------------|
| REQ-014 | FTS5 search | Integration | Query returns relevant summaries | Insert 3 summaries ("auth bug", "database migration", "CSS layout"). Query "auth" returns only the auth summary. |
| REQ-014a | XML marker parsing | Integration | Gemini output with marker → ledger write | Feed agent-adapter a mock stdout containing `<triumvirate_tool name="ledger_record">...`. Verify summary in DB. |
| REQ-014b | Prompt injection | Integration | Gemini startup prompt contains XML instructions | Spawn Gemini via ask_agent, capture the prompt, verify XML tool instructions present. |
| REQ-019 | Lessons table | Unit | CRUD works | Add lesson, query it, validate it, list it. All operations succeed. |
| REQ-020-LL | Confidence decay | Unit | Decay formula correct | Add lesson 50 days ago (mock time). Query with min_confidence=0.3 → found. Query with min_confidence=0.4 → not found (0.36 < 0.4). |
| REQ-022 | Auto-lesson from summary | Integration | bug_fix summary → lesson created | Insert summary with type=bug_fix. Verify lesson auto-created with confidence=0.6. |

---

## Phase 3: Fleet Core

| REQ | Acceptance Criteria | Test Type | Pass Condition | Reality Test |
|-----|-------------------|-----------|----------------|-------------|
| REQ-028 | Worktree creation | Integration | Worktree exists at expected path | Call fleet_spawn(dry_run=false). Verify worktree directory exists. `git -C <worktree> status` succeeds. |
| REQ-030 | Clean tree check | Integration | Dirty tree → error | Make uncommitted change. Call fleet_spawn. Verify error message names the dirty files. |
| REQ-032 | Atomic claiming | Integration | Two agents can't claim same task | Simulate two concurrent fleet_claim_task for same task_id. Exactly one succeeds, one gets "already claimed." |
| REQ-033 | Dependency resolution | Unit | Blocked tasks not claimable | Create T-001 (no deps) and T-002 (depends on T-001). T-002 not in claimable list. Complete T-001. T-002 now claimable. |
| REQ-034a | Crash recovery | Integration | Stale fleet cleaned on restart | Set fleet to `running` in SQLite. Start daemon. Fleet marked `failed`. Worktrees cleaned. Recovery event in Ledger. |
| REQ-035 | Dry-run default | E2E | fleet_spawn without dry_run=false returns plan | Call fleet_spawn(task="test", agents={claude:1}). Response is plan text, not fleet_id. |
| REQ-037 | Task file delivery | Integration | fleet-task.md exists in worktree | After fleet_spawn, check `<worktree>/.triumvirate/fleet-task.md` exists with correct frontmatter. |
| REQ-038 | Sequential merge | Integration | Merges happen in completion order | Fleet with 3 agents. Complete in order: agent-2, agent-3, agent-1. Merge order matches: 2, 3, 1. |

---

## Phase 4: Peer Review

| REQ | Acceptance Criteria | Test Type | Pass Condition | Reality Test |
|-----|-------------------|-----------|----------------|-------------|
| REQ-024-PR | Non-self-review | Unit | Author can't be reviewer | review_request(author=codex) assigns claude or gemini, never codex. |
| REQ-024a | Bounded queue | Integration | Max 2 inflight respected | Submit 5 review requests. Only 2 processing at a time. Others queued. |
| REQ-024a | Timeout | Integration | 120s timeout → failed | Submit review, mock reviewer that never responds. After 120s, review state = failed. |
| REQ-040 | Review before merge | Integration | Merge blocked without approval | Fleet completes. Skip review. Attempt merge. Merge rejected. |
| REQ-040b | Skip flag | Integration | TRIUMVIRATE_FLEET_SKIP_REVIEW=1 bypasses | Set env var. Fleet merge proceeds without review. Ledger contains review_skipped entry. |

---

## Phase 5: Dashboard

| REQ | Acceptance Criteria | Test Type | Pass Condition | Reality Test |
|-----|-------------------|-----------|----------------|-------------|
| REQ-041 | SPA builds | Build | `npm run build` produces dist/ | dist/ contains index.html + JS + CSS |
| REQ-042 | rust-embed serves | E2E | `curl localhost:8080/` returns HTML | Response contains `<html` and Svelte app mount point |
| REQ-044 | WebSocket streams | E2E | wscat connects and receives events | Connect to ws://localhost:8080/ws. Trigger agent action. Receive JSON event. |
| REQ-044a | No local execution | E2E | MCP bridge errors without daemon | Stop daemon. Call ask_agent via MCP. Receive explicit error with recovery instructions. |
| REQ-047 | Health indicator | E2E | Dashboard shows correct color | Active session + events → green. Active session + no events for 5 min → red. |

---

## Phase 6: Enrichment + Codex

| REQ | Acceptance Criteria | Test Type | Pass Condition | Reality Test |
|-----|-------------------|-----------|----------------|-------------|
| REQ-049 | OutboxEvent fields | Unit | New fields serialize/deserialize | Create OutboxEvent with all 3 new fields. Round-trip through JSON. Fields present. |
| REQ-050 | Fields populated | Integration | Agent execution fills fields | Run ask_agent(codex, "echo hello"). OutboxEvent contains token_usage and tool_name. |
| REQ-051 | App-server parser | Unit | JSON-RPC lifecycle parsed | Feed mock app-server JSONL through CodexAppServerParser. All events recognized. |
| REQ-054 | --full-auto injection | Integration | Codex spawned with flag | Set CODEX_AUTO_APPROVE=1. Spawn Codex. Verify command includes --full-auto. |
| REQ-055 | Auto-approve audit | Integration | Ledger contains auto_approved entry | Run auto-approved Codex action. Query Ledger for summary_type=auto_approved. Found. |

---

## Phase 7: GC

| REQ | Acceptance Criteria | Test Type | Pass Condition | Reality Test |
|-----|-------------------|-----------|----------------|-------------|
| REQ-056 | Retention policy | Unit | Old events without summaries deleted | Insert event 31 days ago with no summary. Run GC. Event gone. |
| REQ-056 | Events with summaries kept | Unit | Old event WITH summary survives | Insert event 31 days ago with linked summary. Run GC. Event still exists. |
| REQ-058 | Startup GC | Integration | GC runs if >24h since last | Set last_gc to 25h ago. Start daemon. GC runs. |
| REQ-058 | Fleet blocks GC | Integration | Active fleet prevents GC | Set fleet to `running`. Start daemon. GC skipped. |

---

## Cross-Phase Integration Tests

| Test | Features | Pass Condition |
|------|----------|----------------|
| Full lifecycle | All | Hook fires → spool → drain → SQLite → WebSocket → dashboard renders event |
| Fleet + review + merge | FEAT-003 + FEAT-004 | Spawn fleet → agents work → reviews → merge → done |
| Crash + recovery | FEAT-001 + FEAT-004 | Kill daemon mid-fleet → restart → fleet failed → spool drained → zero data loss |
| Lesson auto-create | FEAT-001 + FEAT-002 | Error events → compression → bug_fix summary → lesson auto-created |
