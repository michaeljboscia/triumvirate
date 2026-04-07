# Working State Protocol — Triumvirate v2 Spec

**Status:** DRAFT
**Date:** 2026-04-06
**Author:** Claude + Mike (research session)
**Depends on:** CONVERSATIONAL_PARITY.md

---

## 1. Problem Statement

When triumvirate dispatches work to Gemini or Codex via `spawn_session` / `ask_session`, the caller has no visibility into execution state. The current heartbeat system emits dumb timer messages ("still working... 10s elapsed") that carry zero information about what the agent is actually doing.

Both CLI agents have rich internal state/event systems designed for orchestrators. Triumvirate ignores them because it treats the CLIs as dumb subprocesses (stdin/stdout pipes).

## 2. Goal

Replace dumb heartbeats with real working-state signals by integrating with each agent's native protocol. The caller should know — in real time — whether the agent is thinking, generating tokens, calling tools, writing files, executing commands, stuck in a loop, or done.

## 3. Non-Goals

- Modifying Gemini or Codex source code
- Building a general-purpose agent observability platform
- Real-time token streaming to the MCP caller (just state, not content)

---

## 4. Agent Protocol Inventory

### 4.1 Gemini CLI

**Source:** `/opt/homebrew/lib/node_modules/@google/gemini-cli/dist/src/`
**Version:** 0.35.0
**Runtime:** Node.js
**Protocols:** ACP (Agent Client Protocol), Stream-JSON, Interactive (Ink/React)

#### 4.1.1 GeminiEventType (internal stream events)

The core agent loop emits these events via `sendMessageStream()`:

| Value | Constant | Description |
|-------|----------|-------------|
| `content` | `Content` | Text token delta |
| `tool_call_request` | `ToolCallRequest` | Agent wants to call a tool |
| `tool_call_response` | `ToolCallResponse` | Tool returned a result |
| `tool_call_confirmation` | `ToolCallConfirmation` | Tool confirmation dialog |
| `user_cancelled` | `UserCancelled` | User cancelled |
| `error` | `Error` | Error during generation |
| `chat_compressed` | `ChatCompressed` | Context window compacted |
| `thought` | `Thought` | Reasoning/thinking token |
| `max_session_turns` | `MaxSessionTurns` | Turn limit exceeded |
| `finished` | `Finished` | Agent turn complete |
| `loop_detected` | `LoopDetected` | **Stuck detection** — agent is looping |
| `citation` | `Citation` | Source citation |
| `retry` | `Retry` | Retrying after error |
| `context_window_will_overflow` | `ContextWindowWillOverflow` | Context approaching limit |
| `invalid_stream` | `InvalidStream` | Stream parse error |
| `model_info` | `ModelInfo` | Model metadata |
| `agent_execution_stopped` | `AgentExecutionStopped` | Agent stopped (clean exit) |
| `agent_execution_blocked` | `AgentExecutionBlocked` | Agent blocked (needs approval) |

**Source:** `gemini-cli-core/dist/src/core/turn.js:15-35`

#### 4.1.2 Stream-JSON Wire Format (--output-format=stream-json)

NDJSON events on stdout. 6 event types:

```
JsonStreamEventType = { INIT, MESSAGE, TOOL_USE, TOOL_RESULT, ERROR, RESULT }
```

**Exact wire format (from test suite):**

```json
{"type":"init","timestamp":"2025-10-10T12:00:00.000Z","session_id":"test-session-123","model":"gemini-2.0-flash-exp"}
{"type":"message","timestamp":"...","role":"user","content":"What is 2+2?"}
{"type":"message","timestamp":"...","role":"assistant","content":"4","delta":true}
{"type":"tool_use","timestamp":"...","tool_name":"Read","tool_id":"read-123","parameters":{"file_path":"/path/to/file.txt"}}
{"type":"tool_result","timestamp":"...","tool_id":"read-123","status":"success","output":"File contents here"}
{"type":"tool_result","timestamp":"...","tool_id":"read-123","status":"error","error":{"type":"FILE_NOT_FOUND","message":"File not found"}}
{"type":"error","timestamp":"...","severity":"warning","message":"Loop detected, stopping execution"}
{"type":"result","timestamp":"...","status":"success","stats":{"total_tokens":100,"input_tokens":50,"output_tokens":50,"cached":0,"input":50,"duration_ms":1200,"tool_calls":2,"models":{}}}
{"type":"result","timestamp":"...","status":"error","error":{"type":"MaxSessionTurnsError","message":"Maximum session turns exceeded"},"stats":{...}}
```

