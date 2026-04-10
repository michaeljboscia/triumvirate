# Streaming MCP Research — Triumvirate Live Agent Visibility

**Goal:** Deliver the README experience — real-time streaming of agent tool calls, file reads, and responses visible in the Claude Code terminal during `ask_session` execution.

---

## VERDICT: Path 2 (Streamable HTTP) is the move. Path 1 works too — and we can do both.

---

## Path 1: MCP Progress Notifications (works over stdio TODAY)

### How it works
- Client sends `progressToken` in `_meta` field of tool call request
- Server sends `notifications/progress` JSON-RPC messages during execution
- Fields: `progressToken`, `progress` (float), `total` (optional float), `message` (string)
- Works over ANY transport — stdio included

### rmcp API (exact code)
```rust
// Inside a tool handler:
context.peer.notify_progress(ProgressNotificationParam {
    progress_token: ProgressToken(NumberOrString::Number(step as i64)),
    progress: percentage,
    total: Some(100.0),
    message: Some(format!("Gemini: calling read_file (src/auth.rs)")),
    extensions: Default::default(),
}).await?;
```

### Does Claude Code display these?
**MIXED.** Research says:
- The MCP spec fully supports `notifications/progress`
- Claude Code is "designed to display these as progress bar or status indicator"
- BUT: "Claude Code might not always have a generic UI for displaying real-time progress from all custom MCP servers, sometimes buffering output until the process completes"
- There are "ongoing efforts to improve tool rendering and progress indicators within VS Code Chat UI"
- `CLAUDE_CODE_NO_FLICKER=1` env var exists for terminal rendering control

**BOTTOM LINE:** Progress notifications MIGHT display in current Claude Code. Need to test. Even if they don't display yet, they will soon — this is the direction Anthropic is going.

### What we'd change in the daemon
1. During `ask_session` tool execution, forward `GeminiStreamParser` / `CodexExecParser` events as `notify_progress` calls
2. Each parsed streaming event (tool call, file read, response chunk) becomes a progress notification
3. The tool still returns the final response blob — progress is supplementary

### Risk: LOW
- Zero transport change needed (stays stdio)
- Additive only — existing behavior unchanged
- If Claude Code doesn't render them yet, nothing breaks

---

## Path 2: Streamable HTTP Transport (the real solution)

### How it works
- Daemon runs as HTTP server (already does — Axum on port 8080)
- Single MCP endpoint supports POST (commands) + GET (SSE stream)
- Tool responses can be SSE streams instead of single JSON blobs
- Session management via `Mcp-Session-Id` header
- Client opens persistent SSE connection for server-initiated updates

### rmcp support
- Feature flags: `transport-streamable-http-server`, `transport-streamable-http-client`
- Also: `transport-sse-server`, `transport-sse-client`
- `HyperServer` component: Axum-based, handles POST+GET+SSE natively
- `HyperServerOptions { sse_support: true, addr: "127.0.0.1:8080" }`
- Example at `rmcp/examples/servers/src/common/progress_demo.rs`

### Claude Code client config
```bash
# Register as HTTP transport instead of stdio:
claude mcp add --transport http triumvirate http://127.0.0.1:8080/mcp

# Or via JSON:
claude mcp add-json triumvirate '{"type":"http","url":"http://127.0.0.1:8080/mcp"}'
```

### Dual transport (migration path)
- Can run BOTH stdio and HTTP simultaneously
- `mcp-stdio-to-streamable-http-adapter` bridges old clients
- Or just run two server instances sharing the same logic
- Our daemon already has both: stdio MCP (rmcp) + HTTP API (Axum)

### What we'd change
1. Add `transport-streamable-http-server` feature to rmcp dependency
2. Add `/mcp` endpoint to existing Axum router using rmcp's `HyperServer`
3. During tool execution, emit SSE events for each streaming agent event
4. Keep stdio transport as fallback
5. Update `~/.claude.json` to use `{"type":"http","url":"http://127.0.0.1:8080/mcp"}`

### Risk: MEDIUM
- Transport layer change — but additive (old stdio still works)
- We already run Axum, so the HTTP server exists
- Main risk: Claude Code HTTP MCP client behavior differs from stdio

---

## Prior Art: Who Has This Working

### danny-avila/Example-MCP-Server (GitHub)
- **What:** Clean MCP server implementation with Streamable HTTP + progress notifications
- **How:** FastMCP (Python), `long_running_test` and `slow_test` tools with configurable duration
- **Progress:** `ctx.report_progress(current, total)` sends updates during execution
- **Streaming:** SSE-based, session management, verbose logging
- **Language:** Python (FastMCP), but the MCP protocol is the same

### FastMCP (Python SDK)
- `ctx.report_progress(current, total)` for structured progress
- `ctx.log()` for log events during tool execution
- `StreamableHTTPConnectionParams` on client side
- Eliminates polling — real SSE streaming

### rust-mcp-sdk / HyperServer
- Rust equivalent of FastMCP
- `HyperServer` = Axum-based, SSE + Streamable HTTP baked in
- `HyperServerOptions { sse_support: true }`
- Multi-client concurrency, internal session management

### mcp-stdio-to-streamable-http-adapter
- Bridge project: wraps stdio MCP as streamable HTTP
- Enables gradual migration — old clients see stdio, new clients see HTTP
- Acts as proxy, converting messages both directions

---

## Recommended Implementation Plan

### Phase 1: Progress Notifications (Path 1) — 1 day
- Add `notify_progress` calls during `ask_session` / `ask_agent` tool execution
- Forward GeminiStreamParser events as progress messages
- Forward CodexExecParser events as progress messages
- Test whether Claude Code CLI renders them
- **Zero risk. Additive only. Ship it.**

### Phase 2: Streamable HTTP (Path 2) — 2-3 days
- Add `transport-streamable-http-server` feature to rmcp
- Create `/mcp` SSE endpoint on existing Axum server
- Wire tool execution to emit SSE events
- Test with `claude mcp add --transport http`
- Keep stdio as fallback
- **Medium risk. Big payoff. The README becomes real.**

### Phase 3: Rich Streaming Events — 1 day
- Define event schema for agent streaming:
  - `agent_turn_started { agent, session_name }`
  - `agent_tool_call { agent, tool_name, args_summary }`
  - `agent_file_read { agent, file_path }`
  - `agent_response_chunk { agent, text_preview }`
  - `agent_turn_completed { agent, tokens_in, tokens_out, cached, duration_ms }`
- Emit these as both SSE events AND progress notifications
- The daemon already captures all this data — just needs to forward it

---

## Key Sources
- rmcp crate: docs.rs/rmcp, crates.io/crates/rmcp
- MCP spec (progress): modelcontextprotocol.io (2025 revision)
- Claude Code MCP config: claude.com docs, mintlify.com
- Example server: github.com/danny-avila/Example-MCP-Server
- Shuttle.dev MCP tutorial (Rust + streamable HTTP)
- HyperServer: crates.io/crates/rust-mcp-sdk
