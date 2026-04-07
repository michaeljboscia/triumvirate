# FLOW STATE SPEC — Triumvirate v2.1

**Status:** DRAFT — Pending goatrodeo review
**Date:** 2026-04-06
**Authors:** Claude (Opus 4.6) + Mike Boscia
**Depends on:** CONVERSATIONAL_PARITY.md
**Research artifacts:** `research/agent-state-internals-20260406.md`, `research/agent-protocol-deep-dive-20260406.md`, `research/working-state-signals-research-20260406.md`
**Generated schemas:** `docs/v2/codex-protocol/` (615 files, auto-generated from Codex binary)

---

## 1. Problem

When triumvirate dispatches work to Gemini or Codex via `spawn_session` / `ask_session`, the caller sees:

```
→ Gemini: working... (10s elapsed)
→ Gemini: working... (20s elapsed)
→ Gemini: working... (40s elapsed)
→ Gemini: done
```

This is a dumb timer. It carries zero information about what the agent is doing — thinking, calling tools, writing files, executing commands, stuck in a loop, or actually generating a response.

Both CLIs internally know exactly what they're doing. They were designed for orchestrators. Triumvirate ignores their signals.

## 2. Discovery: The Signal Is Already There

**The daemon already spawns Gemini with `--output-format stream-json`.** The function `run_gemini_cli_process_with_session()` (main.rs:1326-1412) parses NDJSON stdout. But it only extracts `{"type":"message","role":"assistant"}` events — throwing away `TOOL_USE`, `TOOL_RESULT`, `ERROR`, `INIT`, and `RESULT` events that contain tool names, success/failure, token usage, stuck detection, and timing.

For Codex, the daemon spawns `codex exec --json` which produces JSONL with thread_id and response data. Most structured data is discarded — only the final text from `--output-last-message <tmpfile>` is used.

**The fix is not to build new infrastructure. The fix is to parse what we already receive.**

## 3. Second Discovery: Live Events Are Blocked

Both agent runners use `Command::new(...).output().await` which buffers ALL stdout and waits for process exit. This means **zero live events are possible with the current architecture** — even if we parse all event types, we only see them after the agent finishes.

To get real-time flow state, we must switch from `.output()` to `.spawn()` with line-by-line async reading. This is the single most important architectural change.

## 4. Goal

Replace dumb heartbeats with real working-state signals:

```
→ Gemini: thinking...
→ Gemini: calling ReadFile (src/auth/middleware.rs)
→ Gemini: ReadFile completed (247 lines, 0.3s)
→ Gemini: calling Edit (src/auth/middleware.rs)
→ Gemini: Edit completed
→ Gemini: generating response
→ Gemini: done (1,247 tokens, 3 tool calls, 8.2s)
```

### Requirements

- **REQ-001:** The daemon MUST parse all Gemini stream-json event types (init, message, tool_use, tool_result, error, result) from the NDJSON stdout it already receives.
- **REQ-002:** The daemon MUST parse Codex exec JSONL output to extract tool calls, token usage, and thread_id.
- **REQ-003:** The daemon MUST emit real-time working-state events during agent execution, not just after completion.
- **REQ-004:** Working-state events MUST surface through the existing ProgressEmitter as human-readable messages (e.g. "Gemini: calling ReadFile (src/main.rs)").
- **REQ-005:** The heartbeat timer MUST become a fallback — only emitted when no real state events have been received within the heartbeat interval.
- **REQ-006:** The daemon MUST define a unified WorkingState enum that normalizes both Gemini and Codex events to a single vocabulary.
- **REQ-007:** The daemon MUST switch from `Command::output().await` (batch) to `Command::spawn()` with line-by-line async stdout reading to enable live event emission.
- **REQ-008:** The daemon MUST capture token usage from both agents (Gemini: result.stats, Codex: thread/tokenUsage/updated) and include it in the AgentResult.
- **REQ-009:** The daemon MUST detect stuck agents — Gemini via built-in LoopDetected event, Codex via a StuckDetector that monitors for idle timeouts (>60s) and repeated tool call patterns (>5x same tool).
- **REQ-010:** When a stuck condition is detected, the daemon MUST emit `WorkingState::Stuck` and surface it to the MCP caller.
- **REQ-011:** The Codex app-server integration (JSON-RPC 2.0 over stdio) MUST be opt-in via `TRIUMVIRATE_CODEX_PROTOCOL=app-server` environment variable, with `exec` as the default.
- **REQ-012:** If the stream parser returns an empty response, the daemon MUST fall back to the current raw-stdout-trimming behavior.
- **REQ-013:** The Gemini live streaming mode MUST be disableable via `TRIUMVIRATE_GEMINI_STREAMING=false` environment variable, falling back to batch parsing.
- **REQ-014:** All spawned agent subprocesses MUST use `kill_on_drop(true)` to prevent orphaned processes.
- **REQ-015:** The `ask_twins` MCP tool, HTTP route, execution function, shared types, and tests MUST be removed (~750 lines).
- **REQ-016:** Tool call records MUST include tool name, success/failure status, and duration where available.
- **REQ-017:** The OutboxEvent struct MUST be extended with optional `working_state`, `token_usage`, and `tool_name` fields (backward compatible — old readers ignore new fields).
- **REQ-018:** A new `agent-adapter` crate MUST be created to house WorkingState types, parsers, stream parser, and StuckDetector — separate from the MCP surface types in shared-types.
- **REQ-019:** The `ask_session` MCP tool response format MUST remain unchanged (final text). Working-state events are additive progress notifications, not response format changes.
- **REQ-020:** The Codex app-server client MUST implement the JSON-RPC 2.0 handshake: initialize → thread/start → turn/start → stream notifications → turn/completed.
- **REQ-021:** All existing workspace tests MUST continue to pass after every phase of implementation.

## 5. Non-Goals

