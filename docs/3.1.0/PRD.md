# Triumvirate 3.1.0 MCP Consolidation — PRD

**Spec:** `specs/MCP_CONSOLIDATION.md`
**Target version:** `3.1.0` (single source: `daemon/Cargo.toml`)

---

## Features

### FEAT-001: Crate Architecture Refactor
Extract the 6,216-line `main.rs` monolith into organized crates with functional boundaries. Production code (~2,824 lines) splits into `mcp-tools` (tool handlers), `daemon-http` (HTTP routes), and `daemon-core` (state management). `main.rs` shrinks to startup wiring (~300 lines). (Line counts verified 2026-04-09 at HEAD 373256451.)

**Acceptance:** `wc -l main.rs` < 300. `cargo test --workspace` passes. No behavioral change.

### FEAT-002: Tool Aliases (Backwards Compatibility)
Register backwards-compatible MCP tool aliases so existing skills reference TS tool names (`spawn_daemon`, `ask_daemon`, etc.) and they route to Rust equivalents. Parameter mapping handles schema differences (`target` → `agent`, `daemon_id` prefix convention).

**Acceptance:** Every TS tool name callable through Rust daemon. Alias usage logged.

### FEAT-003: Skill Migration (send-to-*)
Update `send-to-codex`, `send-to-gemini`, and `send-to-siblings` skills to use `mcp__triumvirate__ask_session` instead of the TS `send_message` + `get_response` pattern.

**Acceptance:** All three skills work through Rust daemon. No fire-and-forget semantics needed.

### FEAT-004: Front Door Swap
Replace TS MCP server as Claude's primary MCP endpoint with the Rust daemon. Update `~/.claude.json`. Archive TS server.

**Acceptance:** `inter-agent` entry removed from `~/.claude.json`. Node.js process gone. All tools work via `mcp__triumvirate__*`.

### FEAT-005: ObservabilityBus Pre-Wire
Create shared `ObservabilityBus` struct (`Arc<DaemonMetrics>` + `broadcast::Sender<String>`) injected into both HTTP routes and MCP tools. Pre-wires v3.2 observability sprint.

**Acceptance:** McpBridge has access to metrics and WS sender. No instrumentation yet (that's v3.2).

---

## Feature-to-REQ Mapping

| FEAT | REQs |
|------|------|
| FEAT-001 | REQ-C1, C2, C3, C4 |
| FEAT-002 | REQ-A1, A2, A3 |
| FEAT-003 | REQ-J2, J3, J4 |
| FEAT-004 | REQ-F1, F2, F3, F4, X1, X2, X3 |
| FEAT-005 | REQ-B1, B2, B3 |
