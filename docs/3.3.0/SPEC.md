# v3.3.0 — Live Agent Streaming

**Version:** 3.3.0
**Working Directory:** /Users/mikeboscia/projects/triumvirate
**Git Branch:** main (will branch to v3.3.0 for build)
**Goatrodeo:** 7 rounds, 8 decisions, 38 auto-resolves, Phase 3 PASS

## Problem

The daemon captures real-time streaming data from agents (tool calls, file reads, response chunks, token counts) but none of it is visible to the user during execution. When the user calls `ask_session` or `ask_agent`, they stare at a spinner for 30 seconds, then get a wall of text. The README promises live streaming visibility. The product doesn't deliver it.

## Goal

When the user asks an agent a question through the daemon, they see each step as it happens in a watch pane:

```
→ Gemini: turn started
→ Gemini: calling read_file (src/middleware/auth.rs)
→ Gemini: calling read_file (src/middleware/jwt.rs)
→ Gemini: generating response (5s elapsed)
→ Gemini: responded (12,847 in / 1,203 out / 8,400 cached, 2 tools, 4.1s)
```

Delivered today via `triumvirate watch` side pane. Delivered inline when Claude Code supports MCP SSE rendering (future).

## Architecture (Post-Goatrodeo)

Three new single-purpose commands:
- `triumvirate mcp` — stdio MCP server (unchanged from v3.2.0)
- `triumvirate proxy` — stdio↔HTTP bridge with auto-reconnect (golden path)
- `triumvirate watch` — connects to /ws, pretty-prints AgentStreamEvent

The daemon (`triumvirate daemon`) gains a Streamable HTTP MCP endpoint at `/mcp` using rmcp's `transport-streamable-http-server` feature. The proxy bridges Claude Code's stdio to this endpoint. Agent execution flows through the daemon, so metrics, token ledger, and WebSocket all see every event.

```
┌──────────────┐     stdio      ┌──────────────┐     HTTP/SSE     ┌──────────────┐
│  Claude Code  │ ──────────── │  proxy        │ ──────────────── │  daemon      │
│  (user)       │              │  (bridge)     │                  │  (:8080)     │
└──────────────┘              └──────────────┘                  │              │
                                                                  │  ┌─────────┐│
┌──────────────┐     WS        │  │ McpBridge ││
│  watch CLI    │ ◄──────────── │  │ (35+tools)││
│  (streaming)  │              │  │           ││
└──────────────┘              │  │ Axum HTTP ││
                                                                  │  │ WS /ws   ││
                                                                  │  │ SSE /mcp ││
                                                                  │  └─────────┘│
                                                                  └──────────────┘
```

## Requirements (Final — Post Round 7)

### Streaming Event Schema

**REQ-E01:** AgentStreamEvent enum defined in shared-types crate with variants: `TurnStarted { agent, session_name }`, `ToolCall { agent, tool_name, args_summary }`, `FileRead { agent, file_path }`, `ResponseChunk { agent, text_preview }`, `TurnCompleted { agent, tokens_in, tokens_out, cached_tokens, tool_count, duration_ms }`, `Error { agent, message }`. Each event carries a monotonic sequence number.

**REQ-E02:** AgentStreamEvent MUST be defined in the `shared-types` crate so all consumers (MCP, HTTP, WS) use the same type.

**REQ-E03:** GeminiStreamParser and CodexExecParser MUST be modified to produce AgentStreamEvent values via a `tokio::sync::mpsc` channel during stream parsing. New `execute_ask_agent_streaming()` function returns the channel. Old `execute_ask_agent()` wraps it and collects into a String blob (adapter pattern). Existing callers unchanged.

**REQ-E04:** The WebSocket broadcast (`/ws`) MUST emit AgentStreamEvent as a new event type (`agent_stream`) alongside existing v3.2.0 events (`token_update`, `abe_task_state`, `fleet_progress`). No existing events removed or modified.

### Streamable HTTP Transport

**REQ-H01:** The daemon MUST expose an MCP-compliant Streamable HTTP endpoint at `/mcp` (POST for JSON-RPC requests, GET for SSE notifications). All 35+ tools available on both stdio and HTTP transports via shared `Arc<McpBridge>`.

