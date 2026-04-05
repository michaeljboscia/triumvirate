# Triumvirate Daemon — Codex Implementation Instructions

**Working Directory:** /Users/mikeboscia/projects/triumvirate/daemon
**Language:** Rust (edition 2024, Rust 1.93+)
**Git Branch:** Check before starting. Do NOT work on main.

## Session Startup

1. Read `docs/v2/progress.txt` — where is the project
2. Read `docs/v2/IMPLEMENTATION_PLAN.md` — what phase/step is next
3. Read `docs/v2/LESSONS.md` — mistakes to avoid
4. Read `docs/v2/TEST_PLAN.md` — what tests to write alongside code

## Canonical Docs

All at `/Users/mikeboscia/projects/triumvirate/docs/v2/`:

| Doc | What You Need It For |
|-----|---------------------|
| PRD.md | Feature specs with FEAT-IDs and acceptance criteria |
| BACKEND_STRUCTURE.md | SQLite schema (copy exactly), REST API shapes, agent JSON protocols |
| TECH_STACK.md | Exact crate versions — do NOT upgrade without approval |
| IMPLEMENTATION_PLAN.md | Phase/step you're working on, exact file paths |
| TEST_PLAN.md | Test cases to implement alongside each feature |
| DESIGN_SYSTEM.md | UI tokens (if working on frontend) |
| FRONTEND_GUIDELINES.md | Svelte component architecture (if working on frontend) |
| BUILD.md | How to compile, run, test |

## Architecture Summary

- **Rust single binary** — no external runtime deps
- **Agent connectors** — persistent subprocesses with JSON over piped stdin/stdout
  - Claude: `--input-format stream-json --output-format stream-json --session-id`
  - Gemini: `--acp` (JSON-RPC over stdio)
  - Codex: `mcp-server` (MCP JSON-RPC over stdio)
- **Message fabric** — Tokio broadcast/mpsc/watch channels
- **Workflow engine** — SQLite-backed state machine with event sourcing
- **Memory** — SQLite WAL, no cache
- **Fleet** — git worktrees, contracts-first, shared task list, sequential merge
- **Dashboard** — axum + rust-embed (Svelte in Phase 5)

## Rules

### Must Follow
- Every function implements a specific FEAT-ID from PRD.md
- Every new module gets tests from TEST_PLAN.md written alongside the code
- `cargo check` after every file — compiler errors don't accumulate
- `cargo test` after every feature — tests pass before moving on
- Reference BACKEND_STRUCTURE.md for exact SQLite table schemas — copy them, don't improvise
- Reference BACKEND_STRUCTURE.md for exact REST API shapes — implement them as specified

### Must NOT Do
- Do NOT use PTY for agent communication — piped stdio only
- Do NOT use `.unwrap()` in production code — use `?` or handle explicitly
- Do NOT use `println!` — use `tracing` crate
- Do NOT add crate dependencies not listed in TECH_STACK.md without asking
- Do NOT modify existing working code to accommodate new code — if there's a conflict, stop and ask
- Do NOT skip writing tests — every feature gets tests from TEST_PLAN.md

### Attribution
When borrowing patterns from these repos, add inline comment:
```rust
// Adapted from Ruflo's cost-router (ruvnet/ruflo, MIT)
// Adapted from Temporal's workflow history (temporalio/temporal, Apache 2.0)
// Adapted from Clash's worktree conflict detection
// Adapted from swarms-rs agent lifecycle
```

## Crate Layout

```
daemon/
├── crates/
│   ├── agentd/     # Main binary — connectors, fabric, web, fleet
│   ├── proto/      # Shared types — events, agent protocol parsers
│   └── workflow/   # Workflow engine — state machine, persistence
```

## Build

```bash
cargo build                    # dev build
cargo test                     # unit tests
cargo test --features mock     # integration tests
cargo clippy -- -D warnings    # lint
cargo run                      # run daemon at :8080
```
