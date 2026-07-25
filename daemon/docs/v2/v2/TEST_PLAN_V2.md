# TEST PLAN — Triumvirate v2 (MCP-First Rewrite)

**Version:** 2.0
**Date:** 2026-04-05
**Traces to:** SPEC_FINAL.md (user stories + parity checklist), GOATRODEO_LEDGER.md (decisions)
**Built by:** Claude (scaffold) + Codex (flesh out)

**2026-04-06 Update:** `ask_twins` tests are legacy references. Active fan-out validation now uses explicit session flow (`spawn_session` + parallel `ask_session` + `dismiss_session`).

---

## Part 1: REQ → User Story → Test Traceability

Every test traces to a user story. Every user story traces to a job. No orphans.

| US | Job (Functional) | Job (Emotional) | Tests |
|----|-----------------|-----------------|-------|
| US-1: Ask the twins | Get answers from multiple agents | Confident they're working | T-001 through T-012 |
| US-2: Direct routing | Send task to the right specialist | In control of who does what | T-013 through T-016 |
| US-3: Plain conversation | Normal Claude experience | Tools serve me, not the other way | T-017 through T-019 |
| US-4: Failure visibility | Know what broke and fix it | Calm, not anxious | T-020 through T-035 |
| US-5: Fleet orchestration | Parallel agent work on codebase | Ambitious about what I can tackle | T-036 through T-039 |
| US-6: Dashboard | See all agent activity in one place | Informed and in control | T-040 through T-044 |
| US-7: Zero ceremony | System ready when I am | Tools serve me | T-045 through T-049 |
| US-8: Persistent sessions | Ongoing conversations with context | Working WITH someone | T-050 through T-056 |
| US-9: Proactive contribution | Agents surface relevant insights unprompted | Team has my back | **FUTURE — Tier 2+. No tests until in scope.** |

---

## Part 2: Core Path Tests (US-1, US-2, US-3)

### T-001: ask_twins happy path
```
SETUP: daemon running, Gemini + Codex sessions alive
ACTION: Claude calls ask_twins("What is 2+2?", cwd="/tmp")
EXPECT: 
  - Progress: "→ Gemini: sent ✓" within 1s
  - Progress: "→ Codex: sent ✓" within 1s
  - Progress: "→ [agent]: working..." with elapsed time
  - Progress: "→ [agent]: responded ✓" for each
  - Result contains both agent responses
  - Result includes role-adapted prompt evidence (Gemini got research framing, Codex got implementation framing)
PASS: both agents respond, all lifecycle events emitted
```

### T-002: ask_twins non-blocking return order
```
SETUP: mock-gemini configured with 5s delay, mock-codex with 1s delay
ACTION: Claude calls ask_twins("test")
EXPECT:
  - Codex progress events appear first
  - Gemini progress events appear second
  - Both results returned in single tool response
  - Total time ~5s (not 6s — parallel, not sequential)
PASS: responses return in completion order, not call order
```

### T-003: ask_twins default prompt passthrough
```
SETUP: daemon running with mock CLIs that echo received prompt
ACTION: Claude calls ask_twins("Add authentication to the API")
EXPECT:
  - Gemini received the raw user prompt unchanged
  - Codex received the raw user prompt unchanged
PASS: default behavior is passthrough; no hidden role-twisting
```

### T-004: ask_twins synthesis
```
SETUP: mock-gemini returns "use JWT", mock-codex returns "use session tokens"
ACTION: Claude calls ask_twins("best auth approach?")
EXPECT:
  - Result includes both raw responses
  - Claude (the orchestrator) evaluates quality and flags the disagreement
PASS: disagreement surfaced, not blindly merged
```

### T-005: ask_twins reuses alive session
```
SETUP: Gemini session already alive from previous ask
ACTION: Claude calls ask_twins("follow-up question")
EXPECT:
  - Lifecycle shows "reused" not "spawned" for Gemini
  - No new process created for Gemini
  - Codex may be "spawned" if first use
PASS: session_source: reused in lifecycle events
```

### T-005b: ask_twins + session tools share backend
```
SETUP: spawn_session("gemini", "shared"), ask_session("shared", "prime")
ACTION: ask_twins("follow-up", cwd=same)
EXPECT:
  - Gemini lifecycle indicates reused persistent worker
PASS: ask_twins/session tools hit same persistent backend
```