**REQ-H02:** The daemon MUST expose a `GET /mcp` endpoint for clients to establish an SSE connection for server-initiated notifications.

**REQ-H03:** Session management via `Mcp-Session-Id` header. Daemon generates ID on `initialize`, returns in response header.

**REQ-H04:** Streamable HTTP MUST coexist with existing stdio transport. Both active simultaneously. Two transport adapters sharing one `Arc<McpBridge>`.

**REQ-H05:** During tool execution over Streamable HTTP, the daemon MUST stream formatted text chunks as partial tool_result content. Format: `"→ {agent}: {action} ({detail})\n"`. The daemon formats, the client displays.

**REQ-H06:** Final tool result sent as last SSE frame, then stream closes.

**REQ-H07:** Use rmcp `transport-streamable-http-server` feature (available in 1.3.0, already in deps).

**REQ-H08:** Streamable HTTP endpoint works with direct HTTP MCP clients: `claude mcp add --transport http triumvirate http://127.0.0.1:8080/mcp`. This is for non-proxy clients; the golden path uses the proxy command.

**REQ-H09:** Bearer token auth on `/mcp` matching existing HTTP API.

**REQ-H10:** Integration tests verify SSE streaming: connect, call tool, receive at least 2 SSE progress frames before final result.

### Proxy Command

**REQ-P01:** `triumvirate proxy` bridges stdio (Claude Code) to HTTP (daemon :8080/mcp). One job: translate stdio JSON-RPC ↔ HTTP JSON-RPC.

**REQ-P02:** Auto-reconnect with bounded exponential backoff when daemon restarts. Fail in-flight calls loudly, recover channel for subsequent calls.

**REQ-P03:** If daemon is unreachable at startup, retry for 5 seconds then exit with clear error: "daemon not reachable at 127.0.0.1:8080 — run 'triumvirate daemon' first". No silent fallback.

**REQ-P04:** Unit tests verify: proxy forwards tool calls, proxy reconnects after disconnect, proxy exits cleanly when daemon is down.

### Watch CLI

**REQ-W01:** `triumvirate watch` connects to daemon WebSocket at /ws and pretty-prints AgentStreamEvent values. Format: `"→ {agent}: {action} ({detail})"`.

**REQ-W02:** Default shows `agent_stream` events only. `--all` flag shows all WS event types.

**REQ-W03:** `--session <name>` filter for specific agent session.

**REQ-W04:** Heartbeat display during long generation: `"→ Gemini: generating response (15s elapsed)"` with running timer updated in-place.

**REQ-W05:** Handles daemon-not-running gracefully: retry with clear message, not crash.

**REQ-W06:** Detects sequence number gaps, prints `"[events skipped, resynced at seq N]"`.

### Spike Test

**REQ-K01:** Build a 50-line test MCP server using rmcp progress_demo.rs that sends 5 SSE notification frames over Streamable HTTP. Register in Claude Code. Call a tool. Observe whether Claude Code renders intermediate frames. Document results. Runs in parallel with main build.

## Non-Goals

- Dashboard UI (tracked in GitHub #12, becomes Pantheon v4.0)
- Streaming for ABE fleet dispatch (headless workers)
- Token-by-token LLM output streaming (agents don't expose this)
- Breaking changes to stdio MCP interface
- Replacing existing WS event schemas

## Dependencies

- `rmcp` 1.3.0 with `transport-streamable-http-server` feature
- Existing `GeminiStreamParser` and `CodexExecParser` in agent-adapter
- Existing WebSocket broadcast on `/ws`
- Existing Axum HTTP server on :8080

## Goatrodeo Decision Log

| Round | Decision | Choice |
|-------|----------|--------|
| R1 | SSE content format | Formatted text chunks (not JSON-RPC progress) |
| R1 | Executor refactor | Adapter pattern (new streaming fn wraps old) |
| R1 | WS backward compat | Alongside existing events (not replace) |
| R2 | Build strategy | Full stack now, side-pane today, inline when CC supports SSE |
| R2 | Spike test | Parallel with build, doesn't block |
| R5 | Migration safety | Smart proxy (later split into three commands) |
| R6 | Smart proxy confirmed | Build despite no CC streaming benefit — unified execution |
| R7 | Proxy architecture | Three commands (mcp, proxy, watch), proxy reconnects, no fallback |
