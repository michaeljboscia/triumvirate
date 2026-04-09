# v3.1 MCP Consolidation — Implementation Plan

**Spec:** `specs/MCP_CONSOLIDATION.md`
**PRD:** `docs/v3.1/PRD.md`
**Backend:** `docs/v3.1/BACKEND_STRUCTURE.md`

---

## Build Overview

- **5 Waves, 18 Tasks**
- Wave 0: Contracts (types, traits, interfaces)
- Wave 1: Extract MCP tool handlers from main.rs → mcp-tools modules
- Wave 2: Extract HTTP routes from main.rs → daemon-http + DaemonState to daemon-core
- Wave 3: Build aliases + update skills
- Wave 4: Front door swap + cleanup

**Build method:** ABE fleet dispatch (`dispatch_codex_worktree`). This is the dogfood run.
**max_parallel:** 7 (proven in stress test)
**Test command:** `cargo test --workspace`

---

## Wave 0: Contracts and Interfaces

<task id="T-001" req="REQ-B1,REQ-B2,REQ-B3" wave="0" depends="">
  <description>Define ObservabilityBus struct and module trait interfaces in shared-types or daemon-core</description>
  <files>daemon/crates/daemon-core/src/lib.rs, daemon/crates/shared-types/src/lib.rs</files>
  <scope_out>Do not implement metrics registration. Do not move DaemonMetrics yet — just define the ObservabilityBus struct shape and the trait interfaces each mcp-tools module will receive (SessionStore, AgentExecutor, TaskTrackerHandle, LedgerStoreFactory, etc.)</scope_out>
  <tools>cargo check --workspace</tools>
  <verify>cargo check --workspace</verify>
  <reality_test>Import ObservabilityBus from daemon-core in mcp-tools Cargo.toml — compiler accepts. Create a mock ObservabilityBus in a test — compiles. Instantiate each trait interface — compiles.</reality_test>
  <done_when>ObservabilityBus struct defined with metrics: Arc&lt;DaemonMetrics&gt; and ws_events: broadcast::Sender&lt;String&gt;. Module trait interfaces defined. All compile.</done_when>
</task>

<task id="T-002" req="REQ-A1,REQ-A2" wave="0" depends="">
  <description>Define alias parameter mapping types and the TS→Rust schema conversion functions</description>
  <files>daemon/crates/mcp-tools/src/aliases.rs</files>
  <scope_out>Do not register aliases with tool_router yet. Do not modify McpBridge. Types and mapping functions only.</scope_out>
  <tools>cargo check -p mcp-tools</tools>
  <verify>cargo check -p mcp-tools</verify>
  <reality_test>Call map_spawn_daemon_params with TS schema { target: "gemini", session_name: "x" } → returns Rust schema { agent: "gemini", name: "x" }. Call with { target: "codex" } → returns { agent: "codex" }. Call with { target: "claude" } → returns error (strict enum).</reality_test>
  <done_when>All 8 alias mapping functions defined and unit-tested. Parameter conversion for spawn_daemon, ask_daemon, dismiss_daemon, list_daemons, send_message, get_response, list_jobs, code_review.</done_when>
</task>

---

## Wave 1: Extract MCP Tool Handlers

All tasks in this wave extract existing code from main.rs into mcp-tools modules. ZERO behavioral change. Every function moves verbatim, only changing how it accesses shared state (from &self on McpBridge to narrowed trait interfaces).

<task id="T-003" req="REQ-C1" wave="1" depends="T-001">
  <description>Extract inter-agent tool handlers (spawn_session, ask_session, dismiss_session, list_sessions, ask_agent, get_status, daemon_health) from main.rs to mcp-tools/src/inter_agent.rs</description>
  <files>daemon/crates/mcp-tools/src/inter_agent.rs, daemon/crates/mcp-tools/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change any tool behavior. Do not modify tool schemas. Do not add new tools. Do not touch HTTP routes.</scope_out>
  <tools>cargo test --workspace, cargo check --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>Call spawn_session via MCP → session created. Call ask_session → response received. Call dismiss_session → session removed. All 7 tools produce identical output to pre-extraction.</reality_test>
  <done_when>7 tool handlers live in inter_agent.rs. main.rs no longer contains these functions. All existing tests pass.</done_when>
