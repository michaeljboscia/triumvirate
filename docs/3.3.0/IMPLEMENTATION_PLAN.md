# IMPLEMENTATION_PLAN — v3.3.0 Live Agent Streaming

**Version:** 3.3.0
**Working Directory:** /Users/mikeboscia/projects/triumvirate
**Git Branch:** v3.3.0 (branch from main after Wave 0 commit)
**Spec:** docs/3.3.0/SPEC.md
**PRD:** docs/3.3.0/PRD.md

## Wave 0: Contracts + Types (sequential)

<task id="T-300" req="REQ-E01,REQ-E02" wave="0" depends="">
  <description>Define AgentStreamEvent enum and EventSequencer in shared-types and daemon-core. This is the contract all downstream tasks build against.</description>
  <files>daemon/crates/shared-types/src/streaming.rs, daemon/crates/shared-types/src/lib.rs, daemon/crates/daemon-core/src/sequencer.rs, daemon/crates/daemon-core/src/lib.rs</files>
  <scope_out>Do not modify any existing types. Do not touch agent-adapter or the binary crate. Only define new types and re-export them.</scope_out>
  <tools>cargo check --workspace, file read/write within files list</tools>
  <verify>cargo check --workspace</verify>
  <reality_test>Import AgentStreamEvent in a test, construct all 6 variants, serialize to JSON, deserialize back. Assert round-trip equality. Assert each variant has a seq field. Assert serde tag is "event_type".</reality_test>
  <done_when>AgentStreamEvent with 6 variants and EventSequencer are defined, exported, and compile across the workspace. All variants serialize/deserialize correctly with serde tag discriminator.</done_when>
</task>

<task id="T-301" req="REQ-E01" wave="0" depends="T-300">
  <description>Define the streaming executor function signature and the adapter wrapper. No implementation — just the signatures and the blob-collecting wrapper logic.</description>
  <files>daemon/crates/triumvirate/src/streaming.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not modify agent-adapter parsers. Do not implement actual streaming. Only define signatures and the adapter that collects mpsc into String.</scope_out>
  <tools>cargo check -p triumvirate, file read/write within files list</tools>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Call execute_ask_agent() (the adapter) with a mock that sends 3 events then a final string. Assert the adapter returns the string and the events were produced on the channel.</reality_test>
  <done_when>execute_ask_agent_streaming() signature exists. execute_ask_agent() wraps it. Both compile. Adapter correctly collects channel into blob.</done_when>
</task>

## Wave 1: Event Pipeline (parallel-safe)

<task id="T-302" req="REQ-E03" wave="1" depends="T-300,T-301">
  <description>Modify GeminiStreamParser to emit AgentStreamEvent via mpsc channel during stream parsing. Parser continues to return final String as before.</description>
  <files>daemon/crates/agent-adapter/src/gemini.rs</files>
  <scope_out>Do not modify CodexExecParser. Do not modify the public return type of the parser's main function. Do not touch daemon-core or the binary crate.</scope_out>
  <tools>cargo check -p agent-adapter, cargo test -p agent-adapter, file read/write within files list</tools>
  <verify>cargo check -p agent-adapter</verify>
  <reality_test>Feed the parser a recorded Gemini NDJSON stream (fixture file). Assert it emits TurnStarted, at least one ToolCall, and TurnCompleted events via the channel. Assert the final String result is unchanged from v3.2.0 behavior.</reality_test>
  <done_when>GeminiStreamParser emits structured AgentStreamEvents during parsing while still returning the final response string. Events include tool calls with names and file reads with paths.</done_when>
</task>