- **NG-001:** Modifying Gemini or Codex source code
- **NG-002:** Building a dashboard (visualization is separate from the signal)
- **NG-003:** Real-time token content streaming to MCP caller (just state, not content)
- **NG-004:** Full ACP integration (deferred to v2.2)
- **NG-005:** Gemini ACP mode for v2.1 (stream-json is sufficient; ACP adds thinking tokens in v2.2)
- **NG-006:** Codex WebSocket transport (stdio:// is sufficient; ws:// adds dependency for no benefit)
- **NG-007:** Broadcast/pub-sub fabric (deferred — JSONL outbox is sufficient for v2.1)
- **NG-008:** Stenographer implementation (requires broadcast fabric)
- **NG-009:** Dashboard/frontend for flow state visualization

---

# PART I: RESEARCH — Agent Protocol Inventory

Everything below was verified by reading the actual source code of both CLI binaries installed on this machine. Not documentation — source.

---

## 6. Gemini CLI Internals

**Binary:** `/opt/homebrew/bin/gemini` → `/opt/homebrew/lib/node_modules/@google/gemini-cli/dist/index.js`
**Version:** 0.35.0
**Runtime:** Node.js
**Source root:** `/opt/homebrew/lib/node_modules/@google/gemini-cli/dist/src/`
**Core library:** `/opt/homebrew/lib/node_modules/@google/gemini-cli/node_modules/@google/gemini-cli-core/dist/src/`

### 6.1 Three Operating Modes

| Mode | Flag | Transport | Protocol | Fidelity |
|------|------|-----------|----------|----------|
| Interactive | (default) | Terminal | Ink/React UI | Highest (UI state) |
| Non-Interactive | `-p "prompt"` | stdin/stdout | Text or NDJSON | Medium |
| ACP | `--acp` | NDJSON stdio | Agent Client Protocol v1 | Highest (orchestrator) |

Mode routing at `gemini.js:352-354`:
```javascript
if (config.getAcpMode()) return runAcpClient(config, settings, argv);
```

ACP detection at `config.js:508`:
```javascript
const isAcpMode = !!argv.acp || !!argv.experimentalAcp;
```

### 6.2 GeminiEventType Enum (Core Agent Loop Events)

Source: `gemini-cli-core/dist/src/core/turn.js:15-35`

These events flow through `sendMessageStream()` in the agent loop:

| Value | Constant | Description | Flow State Mapping |
|-------|----------|-------------|-------------------|
| `content` | `Content` | Text token delta | `Generating` |
| `tool_call_request` | `ToolCallRequest` | Agent wants to call a tool | `ToolCalling` |
| `tool_call_response` | `ToolCallResponse` | Tool returned result | `ToolDone` |
| `tool_call_confirmation` | `ToolCallConfirmation` | Tool needs approval | `WaitingForApproval` |
| `user_cancelled` | `UserCancelled` | User cancelled | `Error` |
| `error` | `Error` | Error during generation | `Error` |
| `chat_compressed` | `ChatCompressed` | Context compacted | `ContextCompacting` |
| `thought` | `Thought` | Reasoning token | `Thinking` |
| `max_session_turns` | `MaxSessionTurns` | Turn limit exceeded | `Error` |
| `finished` | `Finished` | Turn complete | `TurnCompleted` |
| `loop_detected` | `LoopDetected` | **STUCK** — agent is looping | `Stuck` |
| `citation` | `Citation` | Source citation | (informational) |
| `retry` | `Retry` | Retrying after error | (informational) |
| `context_window_will_overflow` | `ContextWindowWillOverflow` | Context near limit | (warning) |
| `invalid_stream` | `InvalidStream` | Stream parse error | `Error` |
| `model_info` | `ModelInfo` | Model metadata | (informational) |
| `agent_execution_stopped` | `AgentExecutionStopped` | Clean exit | `TurnCompleted` |
| `agent_execution_blocked` | `AgentExecutionBlocked` | Needs approval | `WaitingForApproval` |

### 6.3 Stream-JSON Wire Format

Flag: `--output-format stream-json` (or `-o stream-json`)

Source: `gemini-cli-core/dist/src/output/types.js:13-21`

```javascript
JsonStreamEventType = { INIT: "init", MESSAGE: "message", TOOL_USE: "tool_use",
                        TOOL_RESULT: "tool_result", ERROR: "error", RESULT: "result" }
```

**Exact wire format (from test suite at `stream-json-formatter.test.js`):**

#### INIT — Session started
```json
{"type":"init","timestamp":"2025-10-10T12:00:00.000Z","session_id":"test-session-123","model":"gemini-2.0-flash-exp"}
```

#### MESSAGE — Text delta
```json
{"type":"message","timestamp":"...","role":"user","content":"What is 2+2?"}
{"type":"message","timestamp":"...","role":"assistant","content":"4","delta":true}
```

#### TOOL_USE — Tool invocation
```json
{"type":"tool_use","timestamp":"...","tool_name":"Read","tool_id":"read-123","parameters":{"file_path":"/path/to/file.txt"}}
```

#### TOOL_RESULT — Tool completion
```json
{"type":"tool_result","timestamp":"...","tool_id":"read-123","status":"success","output":"File contents here"}
{"type":"tool_result","timestamp":"...","tool_id":"read-123","status":"error","error":{"type":"FILE_NOT_FOUND","message":"File not found"}}
```

#### ERROR — Error event
```json
{"type":"error","timestamp":"...","severity":"warning","message":"Loop detected, stopping execution"}
{"type":"error","timestamp":"...","severity":"error","message":"Maximum session turns exceeded"}
```

#### RESULT — Final result with metrics
```json
{"type":"result","timestamp":"...","status":"success","stats":{"total_tokens":100,"input_tokens":50,"output_tokens":50,"cached":0,"input":50,"duration_ms":1200,"tool_calls":2,"models":{"gemini-2.0-flash":{"total_tokens":80,"input_tokens":50,"output_tokens":30,"cached":0,"input":50}}}}
{"type":"result","timestamp":"...","status":"error","error":{"type":"MaxSessionTurnsError","message":"Maximum session turns exceeded"},"stats":{...}}
```

### 6.4 ACP Protocol (Agent Client Protocol)

**SDK:** `@agentclientprotocol/sdk` v0.12.0
**Protocol version:** 1 (`acp.PROTOCOL_VERSION`)
**Transport:** NDJSON (Newline-Delimited JSON) over stdio
**Flag:** `gemini --acp`

#### Bootstrap Sequence

1. Host spawns `gemini --acp` as subprocess
2. Wraps stdin/stdout as NDJSON streams: `acp.ndJsonStream(stdout, stdin)`
3. Creates `AgentSideConnection` → triggers `initialize()` callback
4. Initialize returns: `protocolVersion`, `authMethods[]`, `agentInfo`, `agentCapabilities`
5. Host authenticates (API key or OAuth)
6. Host calls `newSession()` or `loadSession()`
7. Host calls `prompt()` → receives streaming session updates

Source: `acp/acpClient.js:22-34, 51-100`

#### Session Update Events (real-time during prompt execution)

Source: `acp/acpClient.js:380-740`

| sessionUpdate | Fields | Description |
|--------------|--------|-------------|
| `agent_thought_chunk` | `content: { type: "text", text }` | Reasoning/thinking tokens |
| `agent_message_chunk` | `content: { type: "text", text }` | Response text streaming |
| `user_message_chunk` | `content: { type: "text", text }` | User input echo |
| `tool_call` | `toolCallId, status, title, content[], locations, kind` | Tool started |
| `tool_call_update` | `toolCallId, status, content[]` | Tool completed |
| `available_commands_update` | `commands[]` | Slash commands |

**Tool status lifecycle:** `pending` → `in_progress` → `completed` | `failed`

**Tool kinds** (from `toAcpToolKind()` at acpClient.js:1146-1166):

| Internal Kind | ACP Kind | Description |
|--------------|----------|-------------|
| `Kind.Read` | `"read"` | File read |
| `Kind.Edit` | `"edit"` | File edit/write |
| `Kind.Execute` | `"execute"` | Shell command |
| `Kind.Search` | `"search"` | Search/grep |
| `Kind.Delete` | `"delete"` | File delete |
| `Kind.Move` | `"move"` | File rename/move |
| `Kind.Think` | `"think"` | Internal reasoning |
| `Kind.Fetch` | `"fetch"` | HTTP fetch |
| `Kind.SwitchMode` | `"switch_mode"` | Mode change |
| `Kind.Agent` | `"think"` | Sub-agent invocation |
| `Kind.Plan` | `"other"` | Planning |
| `Kind.Communicate` | `"other"` | Communication |

**Tool call content types:**
```typescript
{ type: "content", content: { type: "text", text: "..." } }
{ type: "diff", path: "file.rs", oldText: "...", newText: "...", _meta: { kind: "add" | "delete" | "modify" } }
```

**Permission flow:** When a tool requires confirmation, ACP calls `connection.requestPermission()` which is a request-response cycle. The host (triumvirate) must respond with an outcome: `ProceedOnce`, `ProceedAlways`, `Cancel`, etc.

#### Telemetry (via uiTelemetryService)

Source: `ui/contexts/SessionContext.js:104-145`

Tracked per session:
- API requests count, errors, total latency
- Token counts: input, output, cached, thoughts, per-model
- Tool calls: count, success, failure, duration, per-tool-name, decisions (accept/reject/modify/auto)
- File changes: lines added, lines removed

### 6.5 Gemini Internal State Enum

Source: `ui/types.js:22-27`

```javascript
StreamingState = { "Idle": "idle", "Responding": "responding", "WaitingForConfirmation": "waiting_for_confirmation" }
```

### 6.6 Hook System

Source: `gemini.js:420-437`

- `SessionStartEvent` fired before prompt processing (line 422) — can inject system messages
- `SessionEndEvent` registered for graceful exit (line 437)

### 6.7 Agent Event Loop (non-interactive mode)

Source: `nonInteractiveCli.js:186-280`

```
while (true):
  1. sendMessageStream() → get event iterator
  2. for each event:
     - Content → accumulate text (stream-json: MESSAGE delta)
     - ToolCallRequest → schedule tool (stream-json: TOOL_USE)
     - LoopDetected → warn (stream-json: ERROR warning)
     - MaxSessionTurns → error
     - AgentExecutionStopped → emit RESULT, return
     - AgentExecutionBlocked → warn
  3. Execute scheduled tool calls → get results (stream-json: TOOL_RESULT)
  4. If tool results → loop (next turn with tool responses)
  5. Else → exit loop
```

---

## 7. Codex CLI Internals

**Binary:** `/opt/homebrew/bin/codex` → Rust native binary (codex-rs)
**Version:** 0.118.0
**Runtime:** Rust (aarch64-apple-darwin)
**Protocol surface:** Auto-generated TypeScript + JSON Schema

### 7.1 Two Operating Modes

| Mode | Command | Transport | Fidelity |
|------|---------|-----------|----------|
| Exec | `codex exec "prompt"` | Subprocess stdin/stdout | Low (final result only) |
| App-Server | `codex app-server --listen <url>` | WebSocket or stdio | Full (all notifications) |

App-server supports:
- `stdio://` (default) — JSON-RPC 2.0 over piped stdin/stdout
- `ws://IP:PORT` — JSON-RPC 2.0 over WebSocket

### 7.2 Schema Generation (Self-Documenting)

```bash
codex app-server generate-json-schema --out /path/to/schemas   # JSON Schema for all types
codex app-server generate-ts --out /path/to/types              # TypeScript types
```

Generated schemas committed to `docs/v2/codex-protocol/` (615 files). Types generated via `ts-rs` from the Rust source.

### 7.3 JSON-RPC Handshake Sequence

Source: Generated types at `/tmp/codex-ts/`

#### Step 1: Initialize

```json
{
  "jsonrpc": "2.0", "id": 1, "method": "initialize",
  "params": {
    "clientInfo": { "name": "triumvirate", "title": "Triumvirate Daemon", "version": "0.1.0" },
    "capabilities": { "experimentalApi": true }
  }
}
```

Response:
```json
{
  "jsonrpc": "2.0", "id": 1,
  "result": { "userAgent": "codex/0.118.0", "codexHome": "/Users/.../.codex", "platformFamily": "unix", "platformOs": "macos" }
}
```

Then client sends: `{"jsonrpc": "2.0", "method": "initialized"}`

#### Step 2: Start Thread

```json
{
  "jsonrpc": "2.0", "id": 2, "method": "thread/start",
  "params": {
    "model": "gpt-5.3-codex", "cwd": "/path/to/project",
    "ephemeral": false, "persistExtendedHistory": true
  }
}
```

Response includes `thread.id`, `thread.status`, `thread.cwd`.

#### Step 3: Start Turn (Send Prompt)

```json
{
  "jsonrpc": "2.0", "id": 3, "method": "turn/start",
  "params": {
    "threadId": "thread-uuid",
    "input": [{ "type": "text", "text": "Write a hello world function", "text_elements": [] }]
  }
}
```

Response includes `turn.id`, `turn.status`.

#### Step 4: Receive Notification Stream

Server sends notifications (no `id` field — one-way):

```json
{"jsonrpc":"2.0","method":"turn/started","params":{"threadId":"...","turn":{...}}}
{"jsonrpc":"2.0","method":"item/started","params":{"item":{"type":"reasoning",...},"threadId":"...","turnId":"..."}}
{"jsonrpc":"2.0","method":"item/reasoning/textDelta","params":{"threadId":"...","turnId":"...","itemId":"...","delta":"Let me think..."}}
{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"threadId":"...","turnId":"...","itemId":"...","delta":"def hello():"}}
{"jsonrpc":"2.0","method":"item/completed","params":{"item":{...},"threadId":"...","turnId":"..."}}
{"jsonrpc":"2.0","method":"thread/tokenUsage/updated","params":{"threadId":"...","tokenUsage":{"total":{"totalTokens":1500,...}}}}
{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"...","turn":{"id":"...","status":"completed",...}}}
```

### 7.4 Complete ServerNotification Union (50+ methods)

Source: Generated `ServerNotification.ts`

**Turn lifecycle:**
- `turn/started` — `{ threadId, turn: Turn }`
- `turn/completed` — `{ threadId, turn: Turn }`
- `turn/diff/updated` — `{ threadId, turnId }`
- `turn/plan/updated` — `{ threadId, turnId }`

**Item-level events (granular):**
- `item/started` — `{ item: ThreadItem, threadId, turnId }`
- `item/completed` — `{ item: ThreadItem, threadId, turnId }`
- `item/agentMessage/delta` — `{ threadId, turnId, itemId, delta: string }`
- `item/plan/delta` — `{ threadId, turnId, itemId, delta: string }`
- `item/commandExecution/outputDelta` — `{ threadId, turnId, itemId, delta: string }`
- `item/commandExecution/terminalInteraction` — terminal I/O
- `item/fileChange/outputDelta` — `{ threadId, turnId, itemId, delta: string }`
- `item/mcpToolCall/progress` — `{ threadId, turnId, itemId, message: string }`
- `item/reasoning/textDelta` — `{ threadId, turnId, itemId, delta: string, contentIndex: number }`
- `item/reasoning/summaryTextDelta` — `{ threadId, turnId, itemId, delta: string }`
- `item/reasoning/summaryPartAdded` — new reasoning section

**Thread-level:**
- `thread/started` — thread created
- `thread/status/changed` — `{ threadId, status: ThreadStatus }`
- `thread/tokenUsage/updated` — `{ threadId, tokenUsage: { total: TokenUsageBreakdown, last: TokenUsageBreakdown, modelContextWindow } }`
- `thread/compacted` — context compaction occurred
- `thread/closed` — thread terminated
- `thread/name/updated` — thread renamed

**System:**
- `error` — `ErrorNotification`
- `hook/started` — hook execution began
- `hook/completed` — hook execution finished
- `account/updated` — `{ authMode, planType }`
- `account/rateLimits/updated` — rate limit state
- `model/rerouted` — model was substituted
- `fs/changed` — filesystem change detected
- `skills/changed` — skills list updated

**Also present but less relevant:**
- `item/autoApprovalReview/started|completed` — guardian review
- `rawResponseItem/completed` — raw API response
- `serverRequest/resolved` — approval resolved
- `mcpServer/oauthLogin/completed` — MCP server auth
- `mcpServer/startupStatus/updated` — MCP server status
- `thread/realtime/*` — voice/audio (not applicable)
- `fuzzyFileSearch/*` — file search (UI only)

### 7.5 ThreadItem Types (13 item types)

Source: Generated `ThreadItem.ts`

| type | Key Fields | Flow State |
|------|-----------|------------|
| `userMessage` | `id, content: UserInput[]` | N/A (input) |
| `hookPrompt` | `id, fragments: HookPromptFragment[]` | (informational) |
| `agentMessage` | `id, text, phase: "commentary"\|"final_answer"\|null, memoryCitation` | `Generating` |
| `plan` | `id, text` | `Planning` |
| `reasoning` | `id, summary: string[], content: string[]` | `Thinking` |
| `commandExecution` | `id, command, cwd, processId, source, status, commandActions, aggregatedOutput, exitCode, durationMs` | `ExecutingCommand` |
| `fileChange` | `id, changes: FileUpdateChange[], status: PatchApplyStatus` | `WritingFile` |
| `mcpToolCall` | `id, server, tool, status, arguments, result, error, durationMs` | `ToolCalling` |
| `dynamicToolCall` | `id, tool, arguments, status, contentItems, success, durationMs` | `ToolCalling` |
| `collabAgentToolCall` | `id, tool, status, senderThreadId, receiverThreadIds, prompt, model, reasoningEffort, agentsStates` | `SubAgent` |
| `webSearch` | `id, query, action` | `ToolCalling("web_search")` |
| `imageGeneration` | `id, status, revisedPrompt, result, savedPath` | `ToolCalling("image_gen")` |
| `contextCompaction` | `id` | `ContextCompacting` |

### 7.6 Status Enums

Source: Generated TypeScript files

```typescript
TurnStatus = "completed" | "interrupted" | "failed" | "inProgress"

ThreadStatus = { type: "notLoaded" } | { type: "idle" } | { type: "systemError" }
             | { type: "active", activeFlags: ThreadActiveFlag[] }

ThreadActiveFlag = "waitingOnApproval" | "waitingOnUserInput"

CommandExecutionStatus = "inProgress" | "completed" | "failed" | "declined"
McpToolCallStatus = "inProgress" | "completed" | "failed"
PatchApplyStatus = "inProgress" | "completed" | "failed" | "declined"
DynamicToolCallStatus = "inProgress" | "completed" | "failed"
CollabAgentToolCallStatus = "inProgress" | "completed" | "failed"

MessagePhase = "commentary" | "final_answer"
```

### 7.7 Token Usage Structure

Source: Generated `TokenUsageBreakdown.ts` (inferred from notification params)

```typescript
TokenUsageBreakdown = {
  totalTokens: number,
  inputTokens: number,
  cachedInputTokens: number,
  outputTokens: number,
  reasoningOutputTokens: number
}
```

### 7.8 Authentication

- App-server WebSocket supports: `capability-token` (file-based) or `signed-bearer-token` (JWT)
- Stdio mode: no auth needed (trusts the spawning process)
- `InitializeParams` does NOT contain auth — auth is at transport layer
- Opt-out notifications: `capabilities.optOutNotificationMethods: string[]`

### 7.9 MCP Integration

Codex uses `rmcp` — the exact same Rust MCP crate that triumvirate already depends on. This means:
- Shared understanding of MCP types
- Potential code reuse for JSON-RPC parsing
- Same async runtime (tokio)

---

## 8. Protocol Comparison Matrix

| Capability | Codex App-Server | Gemini ACP | Gemini Stream-JSON |
|-----------|-----------------|-----------|-------------------|
| Thinking/reasoning deltas | `item/reasoning/textDelta` | `agent_thought_chunk` | N/A |
| Response text deltas | `item/agentMessage/delta` | `agent_message_chunk` | `message` (delta) |
| Tool call start | `item/started` (type=mcpToolCall) | `tool_call` (in_progress) | `tool_use` |
| Tool call complete | `item/completed` (type=mcpToolCall) | `tool_call_update` (completed) | `tool_result` |
| Tool kind classification | ThreadItem.type | `kind` field (read/edit/execute/...) | N/A |
| Command execution streaming | `item/commandExecution/outputDelta` | tool_call (kind=execute) | `tool_use` |
| File change streaming | `item/fileChange/outputDelta` | tool_call (kind=edit) + diff | `tool_use` |
| Stuck detection | N/A (must implement) | `LoopDetected` (built-in) | `error` (warning) |
| Plan streaming | `item/plan/delta` | N/A | N/A |
| Turn lifecycle | `turn/started` / `turn/completed` | `prompt()` return | `result` |
| Thread status | `thread/status/changed` | N/A | N/A |
| Token usage (live) | `thread/tokenUsage/updated` | N/A | N/A |
| Token usage (final) | in `turn/completed` | via telemetry | `result.stats` |
| Sub-agent calls | `collabAgentToolCall` | N/A | N/A |
| Approval needed | `ThreadActiveFlag.waitingOnApproval` | `requestPermission()` | N/A |
| Context compaction | `thread/compacted` | `ChatCompressed` | N/A |
| File system changes | `fs/changed` | N/A | N/A |
| Rate limits | `account/rateLimits/updated` | N/A | N/A |
| Message phase | `"commentary"` vs `"final_answer"` | N/A | N/A |

---

# PART II: CURRENT DAEMON STATE

## 9. What Exists Today

Source: `daemon-v2/crates/triumvirate/src/main.rs` (5,469 lines)

### 9.1 Gemini Runner

`run_gemini_cli_process_with_session()` at lines 1326-1412:
- Spawns: `gemini -o stream-json -p <message> [-r <session_id>]`
- Uses `Command::new(bin).output().await` — **batch mode, all stdout buffered**
- Parses NDJSON, only extracts `{"type":"message","role":"assistant"}` events
- Discards: `init`, `tool_use`, `tool_result`, `error`, `result`
- Returns: `(response_text, Option<session_id>)`

### 9.2 Codex Runner

`run_codex_cli_process_with_session()` at lines 1468-1560:
- Spawns: `codex exec --json --output-last-message <tmpfile> <message>`
- Uses `Command::new(bin).output().await` — **batch mode**
- Reads final response from tmpfile
- Extracts thread_id from JSONL stdout
- Returns: `(response_text, Option<thread_id>)`

### 9.3 Progress Emitter

`ProgressEmitter` at lines 110-154:
- Two MCP channels: `notify_logging_message()` (what Claude sees) + `notify_progress()` (progress token)
- Called with `emitter.emit("message")` throughout execution
- Currently emits: "→ Gemini: sent ✓", "→ Gemini: working... (Ns elapsed)", "→ Gemini: responded ✓"
- Works well — just needs better messages

### 9.4 Heartbeat System

In `execute_ask_agent()` at lines 947-968:
- `tokio::select!` races the agent future against a sleep timer
- Heartbeat starts at 10s, backs off to 40s, 60s
- Emits "working... (Ns elapsed)" via ProgressEmitter
- This becomes the **fallback** when real flow state events are available

### 9.5 Outbox

File-based JSONL log at `~/.triumvirate/outbox.jsonl`:
- `append_outbox_event()` in daemon-core
- `OutboxEvent { ts_ms, request_id, tool, status, agent, detail, cwd, repo, branch }`
- Read via `outbox_recent` MCP tool
- NOT a broadcast channel — write-once append log

### 9.6 Worker Registry

`WorkerState { agent, cwd, session_id, spawn_count, ask_count, last_used_ms }`
- Persisted to `~/.triumvirate/workers.json`
- Manages session_id across calls for multi-turn
- `acquire_worker()` / `require_reused_worker()` / `update_worker_session()` / `dismiss_worker()`

### 9.7 What Does NOT Exist

- No web dashboard (`:8080` serves REST API only, no frontend)
- No broadcast/pub-sub fabric (just JSONL file)
- No stenographer (specced but not implemented)
- No SIGTERM/SIGKILL graceful chain
- No auto-retry on retryable errors
- No model fallback chain
- No Codex thread resume

---

# PART III: DESIGN

## 10. Unified Working State

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkingState {
    TurnStarted,
    Thinking,
    Planning,                           // Codex plan/delta only
    Generating,
    ToolCalling { tool: String, kind: ToolKind },
    ToolRunning { tool: String, detail: String },
    ToolDone { tool: String, success: bool, duration_ms: Option<u64> },
    ExecutingCommand { command: String },
    WritingFile { path: String },
    WaitingForApproval,
    ContextCompacting,
    Stuck { reason: String },
    Error { message: String },
    TurnCompleted { status: String },   // "completed" | "failed" | "interrupted"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolKind {
    Read, Edit, Execute, Search, Fetch, Delete, Move, Think,
    Mcp { server: String },
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingStateEvent {
    pub agent: String,              // "gemini" | "codex"
    pub state: WorkingState,
    pub detail: String,             // human-readable
    pub item_id: Option<String>,
    pub session_name: Option<String>,
    pub turn_id: Option<String>,
    pub ts_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    pub response_text: String,
    pub session_id: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub events: Vec<WorkingStateEvent>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub tool_id: Option<String>,
    pub success: bool,
    pub duration_ms: Option<u64>,
}
```

## 11. Notification-to-WorkingState Mapping

### 11.1 Gemini Stream-JSON → WorkingState

| Event | Condition | WorkingState |
|-------|-----------|-------------|
| `init` | — | `TurnStarted` |
| `message` | `role=assistant, delta=true` | `Generating` |
| `tool_use` | — | `ToolCalling { tool: tool_name, kind: Other }` |
| `tool_result` | `status=success` | `ToolDone { tool, success: true }` |
| `tool_result` | `status=error` | `ToolDone { tool, success: false }` |
| `error` | `severity=warning`, msg contains "Loop" | `Stuck { reason }` |
| `error` | other | `Error { message }` |
| `result` | `status=success` | `TurnCompleted { status: "completed" }` |
| `result` | `status=error` | `TurnCompleted { status: "failed" }` |

### 11.2 Codex App-Server → WorkingState

| Notification | Condition | WorkingState |
|-------------|-----------|-------------|
| `turn/started` | — | `TurnStarted` |
| `item/reasoning/textDelta` | — | `Thinking` |
| `item/reasoning/summaryTextDelta` | — | `Thinking` |
| `item/plan/delta` | — | `Planning` |
| `item/agentMessage/delta` | — | `Generating` |
| `item/started` | `item.type=commandExecution` | `ExecutingCommand { command }` |
| `item/commandExecution/outputDelta` | — | `ExecutingCommand` (streaming) |
| `item/started` | `item.type=fileChange` | `WritingFile { path }` |
| `item/fileChange/outputDelta` | — | `WritingFile` (streaming) |
| `item/started` | `item.type=mcpToolCall` | `ToolCalling { tool, kind: Mcp { server } }` |
| `item/mcpToolCall/progress` | — | `ToolRunning { tool, detail: message }` |
| `item/completed` | `type=mcpToolCall, status=completed` | `ToolDone { success: true, duration_ms }` |
| `item/completed` | `type=mcpToolCall, status=failed` | `ToolDone { success: false }` |
| `item/completed` | `type=commandExecution, status=completed` | `ToolDone { success: true }` |
| `item/completed` | `type=commandExecution, status=failed` | `ToolDone { success: false }` |
| `item/completed` | `type=fileChange, status=completed` | `ToolDone { success: true }` |
| `thread/status/changed` | `type=active, waitingOnApproval` | `WaitingForApproval` |
| `thread/compacted` | — | `ContextCompacting` |
| `error` | — | `Error { message }` |
| `turn/completed` | `status=completed` | `TurnCompleted { status: "completed" }` |
| `turn/completed` | `status=failed` | `TurnCompleted { status: "failed" }` |
| `turn/completed` | `status=interrupted` | `TurnCompleted { status: "interrupted" }` |

### 11.3 Gemini ACP → WorkingState (v2.2 — deferred)

| Session Update | Condition | WorkingState |
|---------------|-----------|-------------|
| `agent_thought_chunk` | — | `Thinking` |
| `agent_message_chunk` | — | `Generating` |
| `tool_call` | `status=in_progress` | `ToolCalling { tool: title, kind: from tool.kind }` |
| `tool_call` | `status=pending` (needs approval) | `WaitingForApproval` |
| `tool_call_update` | `status=completed` | `ToolDone { success: true }` |
| `tool_call_update` | `status=failed` | `ToolDone { success: false }` |

## 12. Stuck Detection

### 12.1 Gemini: Built-In

`GeminiEventType.LoopDetected` is emitted by the agent loop when it detects repetitive behavior. In stream-json mode, this appears as `{"type":"error","severity":"warning","message":"Loop detected, stopping execution"}`.

Triumvirate just needs to map this to `WorkingState::Stuck`.

### 12.2 Codex: Must Implement

Codex has no built-in loop detection. Triumvirate must implement a `StuckDetector`:

**Signals:**
1. No meaningful event for >60s after `turn/started` → Stuck (timeout)
2. Same `ThreadItem.type` + similar `command`/`tool` appears 3+ times in 60s → Stuck (loop)
3. `item/agentMessage/delta` repeats identical content 3+ times → Stuck (content loop)
4. No events for >90s after last `item/started` → Stuck (frozen)

**Action on stuck:**
1. Emit `WorkingState::Stuck { reason }` via event channel
2. ProgressEmitter surfaces to MCP caller: "→ Codex: STUCK — {reason}"
3. Optionally kill subprocess and retry (existing retry loop)

## 13. Cost Model

### 13.1 Gemini

The `result` event in stream-json includes:
```json
"stats": {
  "total_tokens": 100, "input_tokens": 50, "output_tokens": 50,
  "cached": 0, "input": 50, "duration_ms": 1200, "tool_calls": 2,
  "models": { "gemini-2.5-pro": { "total_tokens": 80, ... } }
}
```

Available after turn completion. Per-model breakdown included.

### 13.2 Codex

`thread/tokenUsage/updated` notification includes:
```json
"tokenUsage": {
  "total": { "totalTokens": 1500, "inputTokens": 800, "cachedInputTokens": 0, "outputTokens": 700, "reasoningOutputTokens": 0 },
  "last": { ... },
  "modelContextWindow": 200000
}
```

Available during execution (live updates) and on completion.

### 13.3 Aggregation

Both sources map to `TokenUsage { input_tokens, output_tokens, cached_tokens, reasoning_tokens, total_tokens }`. Stored in `AgentResult.token_usage`. Surfaced through enriched `OutboxEvent` and `StatusResponse`.

---

# PART IV: IMPLEMENTATION

## 14. Architecture

### 14.1 New Crate: `agent-adapter`

```
daemon-v2/crates/agent-adapter/
  src/
    lib.rs          // WorkingState, WorkingStateEvent, AgentResult, format_working_state()
    types.rs        // All type definitions
    gemini.rs       // parse_gemini_stream_json(), GeminiStreamParser
    codex.rs        // parse_codex_exec_jsonl(), CodexAppServerClient
    stuck.rs        // StuckDetector
```

### 14.2 Integration Points

```
execute_ask_agent()
  │
  ├─ spawn agent subprocess
  │   ├─ Gemini: gemini -o stream-json (existing)
  │   └─ Codex: codex exec --json (existing) OR codex app-server --listen stdio:// (new)
  │
  ├─ read stdout line-by-line (.spawn() instead of .output())
  │   ├─ feed each line through GeminiStreamParser or CodexAdapter
  │   └─ parser emits WorkingStateEvent via mpsc channel
  │
  ├─ tokio::select! loop
  │   ├─ agent future completed → break
  │   ├─ heartbeat timer → emit "working... (Ns)" (FALLBACK ONLY)
  │   └─ WorkingStateEvent received → emit via ProgressEmitter (PRIMARY)
  │
  └─ return AgentResult with response + events + token_usage + tool_calls
```

### 14.3 Backward Compatibility

- `ask_session` response format unchanged — final text
- Working state events are additive (progress notifications)
- Env vars for opt-out: `TRIUMVIRATE_GEMINI_STREAMING=false`, `TRIUMVIRATE_CODEX_PROTOCOL=exec`
- If parser returns empty, fall back to raw stdout trimming (current behavior)

## 15. Implementation Phases

### Phase 0: Types + New Crate
- Create `crates/agent-adapter/` with types
- Add to workspace
- Zero behavioral change
- Verify: `cargo test` passes

### Phase 1: Gemini Batch Parser
- Parse all stream-json events from buffered stdout
- Replace manual parse loop in main.rs (~30 lines)
- Enrich AgentResult with tool_calls, token_usage, events
- Fallback to raw stdout if parsed response empty
- Function signature unchanged

### Phase 2: Codex Batch Parser
- Parse `--json` JSONL enrichment data
- Last-message file remains primary response
- Function signature unchanged

### Phase 3: Gemini Live Streaming (THE KEY)
- Switch from `.output()` to `.spawn()` + line-by-line reading
- `GeminiStreamParser::feed_line()` returns `Option<WorkingStateEvent>`
- Events flow via `mpsc` channel to `execute_ask_agent` select loop
- Heartbeat becomes fallback (only fires when stream quiet for 30s)
- Env var escape hatch to batch mode

### Phase 4: Codex App-Server (Opt-In)
- `CodexAppServerClient` speaks JSON-RPC 2.0 over stdio
- Sequence: initialize → thread/start → turn/start → stream notifications
- Opt-in via `TRIUMVIRATE_CODEX_PROTOCOL=app-server`
- Default remains `exec` mode

### Phase 5: Stuck Detection + Cost
- StuckDetector for Codex (Gemini has built-in)
- Token usage aggregation from both agents
- Wire into execute_ask_agent

### Phase 6: Surface It
- `format_working_state()` produces human-readable progress
- Enriched OutboxEvent with working_state, token_usage, tool_name
- ProgressEmitter emits real flow state instead of timers

### Also: ask_twins Removal
- Remove MCP tool, HTTP route, execution function, tests (~750 lines)
- Remove shared types, mcp-bridge helpers
- Clean tool surface

## 16. Dependency Graph

```
Phase 0 (types)
  ├→ Phase 1 (Gemini batch) ──→ Phase 3 (Gemini streaming) ←── THE KEY
  ├→ Phase 2 (Codex batch) ───→ Phase 4 (Codex app-server)
  ├→ Phase 5 (stuck + cost) ─── needs Phase 1 or 2
  └→ Phase 6 (surface it) ───── needs Phase 1 or 2

ask_twins removal — independent, can happen any time
```

**Minimum viable:** Phases 0 + 1 + 2 + 6
**Full release:** Phases 0–6 + ask_twins removal

## 17. Validation Plan

1. Capture live Gemini stream-json trace → commit as test fixture
2. Capture live Codex exec --json trace → commit as test fixture
3. Validate Codex app-server stdio handshake works
4. Unit tests: feed traces through parsers, verify event sequences
5. Integration: MCP call → agent → real-time flow state messages
6. Regression: `cargo test` across workspace

## 18. Risks

| Risk | Probability | Mitigation |
|------|-------------|-----------|
| `.spawn()` changes timeout/error behavior | Medium | Env var fallback to `.output()` batch mode |
| Gemini changes stream-json format | Low | Parser returns empty → fallback to raw stdout |
| Codex app-server stdio doesn't match docs | Medium | Opt-in env var, default is `exec` |
| Streaming subprocess hangs | Medium | `kill_on_drop(true)` + timeout + StuckDetector |
| New crate increases build time | Low | Small crate, few deps, incremental builds |

## 19. Open Questions

1. **Stream-json vs ACP for Gemini v2.1:** Stream-json loses thinking tokens and tool kind classification. Acceptable for v1? (Recommendation: yes — ACP in v2.2)
2. **Codex app-server auth over stdio:** Does it require any auth when using `stdio://`? (Research says no — trusts spawning process)
3. **Multi-turn with app-server:** Can the daemon send multiple `turn/start` requests to the same thread? (Research says yes — thread persists)
4. **Cost of persistent app-server:** Running `codex app-server` uses more resources than `codex exec`. Acceptable? (Recommendation: yes — opt-in, kill on dismiss)

## 20. Deferred to v2.2+

- Gemini ACP mode (thinking tokens, tool kinds, diffs, bidirectional control)
- Codex WebSocket mode (ws:// vs stdio://)
- Dashboard/frontend (flow state visualization)
- Stenographer (requires broadcast fabric)
- Fleet orchestration, debate, governance
- SIGTERM/SIGKILL graceful shutdown chain
- Model fallback chain
- Auto-retry on retryable errors

---

## Appendix A: Source File References

### Gemini CLI
| File | What |
|------|------|
| `gemini-cli/dist/src/gemini.js:352-354` | Mode routing (ACP detection) |
| `gemini-cli/dist/src/config/config.js:508` | ACP flag parsing |
| `gemini-cli/dist/src/acp/acpClient.js:22-34` | ACP bootstrap |
| `gemini-cli/dist/src/acp/acpClient.js:380-740` | Session update emission |
| `gemini-cli/dist/src/acp/acpClient.js:1146-1166` | toAcpToolKind mapping |
| `gemini-cli/dist/src/nonInteractiveCli.js:186-280` | Agent event loop |
| `gemini-cli-core/dist/src/core/turn.js:15-35` | GeminiEventType enum |
| `gemini-cli-core/dist/src/output/types.js:13-21` | JsonStreamEventType enum |
| `gemini-cli-core/dist/src/output/stream-json-formatter.test.js` | Wire format examples |

### Codex CLI
| File | What |
|------|------|
| `docs/v2/codex-protocol/ts/ServerNotification.ts` | Complete notification union |
| `docs/v2/codex-protocol/ts/v2/ThreadItem.ts` | All item types |
| `docs/v2/codex-protocol/ts/v2/TurnStatus.ts` | Turn status enum |
| `docs/v2/codex-protocol/ts/v2/ThreadStatus.ts` | Thread status enum |
| `docs/v2/codex-protocol/ts/v2/ThreadActiveFlag.ts` | Active flag enum |
| `docs/v2/codex-protocol/ts/v2/CommandExecutionStatus.ts` | Command status |
| `docs/v2/codex-protocol/ts/v2/McpToolCallStatus.ts` | MCP tool status |
| `docs/v2/codex-protocol/ts/v2/AgentMessageDeltaNotification.ts` | Message delta shape |
| `docs/v2/codex-protocol/ts/v2/ReasoningTextDeltaNotification.ts` | Reasoning delta shape |
| `docs/v2/codex-protocol/ts/InitializeParams.ts` | Init handshake |
| `docs/v2/codex-protocol/ts/v2/ThreadStartParams.ts` | Thread start |
| `docs/v2/codex-protocol/ts/v2/TurnStartParams.ts` | Turn start |
| `docs/v2/codex-protocol/json-schema/` | Full JSON Schema (all types) |

### Triumvirate Daemon
| File | Lines | What |
|------|-------|------|
| `daemon-v2/crates/triumvirate/src/main.rs` | 110-154 | ProgressEmitter |
| `daemon-v2/crates/triumvirate/src/main.rs` | 268-395 | Worker registry |
| `daemon-v2/crates/triumvirate/src/main.rs` | 516-566 | ask_twins tool (to remove) |
| `daemon-v2/crates/triumvirate/src/main.rs` | 843-1122 | execute_ask_agent + heartbeat |
| `daemon-v2/crates/triumvirate/src/main.rs` | 1176-1439 | execute_ask_twins (to remove) |
| `daemon-v2/crates/triumvirate/src/main.rs` | 1326-1412 | Gemini runner |
| `daemon-v2/crates/triumvirate/src/main.rs` | 1468-1560 | Codex runner |
| `daemon-v2/crates/triumvirate/src/main.rs` | 2158-2181 | ask_twins HTTP route (to remove) |
| `daemon-v2/crates/triumvirate/src/main.rs` | 2512-2524 | Route registration |
| `daemon-v2/crates/shared-types/src/lib.rs` | 30-50 | AskTwins types (to remove) |
| `daemon-v2/crates/mcp-bridge/src/lib.rs` | 21-31, 94-97 | ask_twins helpers (to remove) |
| `daemon-v2/crates/daemon-core/src/lib.rs` | 177-209 | Outbox event I/O |

## Appendix B: Codex Schema Regeneration

The Codex protocol schemas in `docs/v2/codex-protocol/` are auto-generated and should be refreshed when upgrading the Codex binary:

```bash
codex app-server generate-json-schema --out docs/v2/codex-protocol/json-schema
codex app-server generate-ts --out docs/v2/codex-protocol/ts
```
