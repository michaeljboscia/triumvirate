# Agent Protocol Deep Dive — Exact Wire Formats for Triumvirate Integration

**Date:** 2026-04-06
**Status:** DEFINITIVE — read the actual source, generated the actual schemas
**Files examined:** Gemini CLI ACP source, Codex app-server generated TypeScript + JSON Schema

---

## Codex: App-Server WebSocket Protocol (JSON-RPC 2.0)

### How to Connect

```bash
# Start Codex as a WebSocket app-server
codex app-server --listen ws://127.0.0.1:9100

# Or over stdio (default)
codex app-server --listen stdio://
```

Generated schemas at: `codex app-server generate-json-schema --out <dir>`
Generated TS types at: `codex app-server generate-ts --out <dir>`

### Thread State Machine

```
ThreadStatus = 
  | { type: "notLoaded" }
  | { type: "idle" }  
  | { type: "systemError" }
  | { type: "active", activeFlags: ThreadActiveFlag[] }

ThreadActiveFlag = "waitingOnApproval" | "waitingOnUserInput"

TurnStatus = "completed" | "interrupted" | "failed" | "inProgress"
```

### Complete Notification Catalog (50+ methods)

#### Turn Lifecycle (triumvirate MUST subscribe)
| Method | Params | Maps to WorkingState |
|--------|--------|---------------------|
| `turn/started` | `{ threadId, turn: Turn }` | `Spawning` → `Thinking` |
| `turn/completed` | `{ threadId, turn: Turn }` | `Done` |
| `turn/diff/updated` | `{ threadId, turnId }` | (informational) |
| `turn/plan/updated` | `{ threadId, turnId }` | (informational) |

#### Item Lifecycle (granular — per tool call, per message)
| Method | Params | Maps to WorkingState |
|--------|--------|---------------------|
| `item/started` | `{ item: ThreadItem, threadId, turnId }` | Depends on item.type |
| `item/completed` | `{ item: ThreadItem, threadId, turnId }` | Depends on item.type |
| `item/agentMessage/delta` | `{ threadId, turnId, itemId, delta }` | `Generating` |
| `item/plan/delta` | `{ threadId, turnId, itemId, delta }` | `Thinking` (planning) |
| `item/commandExecution/outputDelta` | `{ threadId, turnId, itemId, delta }` | `ExecutingCommand` |
| `item/commandExecution/terminalInteraction` | (terminal I/O) | `ExecutingCommand` |
| `item/fileChange/outputDelta` | `{ threadId, turnId, itemId, delta }` | `WritingCode` |
| `item/mcpToolCall/progress` | `{ threadId, turnId, itemId, message }` | `ToolCalling(mcp)` |
| `item/reasoning/textDelta` | `{ threadId, turnId, itemId, delta, contentIndex }` | `Thinking` |
| `item/reasoning/summaryTextDelta` | `{ threadId, turnId, itemId, delta }` | `Thinking` |
| `item/reasoning/summaryPartAdded` | (new reasoning section) | `Thinking` |

#### Thread Items (what you get in item/started and item/completed)

```typescript
ThreadItem = 
  | { type: "userMessage", id, content: UserInput[] }
  | { type: "agentMessage", id, text, phase, memoryCitation }
  | { type: "plan", id, text }
  | { type: "reasoning", id, summary: string[], content: string[] }
  | { type: "commandExecution", id, command, cwd, processId, source, status, 
      commandActions, aggregatedOutput, exitCode, durationMs }
  | { type: "fileChange", id, changes: FileUpdateChange[], status }
  | { type: "mcpToolCall", id, server, tool, status, arguments, result, error, durationMs }
  | { type: "dynamicToolCall", id, tool, arguments, status, contentItems, success, durationMs }
  | { type: "collabAgentToolCall", id, tool, status, senderThreadId, receiverThreadIds, 
      prompt, model, reasoningEffort, agentsStates }
  | { type: "webSearch", id, query, action }
  | { type: "imageGeneration", id, status, revisedPrompt, result }
  | { type: "contextCompaction", id }
```

#### Status Types
```typescript
CommandExecutionStatus  // for shell commands
McpToolCallStatus       // for MCP tools  
PatchApplyStatus        // for file changes
DynamicToolCallStatus   // for dynamic tools
CollabAgentToolCallStatus // for sub-agent calls
```

### What Triumvirate Needs to Do for Codex

1. Start `codex app-server --listen ws://127.0.0.1:<port>` as subprocess
2. Connect via WebSocket (JSON-RPC 2.0)
3. Send `initialize` → get capabilities
4. Send `thread/start` → get threadId
5. Send `turn/start` with prompt → get turnId  
6. Subscribe to all server notifications
7. Map notifications to WorkingState events on the fabric

