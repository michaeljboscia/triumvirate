# Changelog — Triumvirate Daemon

All notable changes to the Triumvirate daemon are documented here. This
file follows [Keep a Changelog](https://keepachangelog.com/) and the
project uses [Semantic Versioning](https://semver.org/).

## [3.9.0] — 2026-04-11 — Pantheon Backend

First of the two-release split that delivers Pantheon v4.0. The 3.9.0
release is backend-only: it adds the HTTP + WebSocket surfaces Pantheon's
Tauri app (shipping as v4.0.0) will consume, plus the event-lineage
infrastructure Pantheon needs to render hierarchical worker views and
recover state on reconnect. No user-facing CLI changes; `triumvirate watch`
and all existing MCP tools continue to work unchanged.

### Added

- **Pantheon session lineage propagation via both MCP transports**
  (FEAT-011, REQ-010, REQ-033). The daemon now captures a Pantheon
  session ID on every MCP dispatch and threads it through to the
  dispatched worker as `parent_session_id` / `root_session_id`:
  - HTTP Streamable transport reads `X-Pantheon-Session-Id` and optional
    `X-Pantheon-Root-Session-Id` headers via a bearer-auth middleware
    that scopes a tokio task-local (`daemon_core::PANTHEON_SESSION`).
  - stdio transport reads `_meta.pantheon.session_id` /
    `_meta.pantheon.root_session_id` off `CallToolRequestParams` via a
    hand-rolled `ServerHandler` replacing the `#[tool_handler]` macro.
  - Both paths plumb the lineage into `TaskTracker::register` so every
    `WorkerLifecycle` event (see below) carries the correct parent/root.
- **`AgentStreamEvent::WorkerLifecycle` event variant** (FEAT-014,
  REQ-010) emitted on every ABE worker register / complete / fail with
  agent, session_name, task_id, parent_session_id, root_session_id,
  commit_sha, error_message, elapsed_ms, and seq. Pantheon's sidebar
  groups workers into a hierarchy by following parent_session_id.
- **Event replay ring buffer** (FEAT-013, REQ-020).
  `daemon_core::EventReplayBuffer` is an `Arc<RwLock<VecDeque<AgentStreamEvent>>>`
  with 1000-event default capacity. A long-lived fill task spawned by
  `run_daemon` subscribes to the broadcast channel BEFORE the HTTP server
  listens (subscribe-before-read race fix) and pushes every
  `agent_stream` envelope into the buffer. Clients reconnecting with a
  `last_seq` get their missed events replayed.
- **REST API for Pantheon** (FEAT-012, REQ-017):
  - `GET /api/workers` — returns `WorkersResponse` with every ABE worker
    from `TaskTracker::snapshot_workers()` and full lineage. Named MCP
    sessions remain on the existing `/session/list` route (they carry no
    `started_at`/`elapsed_ms`, so fabricating them would be dishonest).
  - `GET /api/fleet` — returns `FleetResponse` aggregating
    `state.fleet_v2_states` (new v3.9.0-shape `FleetBuild` store, keyed
    by `build_id`).
  - `GET /api/fleet/{build_id}` — single-build lookup, 404 on miss,
    axum 0.8 `{build_id}` path syntax (the older `:build_id` form panics
    at router construction in axum 0.8).
- **State snapshot endpoint** (FEAT-013, REQ-020):
  - `GET /api/state` — `StateResponse` with version
    (`daemon_core::VERSION.to_string()`), uptime_ms, workers, fleet,
    last_event_seq. Frozen contract — NO `sessions` field, by design.
- **Replay-aware WebSocket** (FEAT-013, REQ-020):
  - `GET /ws/v2` — new WebSocket route alongside the legacy `/ws`. Bearer
    auth is checked BEFORE `ws.on_upgrade(...)` so unauthenticated
    clients get an HTTP 401, never a connected-then-closed socket.
    Inside the upgraded socket, follows the canonical subscribe-before-read
    pattern: subscribe FIRST, parse `ReplayRequest {"action":"subscribe","last_seq":N}`
    SECOND, read `replay_buffer.replay_since(N)` THIRD, branch on
    `ReplayResult`:
    - `OutOfRange { oldest_seq }` → send a bare
      `ReplayResponse{replay:"out_of_range",oldest_seq}` and close;
      client fetches `/api/state` and reconnects.
    - `Events(v)` → send bare `ReplayResponse{replay:"ok"}` ack, forward
      every historical event wrapped in the SAME
      `daemon_core::encode_ws_event("agent_stream", payload)` envelope
      the live tail uses, track `max_sent` for dedup, then tail
      `ws_events` forever.
  - `RecvError::Lagged` closes the socket. Client reconnects with its
    current `last_seq` and the handshake starts over — canonical
    close-and-reconnect, no in-place recovery.
- **Single-instance PID file** (FEAT-015, REQ-019). `daemon_core::PidFile`
  uses `libc::flock(LOCK_EX | LOCK_NB)` on `~/.triumvirate/daemon.pid`
  at `run_daemon` start. A second daemon on the same machine fails loudly
  with a lock-held error instead of port-bind-failing later.
- **`TaskTracker::snapshot_workers()`** enumerator returning
  `Vec<shared_types::WorkerInfo>` with every tracked record's lineage,
  status, elapsed, and RFC 3339 started_at. Added by T-007.5 to unblock
  `GET /api/workers`.
- **`DaemonState` gains four new fields** (T-007.5): `replay_buffer`,
  `fleet_v2_states`, `last_event_seq`, `started_at`. All `pub` so tests
  can mutate directly.
- **`SessionState` gains three lineage fields** (T-003, REQ-010):
  `parent_session_id`, `root_session_id`, `pantheon_session_id`. All
  `Option<String>` with `#[serde(default)]` for backwards compat with
  sessions persisted before v3.9.0.
- **New `shared_types::api` module** exposing `WorkersResponse`,
  `WorkerInfo`, `FleetResponse`, `FleetBuild`, `FleetTask`,
  `StateResponse`, `ReplayRequest`, `ReplayResponse`. Frozen contract —
  these shapes are consumed by Pantheon's Tauri client.
- **Integration smoke test suite** at
  `daemon/crates/triumvirate/tests/integration_pantheon.rs` with 10
  `#[ignore]`-gated tests covering every Pantheon surface end-to-end
  against a running daemon binary.

### Changed

- `AbeTaskTracker::register` signature extended with `parent_session_id`
  and `root_session_id` parameters so the lineage captured on the inbound
  MCP request survives the `tokio::spawn` to the monitor task.
  `TaskRecord` stores both, and `mark_completed` / `mark_failed` re-read
  them when emitting the terminal `WorkerLifecycle` event.
- `TaskTracker::with_pantheon_observability` constructor adds an
  `EventSequencer` alongside the existing metrics + ws_events for
  monotonic event seqs aligned with the rest of the stream.
- `McpBridge` no longer uses `#[tool_handler]` — the impl is hand-rolled
  so `call_tool` can wrap the inner `tool_router.call` in
  `PANTHEON_SESSION.scope(...)` after extracting lineage from `_meta`.

### Fixed

- Daemon task-completion reaper hang documented 2026-04-09. The daemon
  now runs **four parallel completion detectors** for every ABE worker
  (`child.wait()`, sentinel-file watcher, HEAD-SHA poll, and HTTP
  `/abe/task-complete`). Whichever arrives first transitions the task;
  the sentinel watcher kills any lingering zombie codex-exec process.
  Verified working via two reaper probes in the v3.9.0 closure session.

### Developer notes

- Build manifests for every Wave 2 task live at
  `docs/4.0.0/manifests/BUILD_MANIFEST_T-*.md`. The master append-only
  manifest at `docs/4.0.0/BUILD_MANIFEST.md` records every task commit,
  bake-off outcome, and audit round.
- Wave 2 was executed via a **bake-off pattern**: two parallel Claude
  subagents per task, Gemini code-review judge selecting the winner,
  standout test helpers cherry-picked from the loser. T-008 was a narrow
  win for Apollo (type-alias hygiene); T-009 was a clear win for Apollo
  (Athena duplicated ~160 lines of handler logic into her test module,
  breaking test fidelity).
- A structural Agent-tool worktree-isolation gap was discovered mid-wave:
  `isolation: "worktree"` provides cwd isolation but does NOT sandbox
  absolute file paths, so subagents that copy absolute paths from their
  briefing can escape the worktree and edit main. Recovery patched via
  `git restore --source=<pre-bakeoff-sha> main.rs`. Future waves should
  use relative paths in subagent manifests.

### Test totals (v3.9.0 closure)

- `daemon-core`: 34/34 PASS
- `shared_types`: 28/28 PASS (includes the 5 new `api` roundtrip tests)
- `mcp-tools`: 20/20 PASS (regression)
- `triumvirate::http_mcp::tests`: 6/6 PASS
- `triumvirate::pantheon_stdio_meta_tests`: 10/10 PASS
- `triumvirate::pantheon_session::tests`: 5/5 PASS
- `triumvirate::abe::task_tracker::tests`: 16/16 PASS
- `triumvirate::pantheon_rest_tests`: 9/9 PASS (T-008 new)
- `triumvirate::pantheon_ws_replay_tests`: 9/9 PASS (T-009 new)
- `triumvirate::integration_pantheon`: 10 tests compiled (ignored by
  default; run against a live daemon with `cargo test --test
  integration_pantheon -- --ignored`)

`cargo check --workspace --tests` is clean. The only warnings are
pre-existing dead-code warnings on legacy CLI scaffolding unrelated to
Pantheon.

[3.9.0]: https://github.com/michaeljboscia/triumvirate/releases/tag/v3.9.0