**Source:** `gemini-cli-core/dist/src/output/stream-json-formatter.test.js`

#### 4.1.3 ACP Session Updates (Agent Client Protocol)

When running in ACP mode (`gemini --acp`), emits structured updates via `connection.sessionUpdate()`:

| sessionUpdate | Fields | Description |
|--------------|--------|-------------|
| `agent_thought_chunk` | `content: { type: "text", text }` | Reasoning/thinking |
| `agent_message_chunk` | `content: { type: "text", text }` | Response text |
| `user_message_chunk` | `content: { type: "text", text }` | User input echo |
| `tool_call` | `toolCallId, status, title, content[], locations, kind` | Tool invocation |
| `tool_call_update` | `toolCallId, status, content[]` | Tool completion |
| `available_commands_update` | `commands[]` | Slash commands |

**Tool call status lifecycle:** `pending` → `in_progress` → `completed` | `failed`

**Tool kind classification** (maps from internal `Kind` enum):

| Kind | ACP Kind | Description |
|------|----------|-------------|
| `Kind.Read` | `"read"` | File read |
| `Kind.Edit` | `"edit"` | File edit/write |
| `Kind.Execute` | `"execute"` | Shell command |
| `Kind.Search` | `"search"` | Search/grep |
| `Kind.Delete` | `"delete"` | File delete |
| `Kind.Move` | `"move"` | File rename/move |
| `Kind.Think` | `"think"` | Internal reasoning |
| `Kind.Fetch` | `"fetch"` | HTTP fetch |
| `Kind.SwitchMode` | `"switch_mode"` | Mode change |
| `Kind.Agent` | `"think"` | Sub-agent |
| `Kind.Plan` | `"other"` | Planning |
| `Kind.Communicate` | `"other"` | Communication |
| `Kind.Other` | `"other"` | Uncategorized |

**Tool call content types:**
```typescript
{ type: "content", content: { type: "text", text: "..." } }
{ type: "diff", path: "file.rs", oldText: "...", newText: "...", _meta: { kind: "add" | "delete" | "modify" } }
```

**Source:** `dist/src/acp/acpClient.js:380-740, 1146-1166`

#### 4.1.4 Telemetry Metrics (available in RESULT event)

```typescript
{
  total_tokens: number,
  input_tokens: number,
  output_tokens: number,
  cached: number,
  input: number,        // input_tokens minus cached
  duration_ms: number,
  tool_calls: number,
  models: {
    [model: string]: {
      total_tokens, input_tokens, output_tokens, cached, input
    }
  }
}
```

---

### 4.2 Codex CLI

**Source:** Rust binary with generated TypeScript/JSON-Schema protocol
**Version:** 0.118.0
**Runtime:** Rust (codex-rs)
**Protocols:** JSON-RPC 2.0 over WebSocket or stdio

#### 4.2.1 Connection

```bash
# WebSocket transport
codex app-server --listen ws://127.0.0.1:9100

# Stdio transport (default)  
codex app-server --listen stdio://

# Generate protocol schemas
codex app-server generate-json-schema --out <dir>
codex app-server generate-ts --out <dir>
```

#### 4.2.2 ServerNotification Methods (complete catalog)

50+ notification methods. Those relevant to working state:

**Turn lifecycle:**

| Method | Params | WorkingState mapping |
|--------|--------|---------------------|
| `turn/started` | `{ threadId, turn: Turn }` | `TurnStarted` |
| `turn/completed` | `{ threadId, turn: Turn }` | `TurnCompleted` |
| `turn/diff/updated` | `{ threadId, turnId }` | (informational) |
| `turn/plan/updated` | `{ threadId, turnId }` | (informational) |

**Item-level events:**

