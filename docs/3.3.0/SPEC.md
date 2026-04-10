# v3.3.0 — Live Agent Streaming

**Version:** 3.3.0
**Working Directory:** /Users/mikeboscia/projects/triumvirate
**Git Branch:** main (will branch to v3.3.0 for build)

## Problem

The daemon captures real-time streaming data from agents (tool calls, file reads, response chunks, token counts) but none of it is visible to the user during execution. When the user calls `ask_session` or `ask_agent`, they stare at a spinner for 30 seconds, then get a wall of text. The README promises live streaming visibility. The product doesn't deliver it.

## Goal

When the user asks an agent a question through the daemon, they see each step as it happens:

```
→ Gemini: turn started
→ Gemini: calling read_file (src/middleware/auth.rs)
→ Gemini: calling read_file (src/middleware/jwt.rs)
→ Gemini: generating response
→ Gemini: responded (12,847 in / 1,203 out / 8,400 cached, 2 tools, 4.1s)
"The auth middleware has three issues..."
```

Not after. During.

## Requirements

### Phase 1: MCP Progress Notifications (stdio transport)

**REQ-S01:** During `ask_session` tool execution, the daemon MUST emit MCP `notifications/progress` messages for each streaming event parsed from the agent subprocess. Events include: turn started, tool call, file read, response generation, response complete.

**REQ-S02:** Each progress notification MUST include a human-readable `message` field describing the current agent activity. Format: `"{agent}: {action} ({detail})"`. Example: `"Gemini: calling read_file (src/auth.rs)"`.

**REQ-S03:** Each progress notification MUST include numeric `progress` and `total` fields representing elapsed events vs estimated total events for the turn. If total is unknown, `total` MUST be omitted (not set to 0).

**REQ-S04:** During `ask_agent` tool execution, the daemon MUST emit the same progress notifications as `ask_session`.

**REQ-S05:** Progress notifications MUST use the `progressToken` from the client's original tool call request `_meta` field. If the client does not send a `progressToken`, the daemon MUST NOT emit progress notifications for that call.

**REQ-S06:** Progress notifications MUST be sent via `context.peer.notify_progress()` (rmcp API). No custom JSON-RPC messages. No stderr hacks.

**REQ-S07:** The `GeminiStreamParser` MUST emit structured events that the progress notification layer can consume. Events: `TurnStarted`, `ToolCall { name, args_summary }`, `FileRead { path }`, `ResponseChunk { preview }`, `TurnCompleted { tokens_in, tokens_out, cached, tool_count, duration_ms }`.

**REQ-S08:** The `CodexExecParser` MUST emit the same structured event types as GeminiStreamParser, adapted for Codex's exec JSON format.

**REQ-S09:** If Claude Code does not render progress notifications in the current version, the feature MUST degrade gracefully — the tool call completes normally with the final response. No errors. No behavior change for clients that don't support progress.

**REQ-S10:** Unit tests MUST verify that the progress notification layer emits the correct events for a mocked agent stream. At least 5 test cases: turn started, tool call, file read, response chunk, turn completed.

### Phase 2: Streamable HTTP Transport

**REQ-H01:** The daemon MUST expose an MCP-compliant Streamable HTTP endpoint at `POST /mcp` that accepts JSON-RPC tool call requests and can return responses as SSE streams.

**REQ-H02:** The daemon MUST expose a `GET /mcp` endpoint for clients to establish an SSE connection for server-initiated notifications (progress updates, resource changes).

**REQ-H03:** The Streamable HTTP endpoint MUST support session management via `Mcp-Session-Id` header. The daemon generates a session ID on first `initialize` request and returns it in the response header.

**REQ-H04:** The Streamable HTTP transport MUST coexist with the existing stdio transport. Both transports MUST be active simultaneously. Claude Code config determines which transport a given client uses.

**REQ-H05:** During tool execution over Streamable HTTP, the daemon MUST stream progress events as SSE `data:` frames containing JSON-RPC `notifications/progress` messages. One frame per agent streaming event.

**REQ-H06:** The final tool result MUST be sent as the last SSE frame, then the SSE stream for that request MUST close.

**REQ-H07:** The daemon MUST use rmcp's `transport-streamable-http-server` feature for the Streamable HTTP implementation. No custom SSE framing.

**REQ-H08:** Claude Code configuration for Streamable HTTP MUST work with: `claude mcp add --transport http triumvirate http://127.0.0.1:8080/mcp`. The daemon MUST handle this configuration correctly.

**REQ-H09:** The Streamable HTTP endpoint MUST enforce bearer token authentication identical to the existing HTTP API. Unauthenticated requests receive 401.

**REQ-H10:** Integration tests MUST verify SSE streaming behavior: connect to `/mcp` via GET, call a tool via POST, receive at least 2 SSE progress frames before the final result frame.

### Streaming Event Schema

**REQ-E01:** All streaming events MUST conform to a shared `AgentStreamEvent` enum with these variants: `TurnStarted { agent, session_name }`, `ToolCall { agent, tool_name, args_summary }`, `FileRead { agent, file_path }`, `ResponseChunk { agent, text_preview }`, `TurnCompleted { agent, tokens_in, tokens_out, cached_tokens, tool_count, duration_ms }`, `Error { agent, message }`.

**REQ-E02:** The `AgentStreamEvent` enum MUST be defined in the `shared-types` crate so both the MCP progress layer and the HTTP SSE layer consume the same type.

**REQ-E03:** The existing `GeminiStreamParser` and `CodexExecParser` in the `agent-adapter` crate MUST be modified to produce `AgentStreamEvent` values via a `tokio::sync::mpsc` channel during stream parsing.

**REQ-E04:** The existing WebSocket broadcast (`/ws`) MUST also emit `AgentStreamEvent` values, maintaining backwards compatibility with any WebSocket consumers.

## Non-Goals

- Dashboard UI (tracked in GitHub #12)
- Streaming for ABE fleet dispatch (separate feature — ABE workers are headless)
- Token-by-token LLM output streaming (agent CLIs don't expose this granularity)
- Breaking change to existing stdio MCP interface

## Dependencies

- `rmcp` crate v1.3+ (already in deps — need to add `transport-streamable-http-server` feature)
- Claude Code must support either progress notifications (Path 1) or HTTP MCP transport (Path 2)
- Existing `GeminiStreamParser` and `CodexExecParser` in `agent-adapter` crate

## Prior Art

- `danny-avila/Example-MCP-Server` — working progress notifications over Streamable HTTP
- FastMCP (Python) — `ctx.report_progress()` pattern
- rmcp `examples/servers/src/common/progress_demo.rs` — Rust example
- `rust-mcp-sdk` HyperServer — Axum-based SSE streaming
