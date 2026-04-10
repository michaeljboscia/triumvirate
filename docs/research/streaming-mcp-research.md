# Streaming MCP Research — Path 1 (Progress Notifications) + Path 2 (Streamable HTTP)

## Key Findings from Initial Search (Batch 1)

### rmcp crate supports streamable HTTP natively
- Feature flags: `transport-streamable-http-server`, `transport-streamable-http-client`
- Also has: `transport-sse-server`, `transport-sse-client`
- Uses `sse-stream` crate as dependency
- Unified endpoint: single HTTP path supports POST (commands) + GET (SSE stream)
- Session-based: server returns `Mcp-Session-Id` header

### MCP Streamable HTTP Protocol (2025 revision)
- Client sends JSON-RPC via HTTP POST
- Server can respond with either:
  - Direct JSON (`application/json`) for simple ops
  - SSE stream (`text/event-stream`) for long-running ops with progress
- Client opens GET connection for server-initiated updates
- Session ID management via `Mcp-Session-Id` header

### Claude Code client support
- Claude Code supports HTTP MCP servers (not just stdio)
- `streamable-http` transport is the modern standard (replaces deprecated HTTP+SSE from 2024-11-05)
- Client config: `"transport": "streamable-http"` in server definition

### Axum compatibility (we already use Axum)
- Axum natively supports SSE via `Body::from_stream()`
- `axum-streams` crate adds JSON lines, CSV, plain text streaming
- We already have Axum in our daemon — no framework switch needed

## Remaining searches queued...