| Method | Params | WorkingState mapping |
|--------|--------|---------------------|
| `item/started` | `{ item: ThreadItem, threadId, turnId }` | Depends on item.type |
| `item/completed` | `{ item: ThreadItem, threadId, turnId }` | Depends on item.type |
| `item/agentMessage/delta` | `{ threadId, turnId, itemId, delta }` | `Generating` |
| `item/plan/delta` | `{ threadId, turnId, itemId, delta }` | `Planning` |
| `item/commandExecution/outputDelta` | `{ threadId, turnId, itemId, delta }` | `ExecutingCommand` |
| `item/commandExecution/terminalInteraction` | (terminal I/O) | `ExecutingCommand` |
| `item/fileChange/outputDelta` | `{ threadId, turnId, itemId, delta }` | `WritingFile` |
| `item/mcpToolCall/progress` | `{ threadId, turnId, itemId, message }` | `ToolRunning` |
| `item/reasoning/textDelta` | `{ threadId, turnId, itemId, delta, contentIndex }` | `Thinking` |
| `item/reasoning/summaryTextDelta` | `{ threadId, turnId, itemId, delta }` | `Thinking` |
| `item/reasoning/summaryPartAdded` | (new section) | `Thinking` |

**Thread-level events:**

| Method | Params | WorkingState mapping |
|--------|--------|---------------------|
| `thread/started` | `{ threadId, ... }` | (lifecycle) |
| `thread/status/changed` | `{ threadId, status: ThreadStatus }` | Status change |
| `thread/tokenUsage/updated` | `{ threadId, ... }` | (metrics) |
| `thread/compacted` | `{ ... }` | `ContextCompacting` |
| `thread/closed` | `{ threadId }` | (lifecycle) |

**System events:**

| Method | Params | WorkingState mapping |
|--------|--------|---------------------|
| `error` | `ErrorNotification` | `Error` |
| `hook/started` | `HookStartedNotification` | (informational) |
| `hook/completed` | `HookCompletedNotification` | (informational) |
| `account/rateLimits/updated` | `...` | (metrics) |
| `model/rerouted` | `ModelReroutedNotification` | (informational) |
| `fs/changed` | `FsChangedNotification` | (informational) |

#### 4.2.3 ThreadItem Types (from item/started and item/completed)

| type | Key Fields | WorkingState mapping |
|------|-----------|---------------------|
| `userMessage` | `id, content: UserInput[]` | N/A |
| `agentMessage` | `id, text, phase: "commentary"\|"final_answer"` | `Generating` |
| `plan` | `id, text` | `Planning` |
| `reasoning` | `id, summary: string[], content: string[]` | `Thinking` |
| `commandExecution` | `id, command, cwd, processId, source, status, commandActions, aggregatedOutput, exitCode, durationMs` | `ExecutingCommand` |
| `fileChange` | `id, changes: FileUpdateChange[], status` | `WritingFile` |
| `mcpToolCall` | `id, server, tool, status, arguments, result, error, durationMs` | `ToolCalling` |
| `dynamicToolCall` | `id, tool, arguments, status, contentItems, success, durationMs` | `ToolCalling` |
| `collabAgentToolCall` | `id, tool, status, senderThreadId, receiverThreadIds, prompt, model, agentsStates` | `SubAgent` |
| `webSearch` | `id, query, action` | `ToolCalling("web_search")` |
| `imageGeneration` | `id, status, revisedPrompt, result` | `ToolCalling("image_gen")` |
| `contextCompaction` | `id` | `ContextCompacting` |

#### 4.2.4 Status Enums

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

---

## 5. Unified Working State Enum

