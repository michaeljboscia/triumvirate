# PRD — v3.3.0 Live Agent Streaming

**Version:** 3.3.0
**Status:** Goatrodeo PASS (7 rounds, 8 decisions, 38 auto-resolves)
**Spec:** docs/3.3.0/SPEC.md

## Features

### FEAT-001: Agent Stream Event Pipeline
**Priority:** P0 — foundation for all other features
**REQs:** REQ-E01, REQ-E02, REQ-E03, REQ-E04
**User Story:** As a developer using Triumvirate, I want agent tool calls, file reads, and responses to be captured as structured events so that any consumer (watch CLI, dashboard, WebSocket) can display them in real-time.
**Acceptance Criteria:**
- AgentStreamEvent enum exists in shared-types crate with 6 variants (TurnStarted, ToolCall, FileRead, ResponseChunk, TurnCompleted, Error)
- Each event carries a monotonic sequence number
- GeminiStreamParser emits events via tokio mpsc channel during parsing
- CodexExecParser emits events via tokio mpsc channel during parsing
- New execute_ask_agent_streaming() returns (String, mpsc::Receiver<AgentStreamEvent>)
- Old execute_ask_agent() wraps streaming version, collects to blob (adapter pattern)
- All existing callers (ABE, HTTP routes, stdio MCP) continue working unchanged
- WebSocket broadcast emits AgentStreamEvent as `agent_stream` event type
- Existing WS events (token_update, abe_task_state, fleet_progress) unchanged

### FEAT-002: Streamable HTTP MCP Transport
**Priority:** P0 — enables proxy and future Claude Code SSE
**REQs:** REQ-H01, REQ-H02, REQ-H03, REQ-H04, REQ-H05, REQ-H06, REQ-H07, REQ-H08, REQ-H09, REQ-H10
**User Story:** As a developer, I want the daemon to serve MCP tools over Streamable HTTP so that HTTP clients (proxy, future Claude Code) can receive streaming responses via SSE.
**Acceptance Criteria:**
- Daemon serves MCP-compliant Streamable HTTP at POST/GET /mcp
- Uses rmcp transport-streamable-http-server feature (v1.3.0)
- Shares Arc<McpBridge> with existing stdio transport — all 35+ tools available
- Session management via Mcp-Session-Id header
- During tool execution, streams formatted text chunks as SSE frames
- Final tool result as last SSE frame, then stream closes
- Bearer token auth matching existing HTTP API
- Heartbeat events during long generation ("generating response (15s elapsed)")
- Integration tests verify SSE streaming behavior
- Coexists with stdio transport — both active simultaneously

### FEAT-003: Proxy Command
**Priority:** P0 — golden path for Claude Code users
**REQs:** REQ-P01, REQ-P02, REQ-P03, REQ-P04
**User Story:** As a Claude Code user, I want a proxy that bridges my stdio MCP connection to the daemon's HTTP endpoint so that all agent execution flows through the centralized daemon with shared observability.
**Acceptance Criteria:**
- `triumvirate proxy` bridges stdio JSON-RPC ↔ HTTP JSON-RPC to daemon :8080/mcp
- Auto-reconnects with bounded exponential backoff when daemon restarts
- Fails in-flight calls loudly on disconnect, recovers channel for subsequent calls
- If daemon unreachable at startup, retries 5s then exits with clear error message
- No silent fallback to local execution
- Unit tests verify: forwarding, reconnect, clean exit on daemon down

### FEAT-004: Watch CLI
**Priority:** P0 — delivers the README streaming experience TODAY
**REQs:** REQ-W01, REQ-W02, REQ-W03, REQ-W04, REQ-W05, REQ-W06
**User Story:** As a developer, I want to run `triumvirate watch` in a side pane and see every agent action as it happens — tool calls, file reads, responses, token counts — in real-time.
**Acceptance Criteria:**
- `triumvirate watch` connects to daemon WebSocket at /ws
- Pretty-prints AgentStreamEvent: "→ {agent}: {action} ({detail})"
- Default shows agent_stream events only; --all shows everything
- --session <name> filter for specific agent session
- Heartbeat display during long generation with running elapsed timer updated in-place
- Handles daemon-not-running: retry with clear message, not crash
- Detects sequence number gaps, prints "[events skipped, resynced at seq N]"

### FEAT-005: SSE Spike Test
**Priority:** P1 — parallel investigation, doesn't block build
**REQs:** REQ-K01
**User Story:** As the development team, we want to empirically test whether Claude Code renders intermediate SSE frames from MCP servers, because if it does, we can skip the side-pane and deliver streaming inline.
**Acceptance Criteria:**
- 50-line test MCP server using rmcp progress_demo.rs pattern
- Sends 5 SSE notification frames over Streamable HTTP during tool execution
- Registered in Claude Code as a second MCP server
- Tool called from Claude Code session
- Results documented: does Claude Code render intermediate frames? Yes/No + evidence

## Non-Goals (v3.3.0)

- Dashboard UI (GitHub #12 → Pantheon v4.0)
- Streaming for ABE fleet dispatch (headless workers)
- Token-by-token LLM output streaming (agents don't expose this)
- Breaking changes to stdio MCP interface
- Replacing existing WS event schemas
- Pantheon TUI (v4.0)