</task>

<task id="T-004" req="REQ-C1" wave="1" depends="T-001">
  <description>Extract ABE tool handlers (dispatch_codex, dispatch_codex_worktree, get_task_status, get_task_output, cancel_task) from main.rs to mcp-tools/src/abe.rs</description>
  <files>daemon/crates/mcp-tools/src/abe.rs, daemon/crates/mcp-tools/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change ABE behavior. Do not modify dispatch logic. Do not touch the abe/ module files (worktree_setup.rs, orchestrator.rs, etc.).</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>Call dispatch_codex_worktree via MCP → worktree created, Codex spawned. Call get_task_status → returns correct state. Identical behavior to pre-extraction.</reality_test>
  <done_when>5 ABE tool handlers live in abe.rs. main.rs no longer contains these functions. All tests pass.</done_when>
</task>

<task id="T-005" req="REQ-C1" wave="1" depends="T-001">
  <description>Extract fleet tool handlers (fleet_spawn, fleet_status, fleet_task_list, fleet_claim_task, fleet_cancel) from main.rs to mcp-tools/src/fleet.rs</description>
  <files>daemon/crates/mcp-tools/src/fleet.rs, daemon/crates/mcp-tools/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change fleet behavior. Do not modify fleet crate internals.</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>Call fleet_spawn via MCP → fleet created. Call fleet_status → returns members. Identical to pre-extraction.</reality_test>
  <done_when>5 fleet tool handlers live in fleet.rs. main.rs no longer contains them. All tests pass.</done_when>
</task>

<task id="T-006" req="REQ-C1" wave="1" depends="T-001">
  <description>Extract knowledge tool handlers (memory_*, scratchpad_*, outbox_*, fallback_*, ledger_*, lesson_*) from main.rs to mcp-tools/src/knowledge.rs</description>
  <files>daemon/crates/mcp-tools/src/knowledge.rs, daemon/crates/mcp-tools/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change any tool behavior. Do not modify ledger, fallback-outbox, or daemon-core crate internals.</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>Call ledger_query via MCP → returns events. Call scratchpad_write → persists. Call lesson_add → creates lesson. All 17 tools produce identical output.</reality_test>
  <done_when>17 knowledge tool handlers live in knowledge.rs. main.rs no longer contains them. All tests pass.</done_when>
</task>

<task id="T-007" req="REQ-C1" wave="1" depends="T-001">
  <description>Extract review + gemini query tool handlers from main.rs to mcp-tools/src/review.rs and mcp-tools/src/gemini_query.rs</description>
  <files>daemon/crates/mcp-tools/src/review.rs, daemon/crates/mcp-tools/src/gemini_query.rs, daemon/crates/mcp-tools/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change review or gemini query behavior. Do not modify peer-review crate.</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>Call review_request via MCP → review created. Call query_gemini → Gemini response received. All 5 tools produce identical output.</reality_test>
  <done_when>3 review handlers in review.rs, 2 gemini handlers in gemini_query.rs. main.rs no longer contains them. All tests pass.</done_when>
</task>

---

## Wave 2: Extract HTTP Routes + DaemonState

<task id="T-008" req="REQ-C2" wave="2" depends="T-003,T-004,T-005,T-006,T-007">
  <description>Extract all *_route HTTP handler functions from main.rs into daemon-http crate</description>
  <files>daemon/crates/daemon-http/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change HTTP route behavior. Do not modify Axum router setup (that stays in main.rs startup). Do not move WebSocket or dashboard routes in this task (T-009 handles those).</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>curl localhost:8080/health → 200. curl localhost:8080/ledger/health → valid JSON. curl POST /ask-agent → agent response. All 19 API routes produce identical output.</reality_test>
  <done_when>19 HTTP route handler functions live in daemon-http. main.rs no longer contains *_route functions (except ws, dashboard, metrics — see T-009). All tests pass.</done_when>
</task>

