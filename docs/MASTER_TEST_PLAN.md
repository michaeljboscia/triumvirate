# Master Test Plan — Triumvirate v3.1.0 + v3.2.0

**Purpose:** Comprehensive test plan covering ALL shipped functionality across both sprints. Organized by layer (unit → integration → system → acceptance). Designed to be executable — every test has a concrete command or procedure.

**Current test count:** 5 (nearly zero coverage)
**Target after this plan:** 150+ tests across all layers

---

## Layer 1: Unit Tests (cargo test, per-crate)

These run fast, no daemon needed, no network, no agent processes.

### 1.1 daemon-core (metrics + observability)

```
cargo test -p daemon-core --manifest-path daemon/Cargo.toml
```

| ID | Test | What it proves |
|----|------|---------------|
| U-DC-01 | `DaemonMetrics::new()` succeeds | All 20 metrics (12 original + 8 ABE) create + register without panic |
| U-DC-02 | `DaemonMetrics` fields are public and incrementable | Each counter/gauge/histogram field is accessible: `.inc()`, `.set()`, `.observe()` |
| U-DC-03 | `ObservabilityBus::new()` constructs | Bus wraps `Arc<DaemonMetrics>` + `broadcast::Sender` |
| U-DC-04 | `ObservabilityBus` is Clone+Send+Sync | Clone bus into 2 tokio::spawn tasks, both access metrics |
| U-DC-05 | `publish_event()` round-trips through broadcast channel | Publish "test" event, receiver gets JSON with type + ts_ms + payload |
| U-DC-06 | `publish_event()` N-value assertion | Two threads publish events with N=1 and N=2, receiver gets both with correct N values |
| U-DC-07 | `Arc<DaemonMetrics>` shared increment | Two threads both call `.agent_requests_total.inc()`, final `.get()` == 2 |
| U-DC-08 | `version::VERSION` matches Cargo.toml | `env!("CARGO_PKG_VERSION")` equals the workspace version |
| U-DC-09 | Placeholder traits exist | SessionStore, AgentExecutor, TaskTrackerHandle, LedgerStoreFactory are importable |

### 1.2 mcp-tools/aliases (parameter mapping)

```
cargo test -p mcp-tools --manifest-path daemon/Cargo.toml
```

| ID | Test | What it proves |
|----|------|---------------|
| U-AL-01 | `map_spawn_daemon_params` gemini → agent "gemini" | target "gemini" maps correctly |
| U-AL-02 | `map_spawn_daemon_params` codex → agent "codex" | target "codex" maps correctly |
| U-AL-03 | `map_spawn_daemon_params` claude → error | strict enum rejects "claude" |
| U-AL-04 | `map_spawn_daemon_params` empty target → error | missing target is caught |
| U-AL-05 | `map_ask_daemon_params` preserves gd_ prefix | daemon_id "gd_abc" → name "gd_abc" |
| U-AL-06 | `map_ask_daemon_params` preserves cd_ prefix | daemon_id "cd_xyz" → name "cd_xyz" |
| U-AL-07 | `map_ask_daemon_params` invalid prefix → error | daemon_id "xx_bad" rejected |
| U-AL-08 | `map_dismiss_daemon_params` drops hard param | hard=true accepted but dropped (tracing::warn logged) |
| U-AL-09 | `map_send_message_params` synchronous mapping | Returns AskSessionRequestLike, not a job_id |
| U-AL-10 | `map_get_response_params` returns deprecation shim | Always returns deprecation message string |
| U-AL-11 | `map_list_jobs_params` accepts target filter | target "gemini" passes, "invalid" errors |
| U-AL-12 | `map_write_scratchpad_params` owner from gd_ prefix | daemon_id "gd_session_xyz" → owner starts with "gemini-" |
| U-AL-13 | `map_write_scratchpad_params` owner from cd_ prefix | daemon_id "cd_session_abc" → owner starts with "codex-" |
| U-AL-14 | `map_write_scratchpad_params` explicit owner field | No daemon_id, owner="custom" → owner="custom" |
| U-AL-15 | `map_write_scratchpad_params` default owner | No daemon_id, no owner → owner="inter-agent" |
| U-AL-16 | `map_write_scratchpad_params` cwd is optional | cwd: None accepted |
| U-AL-17 | `map_code_review_params` passes through cwd/uncommitted/base_branch/commit_sha | All 4 fields passthrough correctly |
| U-AL-18 | `map_code_review_params` no diff/context fields | Output struct does NOT have diff or context |
| U-AL-19 | `map_list_scratchpad_params` cwd passthrough | The only field passes through |
| U-AL-20 | All 10 functions return the correct type | Type-check that each map_* returns the expected Result variant |

