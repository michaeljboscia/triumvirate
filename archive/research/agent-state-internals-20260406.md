# Agent CLI Internal State — What Gemini and Codex Already Expose

**Date:** 2026-04-06
**Purpose:** Map existing state/event systems in both CLIs to design triumvirate working-state messages
**Verdict:** Both CLIs already emit rich structured events. We don't need to infer state from network traffic — we need to tap the streams they already produce.

---

## Gemini CLI — ACP Protocol Is the Gold Mine

Source: `/opt/homebrew/lib/node_modules/@google/gemini-cli/dist/src/`

### State Enum (Already Defined)

```
StreamingState = {
  "Idle": "idle",
  "Responding": "responding",
  "WaitingForConfirmation": "waiting_for_confirmation"
}
```

File: `ui/types.js:22-27`

### ACP Session Updates (THE KEY)

Gemini CLI implements the **Agent Client Protocol (ACP)** at `acp/acpClient.js`. When running as an ACP agent, it emits structured session updates via `connection.sessionUpdate()`:

| Update Type | What It Means | File:Line |
|------------|---------------|-----------|
| `agent_thought_chunk` | Agent reasoning/thinking | acpClient.js:396 |
| `agent_message_chunk` | Response text streaming | acpClient.js:404 |
| `tool_call` | Tool invocation request | acpClient.js:430,656 |
| `tool_call_update` | Tool completion (pending→in_progress→completed/failed) | acpClient.js:668 |
| `available_commands_update` | Slash commands available | acpClient.js:371 |
| `user_message_chunk` | User input echo | acpClient.js:385 |

**Tool status lifecycle:** `pending` → `in_progress` → `completed` | `failed`

### Non-Interactive Stream Events (--output-format=stream-json)

When run with `--output-format=stream-json`, the CLI emits NDJSON events:

| Event | Meaning | File:Line |
|-------|---------|-----------|
| `INIT` | Session started with model info | nonInteractiveCli.js:140 |
| `MESSAGE` | Text delta (user or assistant) | nonInteractiveCli.js:177 |
| `TOOL_USE` | Tool invoked with name, ID, params | nonInteractiveCli.js:223 |
| `TOOL_RESULT` | Tool done with status/output/error | nonInteractiveCli.js:289 |
| `ERROR` | Error with severity | nonInteractiveCli.js:235 |
| `RESULT` | Final result with metrics/stats/duration | nonInteractiveCli.js:265 |

### Agent Loop Events

From `nonInteractiveCli.js:186-280`, the event types the stream emits:

- `GeminiEventType.Content` — text streaming deltas
- `GeminiEventType.ToolCallRequest` — function call requested
- `GeminiEventType.LoopDetected` — STUCK DETECTION BUILT IN
- `GeminiEventType.MaxSessionTurns` — turn limit hit
- `GeminiEventType.AgentExecutionStopped` — agent stopped
- `GeminiEventType.AgentExecutionBlocked` — agent blocked (e.g. approval needed)

### Telemetry (Already Tracked)

Via `uiTelemetryService` at `SessionContext.js:104-145`:
- API requests/errors/latency
- Token counts (input, output, cached, thoughts)
- Tool call stats (count, success, failure, duration)
- File changes (lines added/removed)

### Hook System

- `SessionStartEvent` before prompt processing (gemini.js:422)
- `SessionEndEvent` on exit (gemini.js:437)

---

## Codex CLI — JSON-RPC WebSocket Protocol

Source: Rust binary with TypeScript API surface. Event protocol visible via compiled strings.

### JSON-RPC 2.0 Notification Methods

Codex uses a **WebSocket-based JSON-RPC protocol** with these notification methods:

#### Thread Lifecycle
| Method | Meaning |
|--------|---------|
| `thread/started` | New conversation started |
| `thread/archived` | Thread archived |
| `thread/unarchived` | Thread restored |
| `thread/closed` | Thread terminated |

#### Turn Lifecycle
| Method | Meaning |
|--------|---------|
| `turn/started` | New turn began |
| `turn/completed` | Turn finished |
| `turn/diff/updated` | Code diff changed |
| `turn/plan/updated` | Execution plan changed |

#### Item-Level Events (MOST GRANULAR)
| Method | Meaning |
|--------|---------|
| `item/started` | New item in turn |
| `item/completed` | Item finished |
| `item/agentMessage/delta` | Text streaming delta |
| `item/plan/delta` | Plan text streaming |
| `item/commandExecution/outputDelta` | Command stdout streaming |
| `item/commandExecution/terminalInteraction` | Terminal I/O event |
| `item/fileChange/outputDelta` | File change streaming |
| `item/mcpToolCall/progress` | MCP tool progress |
| `item/reasoning/summaryTextDelta` | Reasoning summary stream |
| `item/reasoning/summaryPartAdded` | New reasoning section |
| `item/reasoning/textDelta` | Raw reasoning stream |

