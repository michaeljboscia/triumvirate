# Tech Stack — Triumvirate v2.2

---

## Rust Backend (daemon + crates)

| Dependency | Version | Purpose | Phase |
|-----------|---------|---------|-------|
| rusqlite | 0.32+ | SQLite bindings (WAL mode, FTS5) | 1 |
| rusqlite bundled | feature | Bundles SQLite — no system dep required | 1 |
| axum | workspace | HTTP daemon (REST + WebSocket) | 1 |
| axum-extra ws | feature | WebSocket upgrade for dashboard streaming | 5 |
| tokio | workspace | Async runtime (compression workers, spool drain) | 1 |
| serde / serde_json | workspace | JSON serialization for spool events, DTOs | 1 |
| rmcp | workspace | MCP protocol (17 new tools) | 1 |
| reqwest | workspace | HTTP client (hook → daemon wake ping) | 1 |
| clap | workspace | CLI (doctor subcommand extension) | 1 |
| prometheus | workspace | Metrics (queue_lag, marker_parse_rate) | 1 |
| rust-embed | 8.11+ | Static asset embedding (dashboard) | 5 |
| uuid | workspace | Session IDs, fleet IDs, review IDs | 1 |
| libc | workspace | PID checks for fleet crash recovery | 3 |
| sha2 | 0.10+ | sha256(cwd) for scratch project roots | 1 |

### New Crates (4)

| Crate | Type | Key Deps | Phase |
|-------|------|----------|-------|
| `ledger` | Library | rusqlite, serde_json, sha2 | 1 |
| `fleet` | Library | shared-types, daemon-core, agent-worker, ledger | 3 |
| `peer-review` | Library | shared-types, ledger | 4 |
| `dashboard` | Build artifact | (Svelte output, embedded) | 5 |

---

## Frontend (dashboard)

| Dependency | Version | Purpose |
|-----------|---------|---------|
| Svelte | 5.x | UI framework (runes, components as functions) |
| Tailwind CSS | 4.x | Utility-first styling |
| Vite | 6.x | Build tool, dev server, proxy |
| @sveltejs/adapter-static | latest | Static site output for rust-embed |

**Build output:** `dashboard/dist/` → embedded via `#[derive(RustEmbed)] #[folder = "dashboard/dist"]`

**Dev workflow:** `npm run dev` (Vite on :5173) + `cargo run -- daemon` (on :8080). Vite proxy routes `/api/*` and `/ws` to :8080. Production: same origin, no CORS.

---

## Hooks (bash)

| Hook | Event | Purpose | Phase |
|------|-------|---------|-------|
| `post-tool-use-ledger.sh` | PostToolUse | Spool write + wake ping | 1 |
| `session-start-ledger.sh` | SessionStart | Spool write (session started event) | 1 |
| `session-end-ledger.sh` | Stop/SessionEnd | Spool write (session ended event) | 1 |

Hooks depend on: `date`, `$$`, `$RANDOM` (POSIX builtins), `curl` (background wake ping), `git` (project root resolution). No `uuidgen`, no `sqlite3`, no `python3`.

---

## SQLite Schema

Single database per project: `<project>/.triumvirate/ledger.db`

**PRAGMAs:**
```sql
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;
PRAGMA synchronous = NORMAL;
```

**Tables:** events, summaries, sessions, health, lessons, tasks, fleets, reviews

---

## Environment Variables (new in v2.2)

| Variable | Default | Purpose |
|----------|---------|---------|
| `TRIUMVIRATE_PROJECT_ROOT` | (unset) | Override project root resolution |
| `TRIUMVIRATE_LEDGER_LLM_MAX_CALLS_PER_DAY` | 20 | Tier 1 compression budget |
| `TRIUMVIRATE_LEDGER_WORKER_CONCURRENCY` | 1 | Compression workers per project |
| `TRIUMVIRATE_LEDGER_MAX_POOLS` | 10 | Max active worker pools |
| `TRIUMVIRATE_FLEET_ENABLED` | 0 | Enable fleet features (Phase 3) |
| `TRIUMVIRATE_FLEET_SKIP_REVIEW` | 0 | Bypass peer review on fleet merge |
| `TRIUMVIRATE_REQUIRE_PEER_REVIEW` | 0 | Mandatory review for all agent output |
| `TRIUMVIRATE_REVIEW_MAX_INFLIGHT` | 2 | Max concurrent reviews |
| `TRIUMVIRATE_CODEX_AUTO_APPROVE` | 1 | Auto-approve Codex actions |
| `TRIUMVIRATE_CODEX_PROTOCOL` | exec | `exec` or `app-server` |

---

## Build Pipeline

```
1. cd dashboard && npm run build    # Svelte → dist/
2. cd .. && cargo build --release   # Rust + rust-embed
3. ./target/release/triumvirate daemon  # Single binary
```

CI must run step 1 before step 2. `rust-embed` reads `dashboard/dist/` at compile time.
