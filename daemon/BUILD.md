# Build Guide — Triumvirate v2

## Prerequisites

- Rust 1.93+ (`rustup update stable`)
- Node.js 20+ (for Svelte dashboard build, Phase 5+)
- git (for worktree tests)
- Agent CLIs (optional — daemon degrades gracefully):
  - `claude` — `npm install -g @anthropic-ai/claude-code`
  - `gemini` — `brew install gemini`
  - `codex` — `npm install -g @openai/codex`

## Quick Start

```bash
cd daemon
cargo build          # Dev build (~8s, cached)
cargo run            # Boot daemon, dashboard at http://127.0.0.1:8080
```

## Commands

| Command | What |
|---------|------|
| `cargo build` | Dev build |
| `cargo build --release` | Release build (LTO, stripped) |
| `cargo run` | Run daemon |
| `cargo test` | Unit tests |
| `cargo test --features mock` | Integration tests with mock CLIs |
| `cargo clippy -- -D warnings` | Lint (zero warnings required) |
| `cargo fmt --check` | Format check |
| `cargo bench` | Performance benchmarks |
| `RUST_LOG=debug cargo run` | Verbose logging |

## Project Layout

```
daemon/
├── Cargo.toml              # Workspace root
├── crates/
│   ├── agentd/             # Main binary
│   │   └── src/
│   │       ├── main.rs     # Startup sequence
│   │       ├── config.rs   # TOML config
│   │       ├── agent/      # Connectors, pool, health, supervisor
│   │       ├── fabric/     # Message bus (Tokio channels)
│   │       ├── memory/     # SQLite store, extraction
│   │       ├── web/        # axum server, WebSocket
│   │       ├── steno/      # Stenographer
│   │       ├── fleet/      # Worktrees, task list, merge
│   │       ├── routing.rs  # Message routing
│   │       ├── digest.rs   # Mechanical digests
│   │       └── quota.rs    # Token tracking
│   ├── proto/              # Shared types, event parsers
│   │   └── src/
│   │       ├── events.rs   # FabricMessage, Topic, Payload
│   │       ├── claude_events.rs
│   │       ├── gemini_events.rs
│   │       └── codex_events.rs
│   └── workflow/           # Workflow engine
│       └── src/
│           ├── engine.rs   # State machine
│           ├── persistence.rs
│           ├── conversation.rs
│           ├── debate.rs
│           ├── fleet.rs
│           └── retry.rs
├── frontend/               # Svelte dashboard (Phase 5)
├── static/                 # POC HTML (replaced by frontend/)
├── config/                 # Default config templates
└── policies/               # Cedar policy files
```

## Config

Default location: `~/.triumvirate/config.toml`

Copy from `daemon/config/default.toml` and customize.

## Data

| File | Purpose |
|------|---------|
| `~/.triumvirate/memory.db` | Memories, sessions, decisions |
| `~/.triumvirate/workflow.db` | Workflow state, event log |
| `~/.triumvirate/sessions/` | Stenographer JSON logs |
| `~/.triumvirate/policies/` | Cedar authorization policies |