<task id="T-303" req="REQ-E03" wave="1" depends="T-300,T-301">
  <description>Modify CodexExecParser to emit AgentStreamEvent via mpsc channel during stream parsing.</description>
  <files>daemon/crates/agent-adapter/src/codex.rs</files>
  <scope_out>Do not modify GeminiStreamParser. Do not modify the public return type. Do not touch daemon-core or the binary crate.</scope_out>
  <tools>cargo check -p agent-adapter, cargo test -p agent-adapter, file read/write within files list</tools>
  <verify>cargo check -p agent-adapter</verify>
  <reality_test>Feed the parser a recorded Codex exec JSONL stream (fixture file). Assert it emits TurnStarted, at least one ToolCall, and TurnCompleted events. Assert TurnCompleted includes token counts (tokens_in may be 0 if Codex doesn't report them — use Option).</reality_test>
  <done_when>CodexExecParser emits structured AgentStreamEvents during parsing while still returning the final response string.</done_when>
</task>

<task id="T-304" req="REQ-E04" wave="1" depends="T-300">
  <description>Wire AgentStreamEvent into the WebSocket broadcast. The daemon's ObservabilityBus re-broadcasts events from a subscriber channel to /ws as agent_stream events.</description>
  <files>daemon/crates/daemon-core/src/observability.rs, daemon/crates/daemon-http/src/lib.rs</files>
  <scope_out>Do not modify existing WS event types (token_update, abe_task_state, fleet_progress). Do not modify the watch CLI (doesn't exist yet). Do not touch agent-adapter.</scope_out>
  <tools>cargo check -p daemon-core -p daemon-http, cargo test -p daemon-core, file read/write within files list</tools>
  <verify>cargo check -p daemon-core -p daemon-http</verify>
  <reality_test>In a unit test, create an ObservabilityBus, subscribe to WS events, publish an AgentStreamEvent::TurnStarted. Assert the subscriber receives a JSON string with event_type "agent_stream" containing the TurnStarted data. Assert existing event types still work.</reality_test>
  <done_when>AgentStreamEvent values published to ObservabilityBus appear on the WebSocket broadcast as agent_stream events. Existing events unchanged.</done_when>
</task>

## Wave 2: Streamable HTTP Transport (sequential — depends on Wave 1)

<task id="T-305" req="REQ-H01,REQ-H02,REQ-H03,REQ-H04,REQ-H07" wave="2" depends="T-302,T-303,T-304">
  <description>Add Streamable HTTP MCP endpoint at /mcp on the existing daemon Axum server. Uses rmcp transport-streamable-http-server feature. Shares Arc McpBridge with stdio transport.</description>
  <files>daemon/Cargo.toml, daemon/crates/triumvirate/src/main.rs, daemon/crates/triumvirate/src/http_mcp.rs</files>
  <scope_out>Do not modify existing HTTP routes. Do not modify the stdio MCP path. Do not modify McpBridge tool implementations. Only add the new transport endpoint and wire it to the shared McpBridge.</scope_out>
  <tools>cargo check -p triumvirate, cargo test -p triumvirate --lib, file read/write within files list</tools>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Start the daemon. POST a JSON-RPC initialize request to http://127.0.0.1:8080/mcp. Assert response contains Mcp-Session-Id header and server capabilities listing all 35+ tools. POST a tools/call for ping tool. Assert valid JSON-RPC response. GET /mcp with Accept: text/event-stream — assert connection opens with Content-Type text/event-stream and stays alive for at least 2s.</reality_test>
  <done_when>Daemon serves MCP over Streamable HTTP at /mcp. Tools callable via HTTP POST. GET /mcp opens SSE stream for server notifications. Session ID management working. All existing tools available on both transports.</done_when>
</task>

<task id="T-306" req="REQ-H05,REQ-H06,REQ-H09" wave="2" depends="T-305">
  <description>Enable SSE streaming on the /mcp endpoint during tool execution. Stream formatted text chunks as progress, final result as last frame. Add bearer auth and heartbeat events.</description>
  <files>daemon/crates/triumvirate/src/http_mcp.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not modify the proxy or watch CLI. Do not modify agent-adapter. Do not change McpBridge tool signatures.</scope_out>
  <tools>cargo check -p triumvirate, cargo test -p triumvirate --lib, curl for manual testing, file read/write within files list</tools>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Start daemon. Call ask_agent tool via HTTP POST to /mcp with a simple prompt. Observe SSE stream: at least one intermediate frame ("→ {agent}: turn started") before the final result frame. Verify bearer auth rejects unauthenticated requests with 401. Verify heartbeat during generation >5s.</reality_test>
  <done_when>SSE streaming works during tool execution with formatted text chunks, final result closes stream, bearer auth enforced, heartbeat events during long generation.</done_when>
</task>

<task id="T-307" req="REQ-H10" wave="2" depends="T-306">
  <description>Integration tests for the Streamable HTTP MCP endpoint.</description>
  <files>daemon/crates/triumvirate/tests/integration_streaming.rs</files>
  <scope_out>Do not modify daemon code. Only write tests. Do not modify existing integration test files.</scope_out>
  <tools>cargo test -p triumvirate --test integration_streaming -- --ignored, file read/write within files list</tools>
  <verify>cargo check -p triumvirate --test integration_streaming</verify>
  <reality_test>Tests connect to /mcp, call a tool, parse SSE frames. Assert: at least 2 intermediate frames received before final result. Assert: Mcp-Session-Id header present. Assert: unauthenticated request returns 401.</reality_test>
  <done_when>Integration test file with at least 5 tests that verify SSE streaming, session management, auth, and tool execution over Streamable HTTP.</done_when>
</task>

## Wave 3: Proxy + Watch CLI (parallel-safe)

<task id="T-308" req="REQ-P01,REQ-P02,REQ-P03,REQ-P04" wave="3" depends="T-305">
  <description>Implement triumvirate proxy subcommand. Bridges stdio JSON-RPC from Claude Code to daemon HTTP /mcp endpoint. Auto-reconnect with bounded backoff.</description>
  <files>daemon/crates/triumvirate/src/proxy.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not modify the daemon's HTTP MCP endpoint. Do not modify the mcp subcommand. Do not modify agent-adapter. Do not touch watch CLI.</scope_out>
  <tools>cargo check -p triumvirate, cargo test -p triumvirate --lib, file read/write within files list</tools>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Start daemon. Run `triumvirate proxy` in a subprocess. Write a JSON-RPC tools/list request to its stdin. Assert stdout contains the 35+ tool list from the daemon. Kill daemon. Write another request. Assert error response (not hang/crash). Restart daemon. Write another request. Assert success (reconnect worked).</reality_test>
  <done_when>triumvirate proxy bridges stdio↔HTTP, auto-reconnects after daemon restart, exits cleanly when daemon unreachable at startup after 5s retry.</done_when>
</task>

<task id="T-309" req="REQ-W01,REQ-W02,REQ-W03,REQ-W04,REQ-W05,REQ-W06" wave="3" depends="T-304">
  <description>Implement triumvirate watch subcommand. WebSocket client that pretty-prints AgentStreamEvent from /ws.</description>
  <files>daemon/crates/triumvirate/src/watch.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do not modify daemon-http or WebSocket broadcast. Do not modify agent-adapter. Do not touch proxy. Do not modify existing CLI commands.</scope_out>
  <tools>cargo check -p triumvirate, file read/write within files list</tools>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Start daemon. Run `triumvirate watch` as subprocess. Publish a mock AgentStreamEvent::ToolCall to the daemon's WS broadcast. Assert watch stdout contains "→ {agent}: calling {tool_name}". Test --session filter: publish events for two sessions, assert filter shows only the specified one. Test gap detection: skip a seq number, assert "[events skipped]" message appears.</reality_test>
  <done_when>triumvirate watch connects to /ws, pretty-prints agent_stream events, supports --all and --session flags, shows heartbeat timer during generation, detects seq gaps, handles daemon-not-running gracefully.</done_when>
</task>

## Wave 4: Spike Test + Polish (parallel-safe)

<task id="T-310" req="REQ-K01" wave="4" depends="T-305">
  <description>Build and run the SSE spike test. Minimal MCP server that sends progress notifications over Streamable HTTP. Register in Claude Code. Document whether Claude Code renders intermediate SSE frames.</description>
  <files>daemon/spike/sse-test-server/src/main.rs, daemon/spike/sse-test-server/Cargo.toml, docs/3.3.0/SPIKE_RESULTS.md</files>
  <scope_out>Do not modify any daemon code. This is a standalone test binary. Do not modify ~/.claude.json permanently — register temporarily, test, remove.</scope_out>
  <tools>cargo build, cargo run, claude mcp add/remove, file read/write within files list</tools>
  <verify>cargo check --manifest-path daemon/spike/sse-test-server/Cargo.toml</verify>
  <reality_test>The spike server runs. Claude Code connects. A tool is called. The document SPIKE_RESULTS.md contains either "Claude Code renders intermediate SSE frames: YES (evidence: [screenshot/log])" or "Claude Code renders intermediate SSE frames: NO (evidence: [description])". A non-answer is not acceptable.</reality_test>
  <done_when>SPIKE_RESULTS.md exists with a definitive YES or NO answer about Claude Code SSE frame rendering, with evidence.</done_when>
</task>

<task id="T-312" req="" wave="4" depends="T-302,T-303,T-304,T-305,T-306,T-307,T-308,T-309">
  <description>Produce BUILD_MANIFEST.md documenting every task, commit SHA, files changed, test results, and deviations. This is a MANDATORY goatrodeo gate (P6.2) that has been missed 5 sprints in a row.</description>
  <files>docs/3.3.0/BUILD_MANIFEST.md</files>
  <scope_out>Do not modify any code. Only produce the manifest document.</scope_out>
  <tools>git log, git diff --stat, file read/write within files list</tools>
  <verify>test -f docs/3.3.0/BUILD_MANIFEST.md</verify>
  <reality_test>BUILD_MANIFEST.md exists and contains: task ID, commit SHA, files changed, test pass/fail for EVERY task T-300 through T-311. No placeholder rows. No "TBD" entries.</reality_test>
  <done_when>BUILD_MANIFEST.md has a complete row for every task with real commit SHAs and test results.</done_when>
</task>

<task id="T-311" req="REQ-E01,REQ-H08" wave="4" depends="T-305,T-308,T-309,T-312">
  <description>End-to-end verification. Full flow: daemon + proxy + watch + Claude Code. Update docs. Version bump.</description>
  <files>daemon/Cargo.toml, CHANGELOG.md, README.md, docs/3.3.0/RELEASE_NOTES.md</files>
  <scope_out>Do not modify daemon code except version bump. Do not add features. Only verify, document, and version.</scope_out>
  <tools>cargo check --workspace, cargo test --workspace --lib, triumvirate daemon, triumvirate proxy, triumvirate watch, file read/write within files list</tools>
  <verify>cargo check --workspace</verify>
  <reality_test>Run the full stack: daemon + proxy (configured in Claude Code) + watch (side pane). Ask an agent a question via Claude Code. Watch pane shows streaming events. Claude Code returns the final result. All 150+ existing tests still pass.</reality_test>
  <done_when>Full end-to-end flow works. Version bumped to 3.3.0. CHANGELOG updated. README updated. No regressions in existing test suite.</done_when>
</task>

## Summary

| Wave | Tasks | Parallel? | Depends On |
|------|-------|-----------|------------|
| 0 | T-300, T-301 | Sequential | — |
| 1 | T-302, T-303, T-304 | Parallel | Wave 0 |
| 2 | T-305, T-306, T-307 | Sequential | Wave 1 |
| 3 | T-308, T-309 | Parallel | Wave 2 (proxy needs T-305), Wave 1 (watch needs T-304) |
| 4 | T-310, T-311 | Parallel | Wave 2+ |

**Total: 12 tasks across 5 waves.**

## Execution Contract

### Backlog Freeze
This document contains 12 tasks across 5 waves. This is the COMPLETE backlog.
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
5. FULL test suite passes (`cargo test --workspace --lib`) — not just this task's tests
6. Git commit is created with message referencing task ID

A task that passes its own tests but breaks other tests is NOT done. Fix the regression first.

### Commit Report Format
After each task commit, respond with EXACTLY this format and nothing else:
```
task: T-{ID}
commit: {hash}
changed: {1-5 bullets, one per file or logical change}
tests: cargo test --workspace --lib → {pass count}/{total count} passed
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
- Do NOT interleave new work with backlog tasks.
- Only an explicit "cancel backlog" or "pause backlog" from the human allows context-switching.

### Self-Validation (MANDATORY)
After each task commit, run the validation script:
```
~/.claude/scripts/validate-task.sh T-{ID} "cargo test --workspace --lib" {files from <files> list}
```
- If BLOCKED (exit 1): fix the failure before proceeding. Do NOT skip to next task.
- If WARN (exit 2): proceed, but include warnings in commit report.
- If PASS (exit 0): proceed to next task.

### End-of-Execution Report
When all tasks are complete, respond with:
```
backlog_status: 0 remaining
completed_tasks: [T-300, T-301, T-302, T-303, T-304, T-305, T-306, T-307, T-308, T-309, T-310, T-311]
total_commits: {N}
collateral_fixes: {N} ({list if any})
validation: {N}/{N} tasks passed validate-task.sh
test_suite: cargo test --workspace --lib → {pass/fail with counts}
```
