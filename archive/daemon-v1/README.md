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

## CLI Usage (Operator Flow)

Use the helper scripts in `daemon/scripts/` for day-to-day usage:

```bash
cd /Users/mikeboscia/projects/triumvirate
./daemon/scripts/triumvirate-cli.sh health
./daemon/scripts/triumvirate-cli.sh ask "what changed in auth?"
./daemon/scripts/ask-the-twins "review this migration plan"
./daemon/scripts/triumvirate-cli.sh debate "Redis vs Postgres for caching"
./daemon/scripts/triumvirate-cli.sh fleet "1 codex: build e2e harness"
```

Notes:
- `ask-the-twins` sends both `@claude` and `@gemini` via the v2 `/api/message` endpoint.
- `TRIUMVIRATE_URL` can override the default `http://127.0.0.1:8080`.
- If Claude responds with `Not logged in · Please run /login`, authenticate with the Claude CLI first.

## Keep Daemon Running (macOS launchd)

For reliable background operation, install the launchd service:

```bash
cd /Users/mikeboscia/projects/triumvirate/daemon
./scripts/triumvirate-service.sh install
./scripts/triumvirate-service.sh status
./scripts/triumvirate-service.sh logs
```

Service management commands:
- `install`
- `start`
- `stop`
- `restart`
- `status`
- `logs`
- `uninstall`

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
- `GET /api/lessons` (filtered machine-readable lessons ledger)
- `POST /api/lessons` (manual lesson capture)

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
