# triumvirate-agentd (v2)

Rust daemon coordinating Claude, Gemini, and Codex in one shared orchestration runtime.

## What It Does

- Runs persistent agent connectors with health supervision and auto-restart.
- Routes human input to agent(s), captures output, tracks quota, and logs decisions.
- Persists workflows and memory to SQLite.
- Provisions fleet worktrees and orchestrates task/merge lifecycle.
- Serves a web API + WebSocket stream on `127.0.0.1:8080` by default.

## Run

```bash
cd /Users/mikeboscia/projects/triumvirate/daemon
cd frontend && npm install && cd ..
cargo run
```

`cargo run` and `cargo build` now invoke a build script in `crates/agentd/build.rs` that runs
`npm run build` in `frontend/` and embeds `frontend/dist` into the Rust binary via `rust-embed`.
Set `TRIUMVIRATE_SKIP_FRONTEND_BUILD=1` to skip this in constrained environments.

## Quality Gate

```bash
cargo check
cargo test
cargo clippy -- -D warnings
cargo build
```

## Fleet APIs

- `POST /api/fleet/spawn`
- `GET /api/fleet/tasks`
- `POST /api/fleet/tasks/claim`
- `POST /api/fleet/tasks/complete`
- `GET /api/fleet/worktrees`
- `POST /api/fleet/worktrees/teardown`
- `POST /api/fleet/merge`
- `POST /api/fleet/peer`
- `GET /api/fleet/status/{fleet_id}`

## Observability

- `GET /metrics` (Prometheus exposition format)
- `GET /api/costs` (per-agent token and estimated cost attribution)

## Governance API

- `POST /api/governance/check`

## Mock CLIs

Workspace includes deterministic mock binaries for test/integration work:

- `mock-claude`
- `mock-gemini`
- `mock-codex`

Connectors can be redirected via env vars:

- `TRIUMVIRATE_CLAUDE_BIN`
- `TRIUMVIRATE_GEMINI_BIN`
- `TRIUMVIRATE_CODEX_BIN`