Both agent protocols normalize to this single enum:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkingState {
    TurnStarted,
    Thinking,               // Gemini: thought/agent_thought_chunk, Codex: reasoning/textDelta
    Planning,               // Codex: item/plan/delta, Gemini: N/A
    Generating,             // Gemini: content/agent_message_chunk, Codex: item/agentMessage/delta
    ToolCalling {           // Both: tool start
        tool: String,
        kind: ToolKind,
    },
    ToolRunning {           // Both: tool in progress
        tool: String,
        detail: String,
    },
    ToolDone {              // Both: tool complete
        tool: String,
        success: bool,
        duration_ms: Option<u64>,
    },
    ExecutingCommand {      // Codex: commandExecution, Gemini: tool_call(kind=execute)
        command: String,
    },
    WritingFile {           // Codex: fileChange, Gemini: tool_call(kind=edit)
        path: String,
    },
    WaitingForApproval,     // Codex: ThreadActiveFlag::waitingOnApproval, Gemini: requestPermission
    ContextCompacting,      // Codex: thread/compacted, Gemini: ChatCompressed
    Stuck {                 // Gemini: LoopDetected (built-in), Codex: (must implement)
        reason: String,
    },
    Error {
        message: String,
    },
    TurnCompleted {
        status: String,     // "completed" | "interrupted" | "failed"
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolKind {
    Read,
    Edit,
    Execute,
    Search,
    Fetch,
    Delete,
    Move,
    Think,
    Mcp { server: String },
    Other,
}
```

## 6. Working State Event

Published to the triumvirate fabric broadcast channel:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingStateEvent {
    pub agent: String,              // "gemini" | "codex"
    pub state: WorkingState,
    pub timestamp_ms: u128,
    pub session_name: String,       // triumvirate session name
    pub item_id: Option<String>,    // Codex itemId or Gemini toolCallId
    pub turn_id: Option<String>,
    pub thread_id: Option<String>,  // Codex threadId
}
```

## 7. Integration Architecture

### 7.1 Gemini Adapter

**Recommended path:** Stream-JSON (`--output-format=stream-json`)

- Simpler than ACP (6 event types vs full bidirectional protocol)
- Sufficient for working state (covers token gen, tool calls, errors, completion)
- Does NOT cover: thinking tokens (ACP only), tool kind classification (ACP only)

**Upgrade path:** ACP for full fidelity (thinking, tool kinds, diffs)

**Mapping:**

| Stream-JSON Event | WorkingState |
|-------------------|-------------|
| `init` | `TurnStarted` |
| `message` (role=assistant, delta=true) | `Generating` |
| `tool_use` | `ToolCalling { tool: tool_name, kind: Other }` |
| `tool_result` (status=success) | `ToolDone { tool, success: true }` |
| `tool_result` (status=error) | `ToolDone { tool, success: false }` |
| `error` (severity=warning, msg contains "Loop") | `Stuck { reason }` |
| `error` (other) | `Error { message }` |
| `result` (status=success) | `TurnCompleted { status: "completed" }` |
| `result` (status=error) | `TurnCompleted { status: "failed" }` |

### 7.2 Codex Adapter

**Recommended path:** App-server WebSocket (`codex app-server --listen ws://...`)

- Full JSON-RPC 2.0 protocol
- Auto-generated schemas (`codex app-server generate-json-schema`)
- Uses `rmcp` (same Rust MCP crate as triumvirate)
- Item-level granularity

**Mapping:**

| Codex Notification | WorkingState |
|-------------------|-------------|
| `turn/started` | `TurnStarted` |
| `item/reasoning/textDelta` | `Thinking` |
| `item/reasoning/summaryTextDelta` | `Thinking` |
| `item/plan/delta` | `Planning` |
| `item/agentMessage/delta` | `Generating` |
| `item/started` (type=commandExecution) | `ExecutingCommand { command }` |
| `item/commandExecution/outputDelta` | `ExecutingCommand { command }` (streaming) |
| `item/started` (type=fileChange) | `WritingFile { path }` |
| `item/fileChange/outputDelta` | `WritingFile { path }` (streaming) |
| `item/started` (type=mcpToolCall) | `ToolCalling { tool, kind: Mcp { server } }` |
| `item/mcpToolCall/progress` | `ToolRunning { tool, detail: message }` |
| `item/completed` (type=mcpToolCall, status=completed) | `ToolDone { tool, success: true, duration_ms }` |
| `item/completed` (type=mcpToolCall, status=failed) | `ToolDone { tool, success: false }` |
| `item/completed` (type=commandExecution, status=completed) | `ToolDone { tool: command, success: true }` |
| `item/completed` (type=commandExecution, status=failed) | `ToolDone { tool: command, success: false }` |
| `item/completed` (type=fileChange, status=completed) | `ToolDone { tool: "file_change", success: true }` |
| `thread/status/changed` (type=active, waitingOnApproval) | `WaitingForApproval` |
| `thread/compacted` | `ContextCompacting` |
| `error` | `Error { message }` |
| `turn/completed` (status=completed) | `TurnCompleted { status: "completed" }` |
| `turn/completed` (status=failed) | `TurnCompleted { status: "failed" }` |
| `turn/completed` (status=interrupted) | `TurnCompleted { status: "interrupted" }` |

### 7.3 Stuck Detection (Codex — must implement)

Gemini has built-in `LoopDetected`. Codex does not. Implement in the adapter:

1. Track `item/started` events in a sliding window
2. If same `ThreadItem.type` + similar `command`/`tool` appears 3+ times in 60s → `Stuck`
3. If no events received for 90s after last `item/started` → `Stuck` (timeout)
4. If `item/agentMessage/delta` repeats identical content 3+ times → `Stuck` (loop)

## 8. Daemon Integration Points

### 8.1 New Crate: `agent-adapter`

```
daemon-v2/crates/agent-adapter/
  src/
    lib.rs          // WorkingState, WorkingStateEvent, ToolKind
    gemini.rs       // GeminiAdapter (stream-json parser)
    codex.rs        // CodexAdapter (WebSocket JSON-RPC client)
    stuck.rs        // StuckDetector (Codex)
```

### 8.2 Fabric Integration

Each adapter publishes `WorkingStateEvent` to the fabric broadcast channel. Existing subscribers (stenographer, progress emitter) automatically receive them.

### 8.3 Progress Emitter Enhancement

Current: "→ Gemini: working... (10s elapsed)"
New: "→ Gemini: calling Read tool (src/main.rs)" / "→ Codex: executing `cargo test`"

The heartbeat timer becomes a **fallback** — only emitted when no real state events have been received within the heartbeat interval.

### 8.4 Session JSONL Enhancement

Stenographer writes `WorkingStateEvent` to the session JSONL alongside existing fabric events. This gives full replay capability for debugging agent behavior.

## 9. Implementation Phases

| Phase | Scope | Effort |
|-------|-------|--------|
| 1 | Define `WorkingState` types in `agent-adapter` crate | Small |
| 2 | Implement `GeminiAdapter` (stream-json parser) | Medium |
| 3 | Implement `CodexAdapter` (WebSocket client) | Medium-Large |
| 4 | Integrate adapters into daemon session management | Medium |
| 5 | Stuck detection for Codex | Small |
| 6 | Progress emitter enhancement | Small |
| 7 | Dashboard working state display | Future |

## 10. Open Questions

1. **ACP vs stream-json for Gemini:** Stream-JSON is simpler but loses thinking tokens and tool kind classification. Is that acceptable for v1?
2. **Codex WebSocket auth:** Does `codex app-server` require auth on the WebSocket? Need to test.
3. **Multiple turns:** Both protocols support multi-turn conversations. How does this map to `ask_session` which currently expects one response per call?
4. **Resource cost:** Running `codex app-server` as a persistent WebSocket server uses more resources than `codex exec`. Acceptable?
5. **Backward compatibility:** The `ask_session` response format stays the same (final text). Working state events are additive (progress notifications). No breaking changes.

---

## Appendix A: File References

| File | What |
|------|------|
| `gemini-cli-core/dist/src/core/turn.js:15-35` | GeminiEventType enum |
| `gemini-cli-core/dist/src/output/types.js:13-21` | JsonStreamEventType enum |
| `gemini-cli/dist/src/acp/acpClient.js:380-740` | ACP session update emission |
| `gemini-cli/dist/src/acp/acpClient.js:1146-1166` | toAcpToolKind mapping |
| `gemini-cli/dist/src/nonInteractiveCli.js:186-280` | Stream event processing loop |
| `/tmp/codex-ts/ServerNotification.ts` | Complete Codex notification union |
| `/tmp/codex-ts/v2/ThreadItem.ts` | Complete Codex item types |
| `/tmp/codex-ts/v2/TurnStatus.ts` | Turn status enum |
| `/tmp/codex-ts/v2/ThreadStatus.ts` | Thread status enum |
| `/tmp/codex-ts/v2/ThreadActiveFlag.ts` | Active flag enum |
| `/tmp/codex-ts/v2/CommandExecutionStatus.ts` | Command status enum |
| `/tmp/codex-ts/v2/McpToolCallStatus.ts` | MCP tool status enum |
| `/tmp/codex-schema/` | Full JSON Schema for all types |

## Appendix B: Codex Schema Generation

The Codex protocol is fully self-documenting:

```bash
# Generate JSON Schema (for validation)
codex app-server generate-json-schema --out /path/to/schemas

# Generate TypeScript types (for reference)
codex app-server generate-ts --out /path/to/types
```

These generated files should be committed to `triumvirate/docs/v2/codex-protocol/` for reference.