---

## Gemini: Two Integration Paths

### Path A: ACP (Agent Client Protocol) — Full Featured

```bash
# Gemini CLI runs as ACP agent over NDJSON stdio
# Triumvirate acts as ACP host
gemini --acp  # (or however ACP mode is invoked)
```

Protocol: NDJSON over stdio (`acp.ndJsonStream(stdout, stdin)`)
SDK: `@agentclientprotocol/sdk` v0.12.0

#### Session Update Types (what Gemini emits)

```typescript
// Agent thinking (reasoning tokens)
{ sessionUpdate: "agent_thought_chunk", content: { type: "text", text: "**subject**\ndescription" } }

// Agent responding (output tokens)  
{ sessionUpdate: "agent_message_chunk", content: { type: "text", text: "..." } }

// Tool invocation started (auto-approved)
{ sessionUpdate: "tool_call", toolCallId, status: "in_progress", title, content: [], 
  locations, kind: "read" | "edit" | "command" | "other" }

// Tool requiring approval
// → connection.requestPermission() → returns outcome

// Tool completed
{ sessionUpdate: "tool_call_update", toolCallId, status: "completed" | "failed", content: [...] }

// User message echo
{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "..." } }

// Available slash commands
{ sessionUpdate: "available_commands_update", commands: [...] }
```

#### Tool Call Content Types
```typescript
// Text result
{ type: "content", content: { type: "text", text: "..." } }

// Diff result (file edits)
{ type: "diff", path: "file.rs", oldText: "...", newText: "...", 
  _meta: { kind: "add" | "delete" | "modify" } }
```

#### Tool Kind Classification
```
"read" | "edit" | "command" | "other"
```

#### Agent Events (from stream)
| Event | Meaning | Maps to WorkingState |
|-------|---------|---------------------|
| `StreamEventType.CHUNK` + `part.thought=true` | Reasoning | `Thinking` |
| `StreamEventType.CHUNK` + `part.text` | Response text | `Generating` |
| `StreamEventType.CHUNK` + `functionCalls` | Tool requests | `ToolCalling` |
| `GeminiEventType.LoopDetected` | Stuck! | `Stuck` |
| `GeminiEventType.AgentExecutionStopped` | Done/stopped | `Done` |
| `GeminiEventType.AgentExecutionBlocked` | Needs approval | `WaitingForApproval` |

### Path B: Stream JSON (--output-format=stream-json) — Simpler

```bash
gemini --output-format=stream-json -p "your prompt"
```

NDJSON events on stdout:

```json
{"type":"INIT","timestamp":"...","session_id":"...","model":"gemini-2.5-pro"}
{"type":"MESSAGE","timestamp":"...","role":"user","content":"your prompt"}
{"type":"MESSAGE","timestamp":"...","role":"assistant","content":"partial","delta":true}
{"type":"TOOL_USE","timestamp":"...","tool_name":"ReadFile","tool_id":"abc","parameters":{...}}
{"type":"TOOL_RESULT","timestamp":"...","tool_id":"abc","status":"success","output":"..."}
{"type":"ERROR","timestamp":"...","severity":"warning","message":"Loop detected"}
{"type":"RESULT","timestamp":"...","status":"success","stats":{...}}
```

---

## Protocol Comparison Matrix

| Capability | Codex WebSocket | Gemini ACP | Gemini Stream-JSON |
|-----------|----------------|-----------|-------------------|
| Thinking/reasoning deltas | `item/reasoning/textDelta` | `agent_thought_chunk` | N/A |
| Response text deltas | `item/agentMessage/delta` | `agent_message_chunk` | `MESSAGE` (delta) |
| Tool call start | `item/started` (type=mcpToolCall) | `tool_call` (in_progress) | `TOOL_USE` |
| Tool call complete | `item/completed` (type=mcpToolCall) | `tool_call_update` (completed) | `TOOL_RESULT` |
| Command execution | `item/commandExecution/outputDelta` | tool_call (kind=command) | `TOOL_USE` |
| File changes | `item/fileChange/outputDelta` | tool_call (kind=edit) + diff | `TOOL_USE` |
| Stuck detection | N/A (must implement) | `LoopDetected` (built-in!) | `ERROR` (warning) |
| Plan streaming | `item/plan/delta` | N/A | N/A |
| Turn lifecycle | `turn/started` / `turn/completed` | prompt() return | `RESULT` |
| Thread status | `thread/status/changed` | N/A | N/A |
| Token usage | `thread/tokenUsage/updated` | via telemetry | via stats |
| Sub-agent calls | `collabAgentToolCall` (!) | N/A | N/A |
| Approval needed | `ThreadActiveFlag.waitingOnApproval` | `requestPermission()` | N/A (blocked) |
| Context compaction | `thread/compacted` | N/A | N/A |
| File system changes | `fs/changed` | N/A | N/A |
| Rate limits | `account/rateLimits/updated` | N/A | N/A |

