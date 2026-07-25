# Triumvirate Daemon — Claude Implementation Instructions

**Working Directory:** /Users/you/projects/triumvirate/daemon
**Language:** Rust (edition 2024, Rust 1.93+)
**Git Branch:** Check before starting. Do NOT work on main.

## Session Startup

1. Read `/Users/you/projects/triumvirate/docs/v2/progress.txt` — where is the project
2. Read `/Users/you/projects/triumvirate/docs/v2/IMPLEMENTATION_PLAN.md` — what phase/step is next
3. Read `/Users/you/projects/triumvirate/docs/v2/LESSONS.md` — mistakes to avoid
4. Check current branch: `git branch --show-current`

## Canonical Docs (Law)

| Doc | Full Path |
|-----|-----------|
| SPEC | `/Users/you/projects/triumvirate/SPEC.md` |
| PRD | `/Users/you/projects/triumvirate/docs/v2/PRD.md` |
| BACKEND_STRUCTURE | `/Users/you/projects/triumvirate/docs/v2/BACKEND_STRUCTURE.md` |
| TECH_STACK | `/Users/you/projects/triumvirate/docs/v2/TECH_STACK.md` |
| IMPLEMENTATION_PLAN | `/Users/you/projects/triumvirate/docs/v2/IMPLEMENTATION_PLAN.md` |
| TEST_PLAN | `/Users/you/projects/triumvirate/docs/v2/TEST_PLAN.md` |
| DESIGN_SYSTEM | `/Users/you/projects/triumvirate/docs/v2/DESIGN_SYSTEM.md` |
| FRONTEND_GUIDELINES | `/Users/you/projects/triumvirate/docs/v2/FRONTEND_GUIDELINES.md` |
| APP_FLOW | `/Users/you/projects/triumvirate/docs/v2/APP_FLOW.md` |
| BUILD | `/Users/you/projects/triumvirate/daemon/BUILD.md` |
| progress | `/Users/you/projects/triumvirate/docs/v2/progress.txt` |
| LESSONS | `/Users/you/projects/triumvirate/docs/v2/LESSONS.md` |

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

### Commenting Rules
- Every public struct, trait, function, and enum gets a `///` doc comment explaining WHAT it does
- Inline `//` comments for any non-obvious logic — if you had to think about it, comment it
- Every borrowed pattern gets attribution: `// Adapted from Ruflo's cost-router (ruvnet/ruflo, MIT)`
- FEAT-ID in a comment at the top of each module: `// FEAT-002: Claude Connector`
- No comment spam — don't comment obvious code

### What's Forbidden
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