### T-006: ask_twins with one agent down
```
SETUP: Gemini alive, Codex process killed
ACTION: Claude calls ask_twins("test")
EXPECT:
  - Gemini responds normally
  - Codex shows FAILED after 3 retries
  - Gemini's result returned immediately (non-blocking)
  - Dead drop launched for Codex
  - Claude told: "Gemini answered. Codex failed. Fallback launched."
PASS: partial results returned, failure visible, fallback triggered
```

### T-007: ask_agent happy path (Gemini)
```
SETUP: daemon running
ACTION: Claude calls ask_agent("gemini", "research JWT best practices", cwd, repo, branch)
EXPECT:
  - All lifecycle events for Gemini only
  - No Codex involvement
  - cwd/repo/branch passed through to agent context
PASS: single agent responds with lifecycle
```

### T-008: ask_agent happy path (Codex)
```
SETUP: daemon running
ACTION: Claude calls ask_agent("codex", "implement auth module", cwd, repo, branch)
EXPECT: same as T-007 but for Codex
PASS: single agent responds with lifecycle
```

### T-009: ask_agent context passing
```
SETUP: daemon running with mock CLI that echoes received context
ACTION: Claude calls ask_agent("gemini", "question", cwd="/Users/you/projects/triumvirate", repo="triumvirate", branch="feat/mcp-first")
EXPECT:
  - Agent receives project context matching passed parameters
  - Outbox log contains cwd, repo, branch
PASS: context flows from Claude → bridge → daemon → agent
```

### T-010: ask_twins activity stages
```
SETUP: mock CLIs configured to emit stage transitions
ACTION: Claude calls ask_twins("test")
EXPECT:
  - Progress includes activity stages: bootstrapping → reading files → drafting → finalizing
  - Stages are coarse, not full chain-of-thought
PASS: at least 2 distinct stages visible per agent
```

### T-011: ask_twins with Bearer auth
```
SETUP: daemon running with auth token
ACTION: bridge sends request WITHOUT Bearer token
EXPECT: 401 Unauthorized
ACTION: bridge sends request WITH correct Bearer token
EXPECT: normal response
PASS: unauthenticated requests rejected
```

### T-012: ask_twins concurrent from multiple bridges
```
SETUP: daemon running, 2 MCP bridges connected
ACTION: both bridges call ask_twins simultaneously
EXPECT:
  - Both requests queued (FIFO)
  - Both get responses
  - No race conditions, no corrupted state
  - Outbox logs show 2 distinct request_ids
PASS: concurrent access works without corruption
```

### T-013: explicit trigger — "@gemini" routes correctly
```
ACTION: Claude receives "@gemini research auth"
EXPECT: Claude calls ask_agent with target=gemini
PASS: explicit trigger recognized
```

### T-014: explicit trigger — "ask the twins" routes correctly
```
ACTION: Claude receives "ask the twins about caching"
EXPECT: Claude calls ask_twins
PASS: explicit trigger recognized
```

### T-015: explicit trigger — "/send-to-siblings" skill alias
```
ACTION: user invokes /send-to-siblings with a message
EXPECT: routes to ask_twins MCP tool (not old TS path)
PASS: skill is an alias to MCP tool
```

### T-016: no false positive routing
```
ACTION: Claude receives "what do you think about auth?"
EXPECT: Claude handles directly, NO MCP call to daemon
PASS: no daemon involvement without explicit trigger
```

### T-017: plain conversation unaffected
```
SETUP: daemon running, MCP bridge connected
ACTION: Claude receives "explain how async/await works in Rust"
EXPECT:
  - Claude responds directly
  - No MCP tool calls made
  - Response latency within 10ms of session without MCP server registered
PASS: MCP presence is invisible for non-triggered messages
```

### T-018: plain conversation latency gate
```
SETUP: measure response latency for 10 plain messages WITH MCP registered
COMPARE: measure same 10 messages WITHOUT MCP registered
EXPECT: delta <= 10ms p95
PASS: release blocker if exceeded
```

### T-019: plain conversation with daemon down
```
SETUP: daemon NOT running, MCP bridge fails to connect
ACTION: Claude receives plain message
EXPECT: Claude responds normally (no daemon needed for plain conversation)
PASS: daemon failure doesn't affect non-triggered messages
```

---

## Part 3: Failure Path Tests (US-4)

### T-020: agent timeout → visible retry
```
SETUP: mock-gemini configured to never respond
ACTION: Claude calls ask_agent("gemini", "test")
EXPECT:
  - Progress: "→ Gemini: working..." with elapsed timer
  - At timeout: "→ Gemini: TIMEOUT after 300s. Sending SIGTERM."
  - "→ Gemini: retrying (1/3)..."
  - "→ Gemini: retrying (2/3)..."
  - "→ Gemini: retrying (3/3)..."
  - "→ Gemini: FAILED after 3 attempts."
PASS: every retry visible, failure surfaced with error detail
```