#### System Events
| Method | Meaning |
|--------|---------|
| `account/updated` | Account state change |
| `account/rateLimits/updated` | Rate limit change |
| `fs/changed` | Filesystem change detected |

### Turn State Machine

From `core/src/state/turn.rs`:
- Fields: `turn.id`, `turn.startedAt`, `turn.completedAt`, `turn.status`
- States: started → active → completed/archived

### MCP Integration

- Full MCP client at `core/src/mcp_connection_manager.rs`
- Uses `rmcp` (Rust MCP SDK) — same crate triumvirate uses!
- Capabilities: Tools, Resources, Prompts, Tasks
- Progress notifications with token tracking

### Exec Mode Constraints

In exec mode, approval requests auto-denied:
- "command execution approval is not supported in exec mode"
- "file change approval is not supported in exec mode"
- "request_user_input is not supported in exec mode"

---

## Synthesis: What Triumvirate Should Tap

### Option A: ACP for Gemini, WebSocket for Codex

Both CLIs have full-featured agent protocols. Triumvirate currently spawns them as subprocesses and reads stdout. Instead:

**Gemini:** Run in ACP mode. Triumvirate acts as ACP host, receives structured session updates:
- `agent_thought_chunk` → `THINKING`
- `tool_call` → `TOOL_CALLING: <name>`
- `tool_call_update` (completed) → `TOOL_DONE: <name>`
- `agent_message_chunk` → `GENERATING`

**Codex:** Connect via WebSocket JSON-RPC. Subscribe to notifications:
- `turn/started` → `TURN_STARTED`
- `item/agentMessage/delta` → `GENERATING`
- `item/commandExecution/outputDelta` → `EXECUTING_COMMAND`
- `item/fileChange/outputDelta` → `WRITING_FILE`
- `item/reasoning/textDelta` → `THINKING`
- `item/mcpToolCall/progress` → `MCP_TOOL_CALL`
- `turn/completed` → `DONE`

### Option B: Stream-JSON for Gemini, Exec Stdout for Codex (Simpler)

**Gemini:** Use `--output-format=stream-json` flag. Parse NDJSON events:
- `INIT` → `SESSION_STARTED`
- `MESSAGE` → `GENERATING`
- `TOOL_USE` → `TOOL_CALLING`
- `TOOL_RESULT` → `TOOL_DONE`
- `RESULT` → `DONE`

**Codex:** Parse `codex exec` structured output. Less rich but works today.

### Option C: Hybrid (Recommended)

1. **Phase 1:** Option B — minimal changes, parse existing output formats
2. **Phase 2:** Option A — full protocol integration for real-time state

---

## Unified Working State Enum for Triumvirate

Mapping both agent's internal states to a single triumvirate vocabulary:

```rust
pub enum WorkingState {
    Spawning,           // Process starting
    Idle,               // Waiting for input
    Thinking,           // Reasoning tokens (Codex: reasoning/textDelta, Gemini: thought_chunk)
    Generating,         // Response tokens (both: message delta)
    ToolCalling(String),// Calling a tool (both: tool_call with name)
    ToolRunning(String),// Tool executing (Gemini: in_progress, Codex: item in progress)
    ToolDone(String),   // Tool completed (both: tool result)
    ReadingCode,        // File read operations
    WritingCode,        // File write operations (Codex: fileChange)
    ExecutingCommand,   // Shell command (Codex: commandExecution)
    WaitingForApproval, // Human-in-the-loop (Gemini: waiting_for_confirmation)
    Stuck,              // Loop detected (Gemini: LoopDetected)
    Error(String),      // Error state
    Done,               // Turn complete
}
```

### What This Means for the Daemon

The triumvirate daemon already has:
- `LifecycleEvent` struct with `state` and `detail` strings
- `ProgressEmitter` for MCP notifications
- Fabric broadcast channel (stenographer listens)
- Outbox event logging

The `WorkingState` enum maps directly to `LifecycleEvent.state` values. Each state change publishes to the fabric, gets logged by the stenographer, and surfaces through the progress emitter to the MCP caller.

**The heartbeat timer ("still working... 10s elapsed") becomes a fallback** — only emitted when no real state signals have been received within the heartbeat interval. State signals are the primary channel.

---

## Key Insight

Neither CLI is a black box. Both were designed for agent-to-agent communication:
- Gemini has **ACP** (Agent Client Protocol) — purpose-built for orchestrators
- Codex has **JSON-RPC WebSocket** — purpose-built for IDE/app integration

Triumvirate is currently treating them as dumb subprocesses (stdin/stdout pipes). The protocols they expose are vastly richer than what we're consuming. The upgrade path is to speak their native protocols instead of scraping stdout.