### 1.3 token-economics (storage + scanner + attribution)

```
cargo test -p token-economics --manifest-path daemon/Cargo.toml
```

| ID | Test | What it proves |
|----|------|---------------|
| U-TE-01 | `open()` creates DB with WAL mode | Fresh temp DB, verify PRAGMA journal_mode = wal |
| U-TE-02 | Schema creates 3 tables | token_records, scan_state, price_table all exist |
| U-TE-03 | `insert_record` + `query_summary` round-trip | Insert 3 records (1 per agent), query returns all 3 |
| U-TE-04 | `query_summary` filters by agent | Insert claude + codex records, query agent=claude returns 1 |
| U-TE-05 | `query_summary` filters by time range | Insert records at different timestamps, query since/until narrows correctly |
| U-TE-06 | `scan_claude_file` parses mock JSONL | Create temp file with 3 known-format lines, scan returns 3 TokenRecords |
| U-TE-07 | `scan_codex_file` parses mock JSONL | Same pattern for Codex format |
| U-TE-08 | `scan_gemini_chat_file` parses mock JSON | Gemini chat file format |
| U-TE-09 | `scan_gemini_telemetry_file` parses mock telemetry | Small telemetry.jsonl with thoughtsTokenCount |
| U-TE-10 | Scanner is incremental by mtime | Scan file, scan again without changes, second returns empty |
| U-TE-11 | `attribute_records` matches session IDs | 3 records + 2 matching outbox entries → 2 attributed, 1 unattributed |
| U-TE-12 | `attribute_records` calculates cost | Record with 1000 input tokens + price $10/MTok → cost_usd = 0.01 |
| U-TE-13 | `attribute_records` unmatched → "unattributed" | Record with no matching outbox entry gets build_id "unattributed" |
| U-TE-14 | `record_daemon_tokens` direct write | Call directly, verify record in DB |
| U-TE-15 | `by_build_query` returns task-level breakdown | Insert records with build_id + task_id, query returns grouped |
| U-TE-16 | `by_session_query` returns session breakdown | Insert records with session_id, query returns that session |
| U-TE-17 | Price table temporal lookup | Insert price with effective_date, query at that date returns correct price |
| U-TE-18 | Empty DB queries return empty, not error | All query functions on fresh DB return Ok(empty), not Err |

### 1.4 shared-types (TokenUsage)

| ID | Test | What it proves |
|----|------|---------------|
| U-ST-01 | `TokenUsage` has thinking_tokens field | Field exists as Option<u64> |
| U-ST-02 | `TokenUsage` has latency_ms field | Field exists as Option<u64> |
| U-ST-03 | `TokenUsage` has tool_calls field | Field exists as Option<u64> |
| U-ST-04 | `TokenUsage` serializes/deserializes with new fields | serde round-trip preserves all fields including new ones |

### 1.5 ledger

| ID | Test | What it proves |
|----|------|---------------|
| U-LE-01 | `LedgerStore::open()` creates DB | Fresh temp dir, open succeeds |
| U-LE-02 | `health()` returns healthy on empty DB | No events, health reports ok |
| U-LE-03 | Ingest + query round-trip | Insert an event, query returns it |
| U-LE-04 | GC removes old events | Insert old event, gc, verify removed |
| U-LE-05 | Lessons CRUD | add_lesson → query_lessons → validate_lesson cycle |