### T-021: retryable error triggers auto-retry
```
SETUP: mock-gemini returns "stream disconnected" on first attempt, succeeds on second
ACTION: Claude calls ask_agent("gemini", "test")
EXPECT:
  - First attempt fails
  - "→ Gemini: retrying (1/3)..."
  - Second attempt succeeds
  - "→ Gemini: responded ✓"
PASS: retryable error detected, auto-retry works, user sees both attempts
```

### T-022: jittered backoff timing
```
SETUP: mock-gemini always fails
ACTION: Claude calls ask_agent("gemini", "test")
EXPECT:
  - Retry 1 at ~250ms after failure
  - Retry 2 at ~1s after retry 1
  - Retry 3 at ~2s after retry 2
PASS: backoff timing matches spec (250ms, 1s, 2s ± jitter)
```

### T-023: SIGTERM/SIGKILL chain
```
SETUP: mock-gemini configured to ignore SIGTERM
ACTION: trigger timeout
EXPECT:
  - SIGTERM sent at timeout
  - 5s grace period
  - SIGKILL sent after grace
  - Process confirmed dead
  - "→ Gemini: process killed after SIGTERM grace period"
PASS: escalation chain works
```

### T-024: dead drop trigger after 3 failures
```
SETUP: mock-gemini always fails
ACTION: Claude calls ask_agent("gemini", "important question")
EXPECT:
  - 3 retries all fail
  - Dead drop file written: {date}_{time}_gemini_{project}_{hash}.md
  - Terminal window spawned via osascript
  - PID captured in daemon state
  - Claude told: "Fallback launched in Terminal."
PASS: dead drop triggered, file exists, Terminal opened, PID tracked
```

### T-025: dead drop file content
```
SETUP: trigger dead drop
EXPECT dead drop file contains:
  - Timestamp
  - Project path
  - Branch
  - Agent target
  - Full question/prompt
  - Files to read (if applicable)
  - Expected output format
  - Response file path
PASS: another agent could answer this with zero additional context
```

### T-026: dead drop response detected
```
SETUP: dead drop triggered, agent writes response file
ACTION: Claude makes any subsequent MCP tool call
EXPECT:
  - Bridge detects completed response file
  - Returns: "A previous fallback completed. Gemini answered your question about X."
  - Response content available
PASS: fallback result surfaced without user having to ask
```

### T-027: dead drop GC — completed pairs
```
SETUP: dead drop request + response files exist, both > 48h old
ACTION: daemon startup (or GC cycle)
EXPECT: both files deleted
PASS: completed pairs cleaned up after 48h
```

### T-028: dead drop GC — failed drops
```
SETUP: dead drop request file exists, no response, > 24h old
ACTION: GC cycle
EXPECT: flagged in diagnostic log
AFTER 7 DAYS: auto-deleted
PASS: failed drops flagged then cleaned
```

### T-029: dead drop Terminal window closed by user
```
SETUP: dead drop spawned, Terminal window open
ACTION: user closes Terminal window (kills agent process)
EXPECT:
  - PID no longer alive
  - No response file written
  - On Claude's next MCP call: "Fallback Codex session was killed before finishing. Want me to re-launch?"
PASS: orphaned process detected, user informed
```

### T-030: failure pattern detection
```
SETUP: same agent fails 3 times in one session
ACTION: next successful MCP call
EXPECT:
  - Warning injected: "Codex has failed its last 3 requests. Common error: [X]."
PASS: pattern detected and surfaced automatically
```

### T-031: daemon down — RED STATE
```
SETUP: daemon not running, bridge cannot connect
ACTION: Claude calls ask_twins
EXPECT:
  - Immediate error: "Triumvirate daemon is offline."
  - ONE auto-start attempt
  - If auto-start fails: exact error + "Twins reachable via direct spawn (slower)"
PASS: instant notification, not silent hang
```

### T-032: daemon mid-restart — visible retry
```
SETUP: daemon restarting (2s window of connection refused)
ACTION: Claude calls ask_agent during restart
EXPECT:
  - "→ daemon: connection refused, retrying (1/3)..."
  - Retry succeeds when daemon comes back
  - "→ daemon: reconnected"
PASS: restart is visible and recovered
```