<task id="T-009" req="REQ-C2,REQ-B3" wave="2" depends="T-001,T-008">
  <description>Extract WebSocket handler, dashboard routes, metrics route, and DaemonState construction into daemon-http and daemon-core</description>
  <files>daemon/crates/daemon-http/src/lib.rs, daemon/crates/daemon-core/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not change WebSocket or metrics behavior. DaemonMetrics struct definition moves to daemon-core (or shared location). Axum Router construction stays in main.rs.</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>WebSocket connects to ws://localhost:8080/ws → receives bootstrap events. curl /metrics → Prometheus text format with all 12 metrics. Dashboard at / serves HTML.</reality_test>
  <done_when>ws_route, dashboard routes, metrics_route, DaemonMetrics, DaemonState, encode_ws_event, publish_ws_event all extracted. main.rs contains only startup wiring. All tests pass.</done_when>
</task>

<task id="T-010" req="REQ-C3,REQ-C4" wave="2" depends="T-008,T-009">
  <description>Verify main.rs is under 300 lines and contains only startup wiring</description>
  <files>daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not add new functionality. This is a verification + cleanup task only. Remove dead imports, unused helper functions, stale comments.</scope_out>
  <tools>wc -l daemon/crates/triumvirate/src/main.rs, cargo test --workspace</tools>
  <verify>cargo test --workspace && test $(wc -l < daemon/crates/triumvirate/src/main.rs) -lt 300</verify>
  <reality_test>wc -l main.rs reports < 300. grep for any async fn that isn't main/run_daemon/run_doctor/run_status — should find zero tool handlers or route handlers. cargo test --workspace passes.</reality_test>
  <done_when>main.rs is under 300 lines. Contains only: CLI parsing, config, tracing init, DaemonState build, McpBridge build, server spawns, shutdown. No tool handlers. No route handlers.</done_when>
</task>

---

## Wave 3: Aliases + Skill Updates

<task id="T-011" req="REQ-A1,REQ-A2,REQ-A3" wave="3" depends="T-002,T-003">
  <description>Register all 8 alias tools in the MCP tool_router with parameter mapping and logging</description>
  <files>daemon/crates/mcp-tools/src/aliases.rs, daemon/crates/mcp-tools/src/lib.rs</files>
  <scope_out>Do not modify canonical tool handlers. Do not change ~/.claude.json yet. Aliases are ADDITIONAL tools, not replacements.</scope_out>
  <tools>cargo test --workspace</tools>
  <verify>cargo test --workspace</verify>
  <reality_test>Call spawn_daemon via MCP → creates session (same as spawn_session). Call ask_daemon → gets response. Call send_message → synchronously calls ask_session, returns response (not job_id). Daemon log shows tracing::info with tool_alias field for each call.</reality_test>
  <done_when>All 8 aliases registered and callable via MCP. Parameter mapping works for all schema differences. Alias usage logged. get_response returns deprecation notice.</done_when>
</task>

<task id="T-012" req="REQ-J2" wave="3" depends="T-011">
  <description>Update send-to-codex skill to use mcp__triumvirate__ask_session</description>
  <files>~/.claude/skills/send-to-codex/SKILL.md</files>
  <scope_out>Do not change skill behavior or purpose. Only update tool references and remove send_message/get_response two-step pattern.</scope_out>
  <tools>cat ~/.claude/skills/send-to-codex/SKILL.md</tools>
  <verify>grep -c "mcp__triumvirate__ask_session" ~/.claude/skills/send-to-codex/SKILL.md</verify>
  <reality_test>Invoke /send-to-codex with a question → Codex responds via ask_session. No job_id in the flow. Response is direct.</reality_test>
  <done_when>Skill references mcp__triumvirate__ask_session. No references to mcp__inter-agent. No send_message/get_response pattern.</done_when>
</task>

<task id="T-013" req="REQ-J3" wave="3" depends="T-011">
  <description>Update send-to-gemini skill to use mcp__triumvirate__ask_session</description>
  <files>~/.claude/skills/send-to-gemini/SKILL.md</files>
  <scope_out>Same as T-012.</scope_out>
  <tools>cat ~/.claude/skills/send-to-gemini/SKILL.md</tools>
  <verify>grep -c "mcp__triumvirate__ask_session" ~/.claude/skills/send-to-gemini/SKILL.md</verify>
  <reality_test>Invoke /send-to-gemini with a question → Gemini responds via ask_session. Direct response.</reality_test>
  <done_when>Skill references mcp__triumvirate__ask_session. No inter-agent references.</done_when>