---

## Triumvirate Integration Architecture

### Phase 1: Dual Protocol Adapter

```
┌─────────────────────────────────────────────┐
│           Triumvirate Daemon                 │
│                                              │
│  ┌──────────────┐    ┌──────────────────┐   │
│  │ GeminiAdapter │    │  CodexAdapter    │   │
│  │ (ACP Host)    │    │  (WS Client)    │   │
│  │               │    │                  │   │
│  │ NDJSON stdio  │    │ JSON-RPC 2.0    │   │
│  │ ↕             │    │ ↕               │   │
│  │ gemini --acp  │    │ codex app-server│   │
│  └──────┬───────┘    └──────┬──────────┘   │
│         │                    │               │
│         ▼                    ▼               │
│  ┌─────────────────────────────────────┐    │
│  │     Unified WorkingState Stream     │    │
│  │                                      │    │
│  │  Both adapters normalize events to:  │    │
│  │  WorkingState { agent, state, detail,│    │
│  │                 timestamp, item_id } │    │
│  └──────────────┬──────────────────────┘    │
│                  │                           │
│         ┌───────┴───────┐                   │
│         ▼               ▼                   │
│  ┌────────────┐  ┌──────────────┐          │
│  │  Fabric    │  │  Progress    │          │
│  │  Broadcast │  │  Emitter     │          │
│  │  (all sub) │  │  (MCP caller)│          │
│  └────────────┘  └──────────────┘          │
└─────────────────────────────────────────────┘
```

### Rust Types for the Adapter

```rust
/// Unified working state — both adapters emit this
pub struct WorkingStateEvent {
    pub agent: AgentName,        // Gemini | Codex
    pub state: WorkingState,
    pub detail: String,          // Human-readable context
    pub item_id: Option<String>, // Codex itemId or Gemini toolCallId
    pub turn_id: Option<String>,
    pub timestamp: u128,
}

pub enum AgentName { Gemini, Codex }

pub enum WorkingState {
    TurnStarted,
    Thinking,            // reasoning tokens
    Planning,            // plan deltas (Codex only)
    Generating,          // response tokens
    ToolCalling(String), // tool name
    ToolRunning(String), // tool executing
    ToolDone(String),    // tool completed
    ExecutingCommand(String), // shell command
    WritingFile(String),     // file path
    WaitingForApproval,
    ContextCompacting,   // Codex only
    Stuck(String),       // reason
    Error(String),
    TurnCompleted,
}
```

### Key Implementation Notes

1. **Codex uses `rmcp`** — the exact same Rust MCP crate triumvirate already depends on. The WebSocket client can be built with `tokio-tungstenite` or the `rmcp` transport layer.

2. **Gemini ACP is NDJSON over stdio** — trivially parsed. The `@agentclientprotocol/sdk` defines the protocol, but triumvirate only needs to parse the NDJSON lines from stdout.

3. **Both protocols are bidirectional** — triumvirate can not only observe but also send commands (approve tool calls, interrupt turns, steer agents).

4. **Codex generates its own schemas** — `codex app-server generate-json-schema` produces JSON Schema for every request, response, and notification. This can feed directly into Rust type generation via `schemars` or `typify`.

5. **Gemini has built-in stuck detection** (`LoopDetected`) — Codex doesn't. Triumvirate should implement its own stuck detector for Codex based on repeated item patterns or no-progress timeouts.

---

## What Changes in the Daemon

### Current (v1 — subprocess pipes)
```
spawn_session → PTY subprocess → read stdout → accumulate → return final text
```

### Proposed (v2 — native protocols)
```
spawn_session("gemini") → start gemini --acp → GeminiAdapter reads NDJSON → 
  emits WorkingStateEvents → fabric → stenographer + progress emitter

spawn_session("codex") → start codex app-server --listen ws://... → 
  CodexAdapter connects WS → subscribes notifications → 
  emits WorkingStateEvents → fabric → stenographer + progress emitter
```

### Backwards Compatibility

The `ask_session` MCP tool response stays the same — final text. But **during** execution, the progress notifications now carry real state:

```
→ Gemini: thinking (reasoning about authentication middleware)
→ Gemini: tool_call: ReadFile (src/auth/middleware.rs)  
→ Gemini: tool_done: ReadFile (247 lines)
→ Gemini: generating (writing response)
→ Gemini: done
```

Instead of:

```
→ Gemini: working... (10s elapsed)
→ Gemini: working... (20s elapsed)
→ Gemini: done
```