### T-033: non-retryable error surfaces immediately
```
SETUP: daemon returns 400 Bad Request (malformed tool call)
ACTION: Claude calls ask_agent with invalid params
EXPECT:
  - Error returned immediately, no retry
  - Error message includes what was wrong
PASS: non-retryable errors don't waste time on retries
```

### T-034: agent process crash mid-response
```
SETUP: mock-gemini crashes after sending partial output
ACTION: Claude calls ask_agent
EXPECT:
  - Partial output NOT returned as complete
  - "→ Gemini: process exited unexpectedly"
  - Retry triggered
PASS: partial responses not treated as complete
```

### T-035: outbox logging on every request
```
SETUP: send 10 requests (mix of ask_twins, ask_agent, failures)
ACTION: query daemon SQLite outbox table
EXPECT:
  - 10 rows (one per request)
  - Each has: request_id, session_id, agent, tool, status, timestamp, duration, error (if any)
  - Indexed by request_id and session_id
PASS: complete audit trail
```

---

## Part 4: Session Tests (US-8)

### T-050: spawn_session
```
ACTION: Claude calls spawn_session("gemini", name="my-research", cwd="/projects/triumvirate")
EXPECT:
  - Session created, agent alive
  - Lifecycle: "→ Gemini session 'my-research': spawned ✓"
  - list_sessions shows it as active
PASS: session exists and is listed
```

### T-051: ask_session multi-turn
```
SETUP: session "my-research" alive
ACTION: ask_session("my-research", "what is JWT?")
ACTION: ask_session("my-research", "what about refresh tokens?")
EXPECT:
  - Second question builds on first (agent has context)
  - Lifecycle events for each turn
PASS: multi-turn context preserved
```

### T-052: dismiss_session
```
SETUP: session "my-research" alive
ACTION: dismiss_session("my-research")
EXPECT:
  - Session log written (taxonomy-compliant)
  - Agent process terminated
  - list_sessions no longer shows it
PASS: clean teardown + log
```

### T-053: sessions stay alive indefinitely
```
SETUP: session spawned
ACTION: wait 30 minutes (simulated or real)
EXPECT: session still alive, agent process still running
PASS: no TTL, no kill, no hibernate
```

### T-054: sessions machine-wide
```
SETUP: session spawned from Claude terminal 1 in project A
ACTION: Claude terminal 2 in project B calls list_sessions
EXPECT: session from terminal 1 is visible
ACTION: Claude terminal 2 calls ask_session on that session
EXPECT: works, agent responds
PASS: sessions shared across terminals
```

### T-055: Codex thread resume
```
SETUP: Codex session with 3 turns of history
ACTION: dismiss and re-spawn Codex for same workspace
EXPECT: 
  - Thread ID tracked in SQLite
  - New session resumes thread via codex exec resume <thread_id>
  - Context from previous turns preserved
PASS: Codex thread continuity across session lifecycle
```

### T-056: Gemini model fallback
```
SETUP: Gemini configured with primary model that returns quota error
ACTION: ask_agent("gemini", "test")
EXPECT:
  - Primary model fails
  - Fallback to next model in chain (--model flag)
  - Lifecycle shows: "→ Gemini: model fallback to gemini-2.5-pro"
  - Response returned from fallback model
PASS: model fallback chain works with visible transition
```

---

## Part 5: Fleet Tests (US-5) — Deferred

Scaffold only. Codex fleshes out when fleet enters scope.

### T-036: fleet_spawn creates agents with worktrees
### T-037: fleet task dependency blocking
### T-038: fleet headline events in Claude, detail in dashboard
### T-039: fleet sequential merge with conflict detection

---

## Part 6: Dashboard Tests (US-6) — Deferred

Scaffold only. Tests added when dashboard rebuild enters scope.

### T-040: dashboard serves at :8080
### T-041: dashboard shows MCP-path events (correlated by request_id)
### T-042: dashboard shows fabric events
### T-043: dashboard layout on 4K monitor (3840x2160)
### T-044: dashboard WebSocket auto-reconnect

---

## Part 7: Infrastructure Tests (US-7)

### T-045: triumvirate install
```
ACTION: run `triumvirate install` on clean state
EXPECT:
  - Binary built/installed
  - launchd plist generated and loaded
  - daemon.token created
  - MCP registered in ~/.claude.json
  - daemon health check passes
PASS: one command, everything works
```

