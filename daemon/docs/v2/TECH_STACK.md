# TECH_STACK — Triumvirate v2

**Version:** 1.0
**Date:** 2026-04-05
**Cross-refs:** PRD.md (FEAT-IDs), BACKEND_STRUCTURE.md, IMPLEMENTATION_PLAN.md

---

## Language & Runtime

| Component | Version | Purpose |
|-----------|---------|---------|
| Rust | 1.93.0 (edition 2024) | Primary language. Zero-cost abstractions, no GC. |
| Tokio | 1.51.0 | Async runtime. Full features (rt-multi-thread, macros, sync, process, net, fs, signal, io-util). |

---

## Core Dependencies (version-locked from Cargo.lock)

### HTTP & Web

| Crate | Version | Purpose | FEAT |
|-------|---------|---------|------|
| axum | 0.8.8 | HTTP server + WebSocket (ws feature) | FEAT-014 |
| tower-http | 0.6.8 | CORS, tracing middleware | FEAT-014 |
| rust-embed | 8.11.0 | Static asset embedding (Svelte build) | FEAT-014 |

### Serialization

| Crate | Version | Purpose | FEAT |
|-------|---------|---------|------|
| serde | 1.0.228 | Serialization framework (derive feature) | ALL |
| serde_json | 1.0.149 | JSON parsing for agent protocols | FEAT-002/003/004 |
| toml | 0.8.23 | Config file parsing | FEAT-026 |

### Database

| Crate | Version | Purpose | FEAT |
|-------|---------|---------|------|
| rusqlite | 0.32.1 | SQLite with WAL mode (bundled feature — compiles SQLite from C source) | FEAT-007, FEAT-012 |

### Observability

| Crate | Version | Purpose | FEAT |
|-------|---------|---------|------|
| tracing | 0.1.44 | Structured logging / spans | ALL |
| tracing-subscriber | 0.3.23 | Log formatting (env-filter, json features) | ALL |

### Utilities

| Crate | Version | Purpose | FEAT |
|-------|---------|---------|------|
| uuid | 1.23.0 | Session IDs, message IDs (v4, serde features) | ALL |
| chrono | 0.4.44 | Timestamps (serde feature) | ALL |
| anyhow | 1.0.102 | Error handling (application level) | ALL |
| thiserror | 2.0.18 | Error handling (library level) | ALL |
| async-trait | 0.1.89 | Async trait dispatch for AgentConnector | FEAT-001 |
| tokio-stream | 0.1.18 | Stream utilities for fabric | FEAT-006 |

### Planned (not yet in Cargo.toml)

| Crate | Expected Version | Purpose | FEAT | Phase |
|-------|---------|---------|------|-------|
| cedar-policy | latest | Authorization / governance | FEAT-022 | Week 2 |
| notify | 8.x | File system watching (fsnotify equivalent) | FEAT-011 | Week 1 |
| portable-pty | 0.9.0 | PTY if needed for edge cases | FEAT-001 | POC 2 |
| opentelemetry | 0.28.x | Distributed tracing | FEAT-025 | Week 4 |
| opentelemetry-otlp | 0.28.x | OTLP exporter | FEAT-025 | Week 4 |

---

## Frontend

| Technology | Version | Purpose |
|-----------|---------|---------|
| Svelte | 5.x | Dashboard UI framework |
| Tailwind CSS | 4.x | Utility-first CSS |
| Vite | 6.x | Build tool for Svelte |

Frontend builds to static HTML/CSS/JS, embedded into the Rust binary via `rust-embed`. No Node.js runtime needed at deployment — only at build time.

---

## External CLI Dependencies

These are NOT bundled. User must have them installed.

| CLI | Install | Protocol | FEAT |
|-----|---------|----------|------|
| claude | `npm install -g @anthropic-ai/claude-code` | stream-json over stdio | FEAT-002 |
| gemini | `brew install gemini` or npm | ACP JSON-RPC over stdio | FEAT-003 |
| codex | `npm install -g @openai/codex` | MCP JSON-RPC over stdio | FEAT-004 |

Daemon checks for CLI presence at boot. Missing CLIs are logged as warnings, not errors — you can run with 1 or 2 agents.

---

## Storage

| Store | Technology | Location | Purpose |
|-------|-----------|----------|---------|
| Memory DB | SQLite WAL | `~/.triumvirate/memory.db` | Memories, sessions, decisions |
| Workflow DB | SQLite WAL | `~/.triumvirate/workflow.db` | Workflow state, event log |
| Config | TOML | `~/.triumvirate/config.toml` | User configuration |
| Cedar Policies | Cedar language | `~/.triumvirate/policies/*.cedar` | Authorization rules |
| Session Logs | JSON | `~/.triumvirate/sessions/` | Stenographer output |

All storage is local filesystem. No network dependencies. No cloud services.

---

## Infrastructure

| Component | Value |
|-----------|-------|
| Hosting | Local machine only (macOS primary, Linux secondary) |
| Network | localhost only (127.0.0.1:8080) |
| External services | None |
| Cloud dependencies | None |
| Docker | Not required |
| Cost | $0 infrastructure. CLI subscriptions are the only cost. |

---

## Build & CI

| Tool | Purpose |
|------|---------|
| `cargo build --release` | Production build (LTO thin, strip symbols) |
| `cargo test` | Unit + integration tests |
| `cargo clippy` | Linting |
| `cargo fmt` | Formatting |
| Feature flag `--features mock` | Enable mock CLIs for testing |

### Build Performance

| Optimization | Status |
|-------------|--------|
| sccache | Recommended (compile cache) |
| mold linker | Recommended (faster linking) |
| Cranelift backend | Optional (faster dev builds, requires nightly) |
| LTO thin | Enabled in release profile |
| Strip symbols | Enabled in release profile |
