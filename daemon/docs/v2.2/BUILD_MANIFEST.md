# Build Manifest — Triumvirate v2.2

**Started:** 2026-04-07
**Completed:** 2026-04-07
**Agent:** Codex (GPT-5.2-Codex, long-running CLI session)
**Status:** RECONSTRUCTED from git history (Codex did not populate during build)

---

## Phase 0.5: Observability Foundation

| Task | SHA | Files | Status |
|------|-----|-------|--------|
| T-001 Wire JSON tracing + remove println | `ac0c110` | tracing_setup.rs, main.rs, cli_ops.rs | ✅ |
| T-002 Wire OTEL endpoint + ask_agent spans | `4247b39` | agent_exec.rs, tracing_setup.rs | ✅ |
| T-003 Expose Prometheus metrics | `20d05d6` | main.rs | ✅ |
| T-004 Instrument daemon public APIs | `765a8ba` | daemon-core/lib.rs, agent-worker/lib.rs, mcp-bridge/lib.rs | ✅ |

## Phase 1: Data (Ledger Ingestion + Health)

### Wave 0 — Contracts

| Task | SHA | Files | Status |
|------|-----|-------|--------|
| T-100 GitOps trait | `d378d9d` | shared-types/git_ops.rs | ✅ |
| T-101 Ledger DTOs | `46fd2f9` | shared-types/ledger.rs | ✅ |
| T-102 LedgerStore API signatures | `ea6d4dd` | ledger/lib.rs | ✅ |

### Wave 1 — SQLite Foundation

| Task | SHA | Files | Status |
|------|-----|-------|--------|
| T-103 Ledger crate deps | `60a5eb0` | ledger/Cargo.toml | ✅ |
| T-104 SQLite open + WAL + schema | `20669a1` | ledger/store.rs | ✅ |
| T-105 FTS5 virtual tables | `9c47282` | ledger/store.rs | ✅ |
| T-106 Idempotent ingest_event | `ecc04f0` | ledger/ingest.rs | ✅ |

### Wave 2 — Spool + Drain

| Task | SHA | Files | Status |
|------|-----|-------|--------|
| T-107 drain_spool + truncation | `2f2aeb1` | ledger/spool.rs | ✅ |
| T-108 POST /ledger/wake endpoint | `7894635` | triumvirate/main.rs | ✅ |
| T-109 60s spool sweep task | `c0d3fef` | triumvirate/main.rs | ✅ |
| T-110 Bash hooks | — | NOT IN GIT | ⚠️ See note |

### Wave 3 — Health + Doctor

| Task | SHA | Files | Status |
|------|-----|-------|--------|
| T-111 Health computation | `e647947` | ledger/health.rs | ✅ |
| T-112 GET /ledger/health endpoint | `c1a1c9a` | triumvirate/main.rs | ✅ |
| T-113 ledger_health MCP tool | `9ec64e4` | triumvirate/main.rs | ✅ |
| T-114 Doctor Ledger diagnostics | `0fba991` | cli_ops.rs, ledger/lib.rs | ✅ |
| T-115 .gitignore initialization | `ce7e0ff` | ledger/init.rs | ✅ |

### Wave 4 — Compression

| Task | SHA | Files | Status |
|------|-----|-------|--------|
| T-116 Tier 0 compression worker | `1be7624` | ledger/compression.rs | ✅ |
| T-117 Heartbeat TTL state machine | `bb4bf91` | ledger/compression.rs | ✅ |
| T-118 Lazy pool manager + idle TTL | `d77b79f` | ledger/pool.rs | ✅ |
| T-119 Write-path priority + queue lag | `caedc72` | ledger/store.rs, ingest.rs, health.rs | ✅ |

## Phase 2: Knowledge

| Task | SHA | Files | Status |
|------|-----|-------|--------|
| T-201 FTS summary query | `64fe186` | ledger/query.rs | ✅ |
| T-202 Session reconstruction | `9eacc53` | ledger/query.rs | ✅ |
| T-203 Manual record ingestion | `0bac4df` | ledger/ingest.rs | ✅ |
| T-204 ledger_query/session/record MCP | `8f728bf` | triumvirate/main.rs, daemon-http, mcp-bridge | ✅ |
| T-205 XML marker parser | `35332f6` | agent-adapter/markers.rs | ✅ |
| T-206 Prompt injection + parse rate | `eaf9ca9` | agent_exec.rs, main.rs | ✅ |
| T-207 Lesson CRUD + decay | `e21c5b5` | ledger/lessons.rs | ✅ |
| T-208 Lesson MCP tools | `d45a140` | triumvirate/main.rs, daemon-http, mcp-bridge | ✅ |
| T-209 Auto-lesson from summaries | `cffdb7c` | ledger/compression.rs | ✅ |

## Phase 3: Fleet Core