</task>

<task id="T-014" req="REQ-J4" wave="3" depends="T-011">
  <description>Update send-to-siblings skill to use mcp__triumvirate__ask_session for both agents</description>
  <files>~/.claude/skills/send-to-siblings/SKILL.md</files>
  <scope_out>Same as T-012.</scope_out>
  <tools>cat ~/.claude/skills/send-to-siblings/SKILL.md</tools>
  <verify>grep -c "mcp__triumvirate__ask_session" ~/.claude/skills/send-to-siblings/SKILL.md</verify>
  <reality_test>Invoke /send-to-siblings → both Gemini and Codex respond via ask_session. Both direct responses.</reality_test>
  <done_when>Skill references mcp__triumvirate__ask_session for both agents. No inter-agent references.</done_when>
</task>

<task id="T-015" req="REQ-A1" wave="3" depends="T-011">
  <description>Update inter-agent-protocol, goatrodeo, design-goatrodeo, and crystallize skills to use mcp__triumvirate__* tool names</description>
  <files>~/.claude/skills/inter-agent-protocol/SKILL.md, ~/.claude/skills/goatrodeo.md, ~/.claude/skills/design-goatrodeo.md, ~/.claude/skills/crystallize/factory/phase-2-diagnose.md</files>
  <scope_out>Do not change skill logic or purpose. Only update MCP tool name references from mcp__inter-agent__* to mcp__triumvirate__*.</scope_out>
  <tools>grep -r "mcp__inter-agent" ~/.claude/skills/</tools>
  <verify>grep -rc "mcp__inter-agent" ~/.claude/skills/ | grep -v ":0$" | wc -l should be 0</verify>
  <reality_test>grep -r "mcp__inter-agent" ~/.claude/skills/ returns zero matches. All skills reference mcp__triumvirate__* only.</reality_test>
  <done_when>Zero references to mcp__inter-agent in any skill file. All updated to mcp__triumvirate__.</done_when>
</task>

---

## Wave 4: Front Door Swap + Cleanup

<task id="T-016" req="REQ-F1,REQ-F2,REQ-F3,REQ-F4" wave="4" depends="T-011,T-015">
  <description>Verify all tools (canonical + aliases) work through the Rust daemon, then remove inter-agent entry from ~/.claude.json</description>
  <files>~/.claude.json</files>
  <scope_out>Do not modify the Rust daemon. This is a configuration change only. Keep a backup of ~/.claude.json before modification.</scope_out>
  <tools>cp ~/.claude.json ~/.claude.json.bak.v3.0 && cargo test --workspace</tools>
  <verify>grep -c "inter-agent" ~/.claude.json should be 0 (after removal). All MCP tools callable via mcp__triumvirate__*.</verify>
  <reality_test>After removing inter-agent entry: call spawn_daemon (alias) → works. Call spawn_session (canonical) → works. Call dispatch_codex_worktree → works. Call every alias and canonical tool name — all respond. No "tool not found" errors. Node process for inter-agent is not running.</reality_test>
  <done_when>~/.claude.json has no inter-agent entry. All 40+ tools accessible via triumvirate. No Node.js MCP process running.</done_when>
</task>

<task id="T-017" req="REQ-X1" wave="4" depends="T-016">
  <description>Archive the TS MCP server to archive/mcp-server-ts/</description>
  <files>mcp-server/, archive/mcp-server-ts/</files>
  <scope_out>Do not delete — archive. Do not modify the archived code. Preserve for reference.</scope_out>
  <tools>git mv mcp-server archive/mcp-server-ts</tools>
  <verify>test -d archive/mcp-server-ts/src && ! test -d mcp-server</verify>
  <reality_test>archive/mcp-server-ts/src/server.ts exists. mcp-server/ directory does not exist. git status shows rename.</reality_test>
  <done_when>TS MCP server archived. Original directory gone. Git tracks the move.</done_when>
