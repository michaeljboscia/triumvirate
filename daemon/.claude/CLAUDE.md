# Triumvirate Daemon — Claude Implementation Instructions

**Working Directory:** /Users/mikeboscia/projects/triumvirate/daemon
**Language:** Rust (edition 2024, Rust 1.93+)
**Git Branch:** Check before starting. Do NOT work on main.

## Session Startup

1. Read `docs/v2/progress.txt` — where is the project
2. Read `docs/v2/IMPLEMENTATION_PLAN.md` — what phase/step is next
3. Read `docs/v2/LESSONS.md` — mistakes to avoid
4. Check current branch: `git branch --show-current`

## Canonical Docs (Law)

All at `/Users/mikeboscia/projects/triumvirate/docs/v2/`:
- PRD.md — 27 features with FEAT-IDs
- BACKEND_STRUCTURE.md — SQLite schema, REST API, agent protocols
- TECH_STACK.md — exact crate versions
- IMPLEMENTATION_PLAN.md — 8 phases, 60+ steps
- TEST_PLAN.md — 170+ test cases across 7 sections

## Skills to Invoke

Before writing Rust code, invoke the relevant skill:
- `mx-rust-core` — ownership, error handling, module architecture
- `mx-rust-async` — tokio::spawn, channels, JoinSet, CancellationToken
- `mx-rust-network` — axum, WebSocket, SSE
- `mx-rust-data` — serde, rusqlite, config, JSON streaming
- `mx-rust-services` — NATS patterns, Cedar, rust-embed
- `mx-rust-systems` — subprocesses, signals, process groups, graceful shutdown
- `mx-rust-testing` — tokio::test, proptest, insta snapshots
- `mx-rust-project` — Cargo workspaces, clippy, build optimization

## Rules

- NO PTY for agent communication — piped stdio only (GR2-D2)
- NO LLM summarization — mechanical extraction only (REQ-2)
- NO `.unwrap()` in production code — use `?` or explicit handling
- NO `println!` — use `tracing::info!`, `tracing::warn!`, `tracing::error!`
- NO `unsafe` without explicit justification
- Every borrowed pattern from Ruflo/Clash/swarms-rs/Temporal gets inline attribution
- `cargo check` after every file change
- `cargo test` after every feature

## Build

```bash
cargo build          # dev
cargo test           # unit tests
cargo test --features mock  # integration with mock CLIs
cargo run            # boot daemon at :8080
```