### 1.6 ABE task_tracker

| ID | Test | What it proves |
|----|------|---------------|
| U-TT-01 | `register` creates Working task | register → get_status returns Working |
| U-TT-02 | `mark_completed` transitions to Completed | register → mark_completed → get_status returns Completed with SHA |
| U-TT-03 | `mark_failed` transitions to Failed | register → mark_failed → get_status returns Failed with error message |
| U-TT-04 | `mark_timeout` transitions to Timeout | register → mark_timeout → get_status returns Timeout |
| U-TT-05 | `mark_stuck` transitions to Stuck | register → mark_stuck → get_status returns Stuck |
| U-TT-06 | Double-transition blocked | mark_completed → mark_failed returns AlreadyTerminal |
| U-TT-07 | Unknown task returns NotFound | mark_completed on non-existent ID returns NotFound |
| U-TT-08 | `cancel` terminates working task | register → cancel → get_status returns Cancelled |

---

## Layer 2: Integration Tests (requires daemon running)

These need the daemon process + potentially agent processes.

```
# Start daemon first
triumvirate daemon &
# Run integration tests
cargo test -p triumvirate --manifest-path daemon/Cargo.toml -- --ignored
```

### 2.1 MCP Tool Integration

| ID | Test | What it proves |
|----|------|---------------|
| I-MCP-01 | `ping` returns "pong" | MCP bridge is alive |
| I-MCP-02 | `daemon_health` returns status ok | Daemon HTTP health reachable from MCP |
| I-MCP-03 | `list_sessions` returns array | Session listing works |
| I-MCP-04 | `get_status` returns daemon state | active_sessions, daemon_mode present |
| I-MCP-05 | `spawn_session` + `ask_session` + `dismiss_session` lifecycle | Full session lifecycle with real Codex |
| I-MCP-06 | `spawn_session` with Gemini agent | Gemini session creates successfully |
| I-MCP-07 | `ledger_health` returns healthy | Ledger DB accessible from MCP |
| I-MCP-08 | `memory_write` + `memory_read` round-trip | Write entry, read back, content matches |
| I-MCP-09 | `lesson_add` + `lesson_list` round-trip | Add lesson, list includes it |
| I-MCP-10 | `outbox_recent` returns events | Recent outbox events listed |
| I-MCP-11 | `fallback_list` returns tickets | Dead-drop tickets listed |

### 2.2 Alias Integration

| ID | Test | What it proves |
|----|------|---------------|
| I-ALIAS-01 | `spawn_daemon(target:"codex")` creates session | Alias works end-to-end |
| I-ALIAS-02 | `ask_daemon(daemon_id:"cd_test")` gets response | Ask alias with prefix preservation |
| I-ALIAS-03 | `dismiss_daemon(daemon_id:"cd_test")` removes session | Dismiss alias |
| I-ALIAS-04 | `send_message(target:"codex",question:"echo test")` returns response directly | Synchronous — no job_id |
| I-ALIAS-05 | `get_response(job_id:"anything")` returns deprecation | Static deprecation string |
| I-ALIAS-06 | `list_daemons` returns sessions | List alias |
| I-ALIAS-07 | `write_scratchpad` + `list_scratchpad` cycle | Scratchpad alias round-trip |

### 2.3 ABE Integration