| Task | SHA | Files | Status |
|------|-----|-------|--------|
| T-301 Fleet crate scaffold | `fcf747b` | fleet/Cargo.toml, fleet/lib.rs | ✅ |
| T-302 Fleet/tasks/reviews tables | `0db435e` | ledger/store.rs | ✅ |
| T-303 Real GitOps impl | `8e7ad86` | triumvirate/git_ops_impl.rs | ✅ |
| T-304 Worktree lifecycle + dirty guard | `40afe9c` | fleet/worktree.rs | ✅ |
| T-305 Atomic task claiming + deps | `6e1d49b` | fleet/tasks.rs | ✅ |
| T-306 Fleet orchestrator + dry-run | `5564f91` | fleet/orchestrator.rs | ✅ |
| T-307 Fleet MCP tools | `47e4051` | triumvirate/main.rs | ✅ |
| T-308 Sequential merge | `d4f9ebe` | fleet/merge.rs | ✅ |
| T-309 Conflict detection + pause | `ecb30c8` | fleet/merge.rs | ✅ |
| T-310 Crash recovery | `07ac27b` | fleet/recovery.rs | ✅ |

## Phase 4: Review

| Task | SHA | Files | Status |
|------|-----|-------|--------|
| T-401 Peer-review crate scaffold | `fc394e8` | peer-review/Cargo.toml, peer-review/lib.rs | ✅ |
| T-403 Assignment + queue + timeout | `ef59a98` | peer-review/lib.rs | ✅ |
| T-404 Review MCP tools | `21c1983` | triumvirate/main.rs | ✅ |
| T-405 Review gate in fleet merge | `9ce04c9` | fleet/merge.rs | ✅ |
| T-406 Skip review env flag | `bd1927a` | fleet/merge.rs | ✅ |
| T-407 Mandatory peer review mode | `580a2c5` | agent_exec.rs, main.rs | ✅ |

## Phase 5: Dashboard

| Task | SHA | Files | Status |
|------|-----|-------|--------|
| T-502 Svelte scaffold | `ff564ac` | dashboard/* (package.json, svelte.config.js, etc.) | ✅ |
| T-503 WebSocket endpoint | `0257ea7` | triumvirate/main.rs | ✅ |
| T-504 Daemon-only ask_agent | `af596a3` | triumvirate/main.rs | ✅ |
| T-505 Sessions view | `e9b084c` | dashboard/sessions/+page.svelte, stores/agents.ts | ✅ |
| T-506 Fleet kanban view | `478689f` | dashboard/fleet/+page.svelte, stores/fleet.ts | ✅ |
| T-507 Ledger search + health view | `8e67503` | dashboard/ledger/+page.svelte, stores/ledger.ts | ✅ |
| T-508 Lessons confidence view | `1abddb4` | dashboard/lessons/+page.svelte, stores/lessons.ts | ✅ |
| T-509 Reviews view | `426b519` | dashboard/reviews/+page.svelte, stores/reviews.ts | ✅ |
| T-510 Metrics view | `110e8fb` | dashboard/metrics/+page.svelte | ✅ |
| T-511 rust-embed static assets | `914d2e6` | triumvirate/Cargo.toml, main.rs | ✅ |

## Phase 6: Enrichment + Codex

| Task | SHA | Files | Status |
|------|-----|-------|--------|
| T-601 OutboxEvent fields | `9a9db0d` | shared-types/lib.rs | ✅ |
| T-602 Populate enrichment fields | `d8c28ee` | agent_exec.rs, main.rs | ✅ |
| T-603 CodexAppServerParser | `16b929b` | agent-adapter/codex_app_server.rs | ✅ |
| T-604 Approval detection + probe | `2df3490` | agent-adapter/codex_app_server.rs | ✅ |
| T-605+T-606 Auto-approve + audit | `a3cbc28` | agent_exec.rs, main.rs | ✅ |

## Phase 7: GC

| Task | SHA | Files | Status |
|------|-----|-------|--------|
| T-701+T-702+T-703 GC + tool + startup | `a8f8495` | ledger/gc.rs, main.rs, daemon-http, mcp-bridge | ✅ |

## Fixup Commits

| SHA | Description |
|-----|-------------|
| `da7fabd` | Align outbox roundtrip test with enriched event DTO |

---

## Summary

| Metric | Value |
|--------|-------|
| Tasks planned | 71 |
| Tasks completed | 70 (T-110 bash hooks not in git) |
| Tasks combined | 3 (T-605+606, T-701+702+703 done in single commits) |
| Total commits | 63 |
| Fixup commits | 1 |
| Deviations documented | 0 (DEVIATION_LOG not populated) |

## Notes

- **T-110 (bash hooks):** Not present in git history. Hooks live at `~/.claude/hooks/` which is outside the repo. May have been written but not committed, or may need to be created separately.
- **BUILD_MANIFEST was not populated during build.** This was reconstructed from `git log` post-build. Future Codex handoffs must explicitly require per-task BUILD_MANIFEST updates.
- **DEVIATION_LOG was not populated.** No deviations were logged. Either Codex followed the plan exactly, or deviations occurred without documentation.