### T-046: triumvirate doctor
```
SETUP: system fully installed
ACTION: run `triumvirate doctor`
EXPECT:
  - Daemon: running ✓
  - Auth token: valid ✓
  - MCP: registered ✓
  - Gemini CLI: found ✓
  - Codex CLI: found ✓
  - Claude CLI: found ✓
  - SQLite: accessible ✓
PASS: all checks pass with clear output
```

### T-047: launchd auto-start
```
SETUP: system installed, daemon not running
ACTION: reboot (or launchctl load)
EXPECT: daemon starts automatically
PASS: zero manual intervention
```

### T-048: MCP bridge auto-connect
```
SETUP: daemon running
ACTION: start new Claude session
EXPECT:
  - triumvirate MCP tools available in Claude
  - get_status returns healthy
PASS: bridge connects to daemon on session start
```

### T-049: MCP bridge with daemon down
```
SETUP: daemon NOT running
ACTION: start new Claude session
EXPECT:
  - Bridge attempts ONE auto-start
  - If fails: Claude informed "Triumvirate daemon offline: [error]"
  - Plain conversation still works (US-3 / T-019)
PASS: failure surfaced immediately, no silent hang
```

---

## Part 8: Reliability Baseline

### T-060: 1,000 synthetic request run
```
SETUP: daemon running with mock CLIs
ACTION: send 1,000 requests (mix: 40% ask_twins, 30% ask_agent, 20% spawn/ask/dismiss, 10% edge cases)
MEASURE:
  - Success rate (target: >84%, aspirational: >99%)
  - Failure rate by type
  - Response time p50, p95, p99
  - Retry rate
  - Dead drop trigger rate
  - Silent failure rate (target: 0%)
PASS: zero silent failures. Success rate and SLO set AFTER measurement.
```

### T-061: concurrent load
```
SETUP: 3 MCP bridges connected simultaneously
ACTION: each sends 100 requests
EXPECT:
  - All 300 requests processed
  - No corruption
  - No deadlocks
  - Response times within 2x of single-bridge baseline
PASS: concurrent access at realistic scale
```

---

## Part 9: Security Tests

### T-070: auth token required
```
ACTION: HTTP request to daemon without Bearer token
EXPECT: 401 Unauthorized
PASS: unauthenticated access blocked
```

### T-071: invalid auth token rejected
```
ACTION: HTTP request with wrong Bearer token
EXPECT: 401 Unauthorized
PASS: bad tokens rejected
```

### T-072: localhost only
```
ACTION: attempt connection from non-localhost IP
EXPECT: connection refused (daemon binds to 127.0.0.1 only)
PASS: no remote access
```

### T-073: dead drop path traversal
```
ACTION: attempt to write dead drop with "../" in filename
EXPECT: rejected, path sanitized
PASS: no escape from dead drop directory
```

---

## Part 10: Test Infrastructure

### Test Levels

| Level | What | When | Command |
|-------|------|------|---------|
| Unit | Individual functions, parsers, types | Every file change | `cargo test` |
| Integration | Multi-component with mock CLIs | Every increment | `cargo test --features mock` |
| E2E (mock) | Full daemon + bridge + mock CLIs | Every increment | `cargo test --test e2e` |
| E2E (live) | Full daemon + bridge + real CLIs | Before migration swap | Manual: type "ask the twins" in Claude |
| Reliability | 1,000 synthetic requests | Before migration swap | `cargo test --test reliability` |
| Security | Auth, localhost, path traversal | Before migration swap | `cargo test --features security` |

### Mock CLIs

Carry over from old daemon. Located in daemon-v2/crates/mocks/:
- mock-gemini: configurable delays, response scripts, error modes
- mock-codex: configurable delays, thread simulation, error modes
- mock-claude: configurable delays, stream-json output

### Test Harness

Layered (per R6 decision):
1. Unit tests for MCP protocol handling + routing logic
2. Integration tests with mock CLIs (daemon + bridge in-process)
3. E2E script that runs daemon + bridge as processes, sends MCP calls via stdio
4. Smoke test with real CLIs on fixed prompt set

---

## Pass/Fail Gate

**Before migration swap, ALL must pass:**
- TS parity checklist: all items checked
- T-001 through T-035: all pass (core + failure paths)
- T-050 through T-056: all pass (sessions)
- T-045 through T-049: all pass (infrastructure)
- T-060: reliability baseline measured, zero silent failures
- T-070 through T-073: all pass (security)

**Soft fail (ship-block unless waived):**
- T-018: latency gate (<=10ms delta)
- T-061: concurrent load
- Dashboard tests (deferred)
- Fleet tests (deferred)