| ID | Test | What it proves |
|----|------|---------------|
| I-ABE-01 | `dispatch_codex_worktree` with trivial task | Worker spawns, commits, sentinel written, task marked completed |
| I-ABE-02 | `get_task_status` returns working then completed | Status transitions tracked by daemon |
| I-ABE-03 | `get_task_output` returns commit SHA + files | Output available after completion |
| I-ABE-04 | `cancel_task` stops a running task | Task cancelled, worker killed |
| I-ABE-05 | Sentinel file triggers completion detection | .triumvirate/TASK_COMPLETE.json → daemon marks completed |
| I-ABE-06 | Three-ceremony closing block works end-to-end | Commit + sentinel + HTTP POST all fire |
| I-ABE-07 | Worker timeout fires after task_timeout_sec | Dispatch with 10s timeout, worker sleeps 30s → timeout |
| I-ABE-08 | Contract validation rejects invalid commit format | Worker commits with wrong message → validation fails |

### 2.4 HTTP Route Integration

| ID | Test | What it proves |
|----|------|---------------|
| I-HTTP-01 | `GET /health` returns version | {"status":"ok","version":"3.2.0"} |
| I-HTTP-02 | `GET /status` returns daemon state | active_sessions, supported_agents |
| I-HTTP-03 | `GET /metrics` returns Prometheus text | Contains triumvirate_ prefix metrics |
| I-HTTP-04 | `GET /ws` WebSocket connects | Connection established, events received |
| I-HTTP-05 | `POST /ask-agent` returns agent response | Codex responds to echo command |
| I-HTTP-06 | `GET /ledger/health` returns healthy | Ledger accessible via HTTP |
| I-HTTP-07 | `POST /memory/write` + `POST /memory/read` | HTTP round-trip for memory |
| I-HTTP-08 | `GET /api/tokens/summary` returns data | Token summary from SQLite |
| I-HTTP-09 | `GET /api/tokens/by-build?build_id=test` | Build cost breakdown |
| I-HTTP-10 | `GET /api/tokens/by-session?session_id=test` | Session token breakdown |
| I-HTTP-11 | `POST /abe/task-complete` with valid payload | 200 OK, task marked completed |
| I-HTTP-12 | `POST /abe/task-complete` without auth | 401 Unauthorized |
| I-HTTP-13 | `POST /abe/task-complete` with empty body | 422 Unprocessable |
| I-HTTP-14 | Bearer token required on all routes | Unauthenticated request → 401 |
| I-HTTP-15 | Dashboard root serves HTML | `GET /` returns HTML with dashboard content |

### 2.5 Token Economics Integration

| ID | Test | What it proves |
|----|------|---------------|
| I-TOK-01 | `get_token_summary` MCP tool returns data | Token data accessible from Claude session |
| I-TOK-02 | `get_build_cost` MCP tool returns data | Build cost accessible from Claude session |
| I-TOK-03 | Scanner detects new session file | Create mock JSONL in watched dir → token_update WS event |
| I-TOK-04 | Direct write from ask_agent populates DB | Call ask_agent → token_records table has new entry |
| I-TOK-05 | Startup reconciliation runs without blocking | Daemon starts, /health responds within 5s |

---

## Layer 3: Observability Verification (requires daemon + real dispatch)

These tests verify the v3.2.0 instrumentation actually produces output.

| ID | Test | What it proves |
|----|------|---------------|
| O-SPAN-01 | ABE dispatch produces trace spans | `RUST_LOG=trace`, dispatch task, log has nested spans with task_id |
| O-SPAN-02 | Ledger operations produce trace spans | Write to ledger, log has spans with event_type |
| O-SPAN-03 | Agent request has parent span | ask_agent call, log shows agent + session_id in span |
| O-MET-01 | /metrics shows non-zero ABE counters after dispatch | abe_task_dispatch_total > 0 |
| O-MET-02 | /metrics shows non-zero agent_tokens after ask_agent | agent_tokens_total > 0 |
| O-MET-03 | /metrics shows non-zero abe_task_duration after completion | Histogram has observations |
| O-MET-04 | /metrics shows worktree_setup_duration after dispatch | Histogram has observations |
| O-LOG-01 | Structured log entry for dispatch start | JSON entry with task_id, wave, "abe_dispatch_start" |
| O-LOG-02 | Structured log entry for task completion | JSON entry with commit_sha, duration_ms, "abe_task_completed" |
| O-LOG-03 | Warning logged on timeout | WARN level entry with task_id, timeout_sec |
| O-LOG-04 | Error context includes file paths | Force an I/O error, error message includes the path + operation |
| O-WS-01 | abe_task_state event on dispatch | WS client receives event with status "dispatched" |
| O-WS-02 | abe_task_state event on completion | WS client receives event with status "completed" + commit_sha |
| O-WS-03 | token_update event after scan | WS client receives event with agent + tokens_added |

