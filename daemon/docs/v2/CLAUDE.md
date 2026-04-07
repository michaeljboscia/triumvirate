# Triumvirate v2 — AI Agent Instructions

**Project:** triumvirate-agentd
**Language:** Rust (edition 2024)
**Working directory:** /Users/mikeboscia/projects/triumvirate/daemon

---

## Canonical Docs (Law)

These documents are the single source of truth. Do not deviate without explicit approval.

| Doc | Path | Contains |
|-----|------|----------|
| PRD | `docs/v2/PRD.md` | 27 features with FEAT-IDs, acceptance criteria |
| APP_FLOW | `docs/v2/APP_FLOW.md` | User journeys, routes, error states |
| TECH_STACK | `docs/v2/TECH_STACK.md` | Exact crate versions, dependencies |
| DESIGN_SYSTEM | `docs/v2/DESIGN_SYSTEM.md` | Colors, typography, spacing, components |
| FRONTEND_GUIDELINES | `docs/v2/FRONTEND_GUIDELINES.md` | Svelte 5 architecture, naming, stores |
| BACKEND_STRUCTURE | `docs/v2/BACKEND_STRUCTURE.md` | SQLite schema, REST API, WebSocket, agent protocols |
| IMPLEMENTATION_PLAN | `docs/v2/IMPLEMENTATION_PLAN.md` | 8 phases, exact files, FEAT-ID references |
| SPEC | `SPEC.md` | Architecture, REQs, Goat Rodeo decisions |

---

## Project Rules

### Tech Stack
- Rust 1.93.0, edition 2024, Tokio async runtime
- axum 0.8 for HTTP/WebSocket, rusqlite 0.32 for SQLite WAL
- serde_json for all JSON parsing, tracing for observability
- Svelte 5 + Tailwind 4 for dashboard (embedded via rust-embed)
- No external runtime dependencies (no Docker, no NATS, no Temporal server)

### Architecture
- Agent connectors: persistent subprocesses with JSON over piped stdio (NO PTY)
- Message fabric: Tokio broadcast/mpsc/watch channels (NATS-shaped topics for future swap)
- Workflow engine: purpose-built, SQLite-backed state machine with event sourcing
- Memory: SQLite WAL only, no hot cache
- Fleet coordination: git worktrees + contracts-first + shared task list + sequential merge

### Naming Conventions
- Rust: snake_case for functions/variables, PascalCase for types/traits, SCREAMING_SNAKE for constants
- Files: snake_case.rs for Rust, PascalCase.svelte for Svelte, camelCase.ts for TypeScript
- FEAT-IDs: reference in commit messages and code comments where implementing a specific feature
- Crate names: `triumvirate-*` prefix for workspace crates

### What's Forbidden
- No PTY for agent communication (GR2-D2)
- No LLM-generated summaries or session notes (REQ-2)
- No Markdown keyword protocol for memory writes (GR2-D5)
- No hot cache layer in front of SQLite (GR2-D6)
- No external process dependencies (no NATS server, no Temporal server)
- No `unsafe` without explicit justification and comment
- No `.unwrap()` in production code — use `?` or explicit error handling
- No `println!` — use `tracing::info!`, `tracing::warn!`, `tracing::error!`

### What's Required
- Every public function has a doc comment
- Every error type uses `thiserror` derive
- Every async test uses `#[tokio::test]`
- Every new crate gets a `README.md` in its directory
- Every borrowed pattern from Ruflo/Clash/swarms-rs/Temporal gets an inline comment with attribution

---

## Workflow Orchestration

### 1. Plan Mode Default
- Enter plan mode for ANY non-trivial task (3+ steps or architectural decisions)
- If something goes sideways, STOP and re-plan immediately
- Use plan mode for verification steps, not just building

### 2. Subagent Strategy
- Use subagents to keep main context window clean
- One task per subagent for focused execution
- Each agent in a fleet worktree for isolation

### 3. Self-Improvement Loop
- After ANY correction: update LESSONS.md with the pattern
- Write rules that prevent the same mistake
- Review lessons at session start

### 4. Verification Before Done
- Never mark a task complete without proving it works
- `cargo check` after every file change
- `cargo test` after every feature
- Run the daemon and verify in browser for UI changes

### 5. Demand Elegance (Balanced)
- For non-trivial changes: pause and ask "is there a more elegant way?"
- Skip for simple, obvious fixes — don't over-engineer

### 6. Autonomous Bug Fixing
- When given a bug: read the error, check the source, fix it
- Zero context switching from the user

---

## Protection Rules

### No Regressions
- Before modifying any existing file, understand what exists
- Never break working functionality to implement new functionality
- `cargo test` must pass before and after every change

### No File Overwrites
- Never overwrite existing documentation files
- Create new versions when updating

### No Assumptions
- If not covered by docs, STOP and ask
- Do not infer. Do not guess.

### No Contract Changes Without Approval
- If a spec defines an interface and reality doesn't match: BLOCKER
- STOP, tell the user, present 3 options, get decision

### Design System Enforcement
- Before creating ANY Svelte component, check DESIGN_SYSTEM.md
- No invented colors, spacing, or tokens
- Every pixel references the system

### Mobile-First Mandate
- Every component starts as mobile layout
- Desktop is the enhancement

---

## Task Management

1. **Plan First:** Write plan to `docs/v2/tasks/todo.md`
2. **Verify Plan:** Check with user before starting
3. **Track Progress:** Mark items complete as you go
4. **Explain Changes:** High-level summary at each step
5. **Document Results:** Update `docs/v2/progress.txt`
6. **Capture Lessons:** Update `docs/v2/LESSONS.md` after corrections

---

## Session Startup Sequence

1. Read this file (CLAUDE.md)
2. Read `docs/v2/progress.txt` (where is the project)
3. Read `docs/v2/IMPLEMENTATION_PLAN.md` (what phase/step is next)
4. Read `docs/v2/LESSONS.md` (what mistakes to avoid)
5. Write `docs/v2/tasks/todo.md` (plan for this session)
6. Verify plan with user before executing

---

## Rust Skills Available

The following skills are loaded and should be consulted during implementation:

- `mx-rust-core` — ownership, borrowing, error handling, module architecture
- `mx-rust-async` — tokio::spawn, select!, channels, JoinSet, CancellationToken
- `mx-rust-network` — axum, WebSocket, SSE, reqwest
- `mx-rust-data` — serde, rusqlite, config, JSON streaming
- `mx-rust-services` — NATS, Temporal patterns, Cedar, rust-embed
- `mx-rust-systems` — subprocesses, PTY, signals, process groups
- `mx-rust-testing` — tokio::test, proptest, insta snapshots
- `mx-rust-project` — Cargo workspaces, clippy, build optimization
