# v3.1 MCP Consolidation — Tech Stack

**Spec:** `specs/MCP_CONSOLIDATION.md`

---

## Languages

| Language | Where | Version |
|----------|-------|---------|
| Rust | Daemon binary, all crates | 2021 edition (workspace) |
| TypeScript | Archived TS MCP server (deleted from runtime) | N/A post-migration |
| Svelte/TS | Dashboard (unchanged) | Existing |

## Frameworks & Libraries (Rust)

| Crate | Version | Purpose | Change |
|-------|---------|---------|--------|
| `rmcp` | workspace | MCP protocol, tool_router, stdio transport | Unchanged — already the MCP backbone |
| `axum` | workspace | HTTP server, routes, middleware | Unchanged — routes extracted to daemon-http |
| `tokio` | workspace | Async runtime | Unchanged |
| `tracing` | workspace | Structured logging, spans | Unchanged (v3.2 adds instrumentation) |
| `serde` / `serde_json` | workspace | JSON serialization | Unchanged |
| `rusqlite` | 0.32 | SQLite (ledger, peer-review, fleet) | Unchanged |
| `prometheus` | (via DaemonMetrics) | Prometheus metrics | DaemonMetrics moves to shared location |
| `dashmap` | — | NOT USED — async job queue eliminated | — |

## External Dependencies

| Dependency | Purpose | Change |
|-----------|---------|--------|
| Gemini CLI | Agent backend | Unchanged |
| Codex CLI | Agent backend | Unchanged |
| Node.js | TS MCP server runtime | **REMOVED from runtime** |

## Infrastructure

| Component | Location | Change |
|-----------|----------|--------|
| Daemon binary | `daemon/target/release/triumvirate` | Unchanged path |
| MCP config | `~/.claude.json` | `inter-agent` entry removed, `triumvirate` entry updated |
| Dashboard | `http://localhost:8080` (or 18180) | Unchanged |
| Prometheus | `http://localhost:8080/metrics` | Unchanged |

## Cost

$0 infrastructure change. Pure code reorganization.