---

## Layer 4: System / Acceptance Tests (full end-to-end)

| ID | Test | What it proves |
|----|------|---------------|
| S-E2E-01 | Fresh install: build from source, start daemon, ping | A stranger can use this |
| S-E2E-02 | `triumvirate --version` prints 3.2.0 | Version correct |
| S-E2E-03 | `triumvirate doctor` reports ready | All health checks pass |
| S-E2E-04 | Full ABE dispatch: briefing → worker → commit → completion | The entire dispatch pipeline works |
| S-E2E-05 | Full session lifecycle: spawn → ask → dismiss | Multi-turn agent session works |
| S-E2E-06 | All 10 aliases callable from a Claude session | Backwards compatibility verified |
| S-E2E-07 | Dashboard accessible at / | Browser serves HTML |
| S-E2E-08 | WebSocket stream receives real events | Connect, trigger activity, events flow |
| S-E2E-09 | Token summary reflects real agent usage | After ask_agent call, /api/tokens/summary shows data |
| S-E2E-10 | Daemon survives 24h uptime | No memory leaks, no crash, metrics still updating |

---

## Layer 5: Regression (existing functionality preserved)

| ID | Test | What it proves |
|----|------|---------------|
| R-REG-01 | Existing ABE stress tests pass | `cargo test -p triumvirate abe_red_team` |
| R-REG-02 | Fleet operations work | fleet_spawn → fleet_status → fleet_cancel |
| R-REG-03 | Peer review works | review_request → review_submit → review_status |
| R-REG-04 | Gemini query works | query_gemini returns response |
| R-REG-05 | Scratchpad persists across daemon restart | Write scratchpad, restart daemon, read back |
| R-REG-06 | Memory entries persist | Write memory, restart, read back |
| R-REG-07 | Ledger events survive restart | Ingest event, restart, query returns it |

---

## Execution Priority

**Phase 1 (immediate — blocks v3.2.0 closure):**
- All Layer 1 unit tests (U-*): implement as `#[test]` / `#[tokio::test]` in each crate
- Estimated: 80 tests, 2-3 hours to write

**Phase 2 (next session):**
- Layer 2 integration tests (I-*): implement as `#[ignore]` tests that require running daemon
- Estimated: 40 tests, 2-3 hours

**Phase 3 (ongoing):**
- Layer 3 observability verification (O-*): manual or scripted
- Layer 4 system tests (S-*): manual acceptance
- Layer 5 regression (R-*): ensure existing tests still pass

**Total test target:** ~150 tests
**Current:** 5
**Gap:** 145 tests to write

---

## How the New Instrumentation Helps

With v3.2.0's tracing spans + structured logging, every test failure now produces:

1. **Structured JSON in daemon.log** with the exact span tree (which function, which task_id, what happened)
2. **Prometheus metrics** showing counters + histograms at the exact moment of failure
3. **WebSocket events** timestamped to millisecond precision
4. **Error context** with file paths, operation names, and task IDs on every anyhow chain

Set `RUST_LOG=triumvirate=debug` and every test failure tells you exactly where in the call chain it broke. No more "it failed somewhere in the ABE pipeline." The traces tell you: "it failed at worktree_setup.rs:47 trying to read contract.json for task T-104 because the file didn't exist."

That's what observability is for.