</task>

<task id="T-018" req="REQ-X2,REQ-X3" wave="4" depends="T-016,T-017">
  <description>Final verification: full test suite, all tools, clean state</description>
  <files>daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>No code changes. Verification only.</scope_out>
  <tools>cargo test --workspace, wc -l daemon/crates/triumvirate/src/main.rs</tools>
  <verify>cargo test --workspace passes. wc -l main.rs < 300. No Node.js MCP processes running. All skills work.</verify>
  <reality_test>Run cargo test --workspace → all pass (including existing 156+ tests). Run wc -l main.rs → under 300. Invoke /goatrodeo → twins spawn via Rust daemon. Invoke /send-to-codex → Codex responds. curl /metrics → all metrics present. WebSocket → events flow.</reality_test>
  <done_when>Everything works. Sprint complete. Ready for v3.2 observability sprint.</done_when>
</task>

---

## Execution Contract

### Backlog Freeze
This document contains 18 tasks across 5 waves. This is the COMPLETE backlog.
- Do NOT accept new tasks until all tasks are complete (backlog_status: 0).
- If new requirements arrive mid-execution, respond: `blocked_on: scope-change — [describe new requirement]` and STOP.
- Only the human can add, remove, or reorder tasks in this backlog.

### Execution Order
- Wave order is strict: complete ALL tasks in Wave N before starting Wave N+1.
- Within a wave: tasks are parallel-safe (no dependencies on each other). Execute concurrently or in any order.
- Within a sequential group: strict FIFO. Do not start T(N+1) before T(N) is committed and reported.

### Definition of Done (Per Task)
A task is DONE when ALL of these are true:
1. Code is written (not stubbed — see reality test)
2. `<verify>` passes (compilation/type check)
3. `<reality_test>` passes (behavioral check that a stub cannot fake)
4. `<done_when>` condition is met (semantic completion check)
5. FULL test suite passes (`cargo test --workspace`) — not just this task's tests
6. Git commit is created with message referencing task ID

A task that passes its own tests but breaks other tests is NOT done. Fix the regression first.

### Commit Report Format
After each task commit, respond with EXACTLY this format and nothing else:
```
task: T-{ID}
commit: {hash}
changed: {1-5 bullets, one per file or logical change}
tests: cargo test --workspace → {pass count}/{total count} passed
remaining: {N} tasks in current wave, {M} total
```
No interim progress updates. No explanations between tasks. No summaries until backlog_status: 0.

### Collateral Fix Protocol
If completing a task REQUIRES touching files outside that task's `<files>` list:
1. Label the commit: `collateral-fix: T-{ID} — {one-line justification}`
2. List extra files in the commit report under a `collateral:` field
3. Re-run full test suite after the collateral fix

If you WANT to touch adjacent code but don't NEED to, don't. Scope discipline > local improvement.

### Blocked Protocol
If blocked on any task, respond with EXACTLY:
```
blocked_on: {single concrete blocker}
task: T-{ID}
evidence: {command + output summary, max 5 lines}
proposed_fix: {single action you would take}
```
Then STOP. Do not proceed to the next task. Do not attempt workarounds without reporting.

### Context-Switch Refusal
If you receive instructions not in this backlog during execution:
- Respond: "Outside current execution contract. Backlog has {N} remaining tasks. Complete backlog first, or explicitly cancel it."
- Do NOT start the new work.

### Self-Validation (MANDATORY)
After each task commit, run the validation script:
```
~/.claude/scripts/validate-task.sh T-{ID} "cargo test --workspace" {files from <files> list}
```
- If BLOCKED (exit 1): fix the failure before proceeding. Do NOT skip to next task.
- If WARN (exit 2): proceed, but include warnings in commit report.
- If PASS (exit 0): proceed to next task.

### End-of-Execution Report
When all tasks are complete, respond with:
```
backlog_status: 0 remaining
completed_tasks: [T-001, T-002, ..., T-018]
total_commits: {N}
collateral_fixes: {N} ({list if any})
validation: {N}/{N} tasks passed validate-task.sh
test_suite: cargo test --workspace → {pass/fail with counts}
```
