# BUILD MANIFEST — T-008 (Pantheon v3.9.0)

**Task ID:** T-008
**Wave:** 2
**REQ:** REQ-017
**FEAT:** FEAT-012 (Daemon REST API for Pantheon)
**Depends on (committed):** T-002, T-003, T-004, T-007.5
**Base SHA:** `c14e057` (T-007.5 closure — pre-Wave-2 leaf tasks)
**Branch:** `v3.9.0`
**Bake-off:** YES — two parallel implementers (Apollo, Athena), Gemini code-review judges, winner cherry-picked.

---

## Audit log

- **Round 1 (2026-04-11)**: REJECTED by Gemini + Codex. Findings: SessionState field-name hallucination (`agent_target`/`started_at_unix_ms` don't exist), scope drift between XML `<files>` and manifest, `/api/fleet/{build_id}` missing from BACKEND_STRUCTURE, done_when misalignment, missing fleet_v2_states population guidance.
- **Round 2 fixes (this revision)**: Dropped SessionState aggregation entirely — T-008 surfaces ABE workers only via `TaskTracker::snapshot_workers()`. Session listing stays on the existing `/session/list` route. BACKEND_STRUCTURE.md updated to document `/api/fleet/{build_id}` and align `/api/state`. XML `<files>` + done_when trimmed to match manifest.

## Mission (one sentence)

Add three new GET endpoints to the daemon's Axum router — `/api/workers`, `/api/fleet`, `/api/fleet/{build_id}` — that return JSON shapes already defined in `shared_types::api`, gated by the existing `is_bearer_authorized` per-handler bearer-token check, sourced exclusively from `state.abe_tasks.snapshot_workers()` and `state.fleet_v2_states`. **SessionState entries are NOT aggregated into /api/workers** — they remain on the existing `/session/list` route, because SessionState has no `started_at`/`elapsed_ms` fields and fabricating them would be dishonest.

## Files you may create or modify

ONLY these files. Touching anything else is a scope violation.

- `daemon/crates/triumvirate/src/main.rs` — add 3 new handler functions, register them in the `app = Router::new()...` chain inside `run_daemon`, add an in-file `#[cfg(test)] mod pantheon_rest_tests` test module
- (no other files)

## Files you MUST NOT modify

- `daemon/crates/shared-types/src/api.rs` — response shapes are FROZEN. T-002 already finalized them. If a field is missing, you've misread the spec — the field is there.
- `daemon/crates/daemon-core/src/lib.rs` — DaemonState shape is FROZEN by T-007.5.
- `daemon/crates/triumvirate/src/abe/task_tracker.rs` — `snapshot_workers()` is FROZEN by T-007.5.
- Any existing route in `run_daemon` — additive only.
- `http_mcp.rs`, `bearer_auth_middleware`, the `/mcp` nested router — auth for the new routes uses the per-handler `is_bearer_authorized` pattern, NOT middleware.

## Public symbols you may use (verified to exist on `c14e057`)

```rust
// daemon-core
use daemon_core::DaemonState;             // already aliased as DaemonRuntimeState in main.rs
// fields you'll read:
//   state.token: String
//   state.sessions: Arc<tokio::sync::Mutex<HashMap<String, shared_types::SessionState>>>
//   state.abe_tasks: TaskTracker
//   state.fleet_v2_states: Arc<tokio::sync::Mutex<HashMap<String, shared_types::FleetBuild>>>
//   state.replay_buffer (don't touch — that's T-009)
//   state.last_event_seq (don't touch — that's T-009)
//   state.started_at (don't touch — that's T-009)

// shared-types (re-exported at the crate root)
use shared_types::{
    WorkersResponse, WorkerInfo,
    FleetResponse, FleetBuild, FleetTask,
    SessionState,
};

// task_tracker (already in scope as `crate::abe::task_tracker::TaskTracker`)
state.abe_tasks.snapshot_workers().await -> Vec<shared_types::WorkerInfo>

// existing helpers in main.rs:
is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) -> bool
```

## SessionState lineage fields (T-003)

`shared_types::SessionState` has these fields you must surface for sessions whose `is_session()` is true:
- `parent_session_id: Option<String>`
- `root_session_id: Option<String>`
- `pantheon_session_id: Option<String>`

Other SessionState fields the route may need: the session's name (key in the HashMap), `agent_target` (e.g., "gemini"/"codex"), `cwd: Option<String>`, `started_at_unix_ms: i64`, `last_active_unix_ms: i64`. **Read the SessionState struct at `daemon/crates/shared-types/src/lib.rs` to confirm field names BEFORE coding** — do not guess.

## Routes to add (axum 0.8 syntax — `{name}`, NOT `:name`)

| Verb | Path | Handler name (suggested) | Returns |
|---|---|---|---|
| GET | `/api/workers` | `api_workers` | `axum::Json<WorkersResponse>` |
| GET | `/api/fleet` | `api_fleet` | `axum::Json<FleetResponse>` |
| GET | `/api/fleet/{build_id}` | `api_fleet_by_id` | `axum::Json<FleetBuild>` (not `FleetResponse`) on hit, `(StatusCode::NOT_FOUND, ())` on miss |

Register them in the `app = Router::new()...` chain alongside the existing `/api/tokens/*` routes. They use the main `state` (DaemonRuntimeState), not the `http_state` (DaemonHttpState).

## Worker aggregation contract

`/api/workers` returns the **union** of:

1. **Persistent named sessions** — every entry in `state.sessions` map. Build a `WorkerInfo` from each `SessionState` using:
   - `session_id` = the HashMap key
   - `agent` = `session_state.agent_target` (e.g. "gemini", "codex")
   - `name` = the HashMap key (Pantheon's UI uses this)
   - `status` = string derived from session state — use `"idle"` as the default since SessionState doesn't track running/idle natively (or pull from a field if one exists; check the struct)
   - `task_id` = `None` for named sessions
   - `parent_session_id`, `root_session_id`, `pantheon_session_id` = pass through directly
   - `cwd` = `session_state.cwd.clone()`
   - `started_at` = format `started_at_unix_ms` as RFC 3339 (use the `chrono` crate which is already a workspace dep — see `format_rfc3339` in task_tracker.rs for the pattern)
   - `elapsed_ms` = `now_unix_ms - started_at_unix_ms` (clamp to u64)

2. **ABE-dispatched workers** — call `state.abe_tasks.snapshot_workers().await` and append all entries to the same `Vec<WorkerInfo>`.

The output `WorkersResponse.workers` is the concatenation. Order is unspecified.

## Auth pattern (per-handler, not middleware)

Every handler starts with the EXACT same gate (copy from `health` in main.rs around line 1395):

```rust
async fn api_workers(
    State(state): State<DaemonRuntimeState>,
    headers: HeaderMap,
) -> Result<axum::Json<WorkersResponse>, StatusCode> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    // ... aggregation logic ...
    Ok(axum::Json(WorkersResponse { workers }))
}
```

## Reality tests (≥5, all in `mod pantheon_rest_tests` at the bottom of main.rs)

Each test must use a real `axum::Router` built via `Router::new().route(...).with_state(state.clone())` — not a mock. Use `tower::ServiceExt::oneshot` to drive requests; the dev-dependency `tower = { version = "0.5", features = ["util"] }` is already present in `daemon/crates/triumvirate/Cargo.toml` (added during T-004).

You will need a helper to build a test `DaemonRuntimeState` with empty sessions, an empty TaskTracker, and an empty `fleet_v2_states`. The constructor signature is `DaemonState::new(token, queues, bind_addr, sessions, sessions_file, abe_tasks, ledger_project_lru, marker_parse_window, metrics, ws_events)` — see how the production code in `run_daemon` builds it.

Required tests (you may add more):

1. **`api_workers_returns_session_with_lineage`** — populate `state.sessions` with one named SessionState that has `pantheon_session_id = Some("pantheon-XYZ")`. Hit GET `/api/workers` with bearer. Assert response.status == 200 AND parsed `WorkersResponse.workers.len() == 1` AND `workers[0].pantheon_session_id == Some("pantheon-XYZ")`. A stub returning `[]` fails this.

2. **`api_workers_aggregates_sessions_and_abe_workers`** — populate `state.sessions` with 1 entry AND register 2 ABE tasks via `state.abe_tasks.register(...)`. Hit GET `/api/workers`. Assert workers.len() == 3 AND the union is correct (no dedup issues). A stub that only reads sessions OR only reads abe_tasks fails this.

3. **`api_workers_rejects_missing_bearer`** — GET `/api/workers` with no Authorization header → 401.

4. **`api_workers_rejects_wrong_bearer`** — GET `/api/workers` with `Bearer wrong-token` → 401.

5. **`api_fleet_returns_v2_builds_from_state`** — populate `state.fleet_v2_states` with one `FleetBuild { build_id: "build-001", task_count: 2, completed: 1, ... tasks: vec![FleetTask{...}, FleetTask{...}] }`. Hit GET `/api/fleet` with bearer. Assert parsed `FleetResponse.builds.len() == 1` AND `builds[0].build_id == "build-001"` AND `builds[0].tasks.len() == 2`.

6. **`api_fleet_by_id_returns_404_for_missing_build`** — empty `fleet_v2_states`. Hit GET `/api/fleet/nonexistent`. Assert response.status == 404. (For an existing build it returns 200 + the FleetBuild.)

7. **`api_fleet_by_id_returns_existing_build`** — populate `fleet_v2_states` with `build-002`. Hit GET `/api/fleet/build-002`. Assert 200 + `FleetBuild.build_id == "build-002"`.

8. **`api_workers_empty_state_returns_empty_array_not_null`** — empty sessions + empty abe_tasks. Hit GET `/api/workers`. Assert `WorkersResponse.workers` deserializes as an empty Vec, NOT null. (Critical for Pantheon's Tauri client which would crash on null.)

## Verify commands

```bash
cargo check -p triumvirate
cargo test -p triumvirate --bin triumvirate -- pantheon_rest_tests
cargo test -p triumvirate --bin triumvirate -- http_mcp::tests pantheon_stdio_meta_tests abe::task_tracker::tests
```

All three must pass. The second is your new test module. The third is the regression check — Wave 1 tests must still be green.

## Done when

- Three new GET routes registered in `run_daemon`'s `Router::new()` chain.
- Three handlers implemented per the contract above.
- 8+ reality tests in `mod pantheon_rest_tests` PASS.
- `cargo check --workspace --tests` clean.
- No existing route or test broken.
- The commit message starts with `T-008:` and references REQ-017.

## Forbidden actions

- Do NOT add a new crate dependency.
- Do NOT modify shared_types.
- Do NOT modify http_mcp or any /mcp subroute.
- Do NOT introduce any `unwrap()` in production paths — use `?` and proper error handling. Tests may use `unwrap`.
- Do NOT use `:build_id` syntax. Axum 0.8 panics on it.
- Do NOT add middleware — auth is per-handler.
- Do NOT include any `TODO`/`FIXME`/`unimplemented!()`/`todo!()` in your code. The Phase 7 reality gate scans for these and rejects.
- Do NOT silently swallow errors. Log them via `tracing::warn!` or surface as 500.

## How to start

1. Read `/Users/mikeboscia/projects/triumvirate/daemon/crates/triumvirate/src/main.rs` lines 1395-1445 to see the existing `health`/`status` handlers — copy their auth pattern exactly.
2. Read `/Users/mikeboscia/projects/triumvirate/daemon/crates/shared-types/src/api.rs` to see the exact response shapes.
3. Read `/Users/mikeboscia/projects/triumvirate/daemon/crates/triumvirate/src/abe/task_tracker.rs` lines around `snapshot_workers` (find via grep) to see how a Vec<WorkerInfo> gets built.
4. Read `/Users/mikeboscia/projects/triumvirate/daemon/crates/shared-types/src/lib.rs` for the SessionState struct fields.
5. Implement the three handlers.
6. Add the routes to the chain in `run_daemon` next to the existing `/api/tokens/*` routes (use `get(handler)` not `get_service` since these handlers take `State<DaemonRuntimeState>` not `State<DaemonHttpState>`).
7. Write the test module.
8. Run the verify commands.
9. Commit with `T-008: REST endpoints (workers/fleet/fleet-by-id) — REQ-017 — FEAT-012`.

## The bake-off

Two implementers (Apollo and Athena) will independently produce a diff against this manifest. Gemini will review BOTH diffs and pick the winner. The winner's code goes in. The loser's standout bits (better test phrasings, cleaner error handling, smarter helpers) get cherry-picked into the merged commit. **Don't try to outsmart your peer — implement faithfully against the contract.** Style variation is fine; scope variation is not.
