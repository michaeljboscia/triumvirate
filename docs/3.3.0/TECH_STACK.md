# TECH_STACK — v3.3.0 Live Agent Streaming

**Version:** 3.3.0

## Runtime

| Component | Version | Purpose |
|-----------|---------|---------|
| Rust | edition 2024 (1.82+) | Language |
| tokio | 1.x (rt-multi-thread, io-std, io-util, process, time, sync) | Async runtime |

## Core Dependencies (workspace-level)

| Crate | Version | Purpose | New in 3.3.0? |
|-------|---------|---------|---------------|
| rmcp | 1.3.0 | MCP protocol server + client | **NEW FEATURE: transport-streamable-http-server** |
| axum | 0.8 (ws feature) | HTTP server, WebSocket, SSE | Existing |
| serde | 1 (derive) | Serialization | Existing |
| serde_json | 1 | JSON parsing | Existing |
| tokio | 1 (full feature set) | Async runtime | Existing |
| clap | 4.5 (derive) | CLI argument parsing | Existing (new subcommands: proxy, watch) |
| reqwest | 0.12 (json, rustls-tls) | HTTP client (proxy → daemon) | **NEW USAGE: proxy HTTP client** |
| tokio-tungstenite | latest | WebSocket client (watch CLI) | **NEW** |
| crossterm | latest | Terminal control (watch CLI in-place updates) | **NEW** |
| schemars | 1 | JSON Schema generation | Existing |
| anyhow | 1 | Error handling | Existing |

## rmcp Feature Flags (Cargo.toml change)

```toml
# Current:
rmcp = { version = "1.3.0", features = ["server", "macros", "transport-io", "transport-async-rw", "client"] }

# v3.3.0:
rmcp = { version = "1.3.0", features = ["server", "macros", "transport-io", "transport-async-rw", "client", "transport-streamable-http-server"] }
```

## New Crate Dependencies (per workspace member)

| Crate | Added To | Purpose |
|-------|----------|---------|
| tokio-tungstenite | triumvirate (binary) | WebSocket client for watch CLI |
| crossterm | triumvirate (binary) | Terminal control for in-place heartbeat updates |

## Architecture

```
triumvirate (binary crate)
├── CLI: mcp, daemon, proxy, watch, install, uninstall, status, doctor
├── McpBridge (Arc-shared across transports)
├── Stdio transport (rmcp transport-io) — for `mcp` command
├── HTTP transport (rmcp transport-streamable-http-server) — for `daemon` /mcp endpoint
├── Proxy (reqwest HTTP client ↔ stdio bridge) — for `proxy` command
└── Watch (tokio-tungstenite WS client) — for `watch` command

shared-types crate
└── AgentStreamEvent enum (NEW)

agent-adapter crate
├── GeminiStreamParser (MODIFIED: mpsc channel output)
└── CodexExecParser (MODIFIED: mpsc channel output)

daemon-core crate
└── ObservabilityBus (MODIFIED: emits AgentStreamEvent to WS)

daemon-http crate
└── Existing Axum routes (UNCHANGED)
```

## Infrastructure

| Component | Detail |
|-----------|--------|
| Daemon process | Long-running, launchd-managed, port 8080 |
| Proxy process | Short-lived, spawned by Claude Code as MCP subprocess |
| Watch process | Long-running, user-launched in side terminal/pane |
| Auth | Bearer token from ~/.triumvirate/daemon.token |
| Storage | SQLite WAL (token economics, ledger) — unchanged |
| Metrics | Prometheus at /metrics — unchanged |

## Cost

No new external services. No cloud dependencies. No API costs beyond agent CLI usage (already existing). Zero incremental cost for v3.3.0 infrastructure.
