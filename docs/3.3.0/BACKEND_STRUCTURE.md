# BACKEND_STRUCTURE — v3.3.0 Live Agent Streaming

**Version:** 3.3.0

## New Types (shared-types crate)

### AgentStreamEvent

```rust
/// Structured event emitted during agent execution.
/// Consumed by: WebSocket broadcast, watch CLI, future dashboard.
/// Serialized as JSON with `event_type` discriminator.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event_type")]
pub enum AgentStreamEvent {
    TurnStarted {
        agent: String,
        session_name: String,
        seq: u64,
    },
    ToolCall {
        agent: String,
        tool_name: String,
        args_summary: String,
        seq: u64,
    },
    FileRead {
        agent: String,
        file_path: String,
        seq: u64,
    },
    ResponseChunk {
        agent: String,
        text_preview: String,
        seq: u64,
    },
    TurnCompleted {
        agent: String,
        tokens_in: i64,
        tokens_out: i64,
        cached_tokens: Option<i64>,
        tool_count: i64,
        duration_ms: u64,
        seq: u64,
    },
    Error {
        agent: String,
        message: String,
        seq: u64,
    },
}
```

### Sequence Number Generator

```rust
/// Global atomic counter for monotonic event ordering.
/// Lives in daemon-core, shared via Arc.
pub struct EventSequencer {
    counter: AtomicU64,
}

impl EventSequencer {
    pub fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }
}
```

## Modified Types

### agent-adapter crate

**GeminiStreamParser** — currently returns `String` (final response). Modified to also push `AgentStreamEvent` values to an `mpsc::Sender<AgentStreamEvent>` during parsing. Events emitted:
- `TurnStarted` — on first NDJSON line
- `ToolCall` — when tool_call event detected in stream
- `FileRead` — when read_file tool call detected (subset of ToolCall)
- `ResponseChunk` — periodic during response generation (every 2s or on significant content)
- `TurnCompleted` — after final stats line parsed
- `Error` — on process exit with non-zero code

**CodexExecParser** — same modification pattern as GeminiStreamParser, adapted for Codex exec JSON format.

### Executor (triumvirate binary)

**New function:**
```rust
pub async fn execute_ask_agent_streaming(
    req: AskAgentRequest,
    sequencer: Arc<EventSequencer>,
) -> anyhow::Result<(String, mpsc::Receiver<AgentStreamEvent>)>
```

**Existing function (adapter):**
```rust
pub async fn execute_ask_agent(
    req: AskAgentRequest,
) -> anyhow::Result<String> {
    let (result, _rx) = execute_ask_agent_streaming(req, sequencer).await?;
    // rx is dropped — events go nowhere for non-streaming callers
    Ok(result)
}
```

### daemon-core ObservabilityBus

**Modified:** `publish_event` gains a new overload or the bus subscribes to the agent event mpsc channel and re-broadcasts each AgentStreamEvent to the WS broadcast channel as `agent_stream` event type. Existing event types unchanged.

## New API Endpoints

### POST /mcp — Streamable HTTP MCP

**Request:** JSON-RPC 2.0 (Content-Type: application/json)
**Response:** `text/event-stream` (SSE) OR `application/json`

SSE stream for tool calls (formatted text chunks as partial tool_result content):
```
data: {"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"→ Gemini: turn started\n"}],"isError":false},"_partial":true}

data: {"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"→ Gemini: calling read_file (src/auth.rs)\n"}],"isError":false},"_partial":true}

data: {"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"→ Gemini: generating response (5s elapsed)\n"}],"isError":false},"_partial":true}

data: {"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"The auth middleware has three issues..."}],"isError":false}}
```

Note: Intermediate frames use `_partial: true` to indicate more data follows. The final frame omits `_partial`. If the MCP client does not support partial frames, it buffers until the final frame — same as current blob behavior.

**Auth:** Bearer token (same as existing HTTP API)
**Session:** Mcp-Session-Id header returned on initialize

### GET /mcp — SSE Notification Stream

**Request:** Accept: text/event-stream
**Response:** Long-lived SSE connection for server-initiated notifications
**Auth:** Bearer token

## New CLI Commands

### triumvirate proxy

**Process model:** Spawned by Claude Code as stdio subprocess
**Upstream:** stdin/stdout JSON-RPC (to Claude Code)
**Downstream:** HTTP POST to http://127.0.0.1:8080/mcp (to daemon)
**Reconnect:** Bounded exponential backoff (100ms → 200ms → 400ms → ... → 5s cap)
**Startup:** Try connect for 5s, exit with error if unreachable
**In-flight failure:** Return JSON-RPC error to Claude Code, reconnect for next call

### triumvirate watch

**Process model:** User-launched, long-running
**Connection:** WebSocket to ws://127.0.0.1:8080/ws
**Filter:** Default: agent_stream events. --all: all event types. --session <name>: specific session.
**Display:** Format AgentStreamEvent → "→ {agent}: {action} ({detail})"
**Heartbeat:** During TurnStarted without TurnCompleted, show elapsed timer updated in-place via crossterm
**Reconnect:** Retry on disconnect with clear message
**Gap detection:** Track seq numbers, print warning on gaps

## Crate Dependency Graph (v3.3.0 changes in bold)

```
triumvirate (binary)
├── shared-types          ← AgentStreamEvent enum
├── daemon-core           ← EventSequencer, ObservabilityBus changes
├── daemon-http           ← unchanged
├── mcp-bridge            ← unchanged
├── mcp-tools             ← unchanged
├── agent-adapter         ← parser mpsc channel changes
├── agent-worker          ← unchanged
├── token-economics       ← unchanged
├── fleet                 ← unchanged
├── peer-review           ← unchanged
├── ledger                ← unchanged
├── fallback-outbox       ← unchanged
└── NEW: tokio-tungstenite, crossterm (watch CLI deps)
```
