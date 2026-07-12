# BUILD MANIFEST — T-009 (Pantheon v3.9.0)

**Task ID:** T-009
**Wave:** 2
**REQ:** REQ-020
**FEAT:** FEAT-013 (Event replay + reconnect resilience)
**Depends on (committed):** T-002, T-006, T-007.5
**Base SHA:** `c14e057` (T-007.5 closure)
**Branch:** `v3.9.0`
**Bake-off:** YES — two parallel implementers (Apollo, Athena), Gemini code-review judges.

---

## Audit log

- **Round 1 (2026-04-11)**: REJECTED by Gemini (CRITICAL) + Codex (HIGH). Both twins flagged a wire-format inconsistency in the reference implementation: historical replay events were to be sent as bare `AgentStreamEvent` JSON while live tail events flow through the `encode_ws_event("agent_stream", ...)` envelope. Client parsers would break on the protocol switch. Codex also flagged that the IMPLEMENTATION_PLAN T-009 reality_test referenced a `sessions (Vec)` field on `StateResponse` that the frozen T-002 api.rs type does not have.
- **Round 2 fixes (this revision)**: Replay frames now use the SAME `daemon_core::encode_ws_event` envelope as the live tail. `StateResponse` no longer mentions a `sessions` field (the frozen type has none). BACKEND_STRUCTURE.md updated to reflect the actual `ReplayResult::OutOfRange { oldest_seq }` shape. `daemon_core::VERSION.to_string()` used explicitly. RecvError::Lagged close-and-reconnect behavior made explicit.

## Mission (one sentence)

Add `GET /api/state` (full daemon state snapshot) and a NEW WebSocket route `/ws/v2` (replay-aware handshake using `state.replay_buffer`) to the daemon's Axum router. **Do not touch the existing `/ws` route — `triumvirate watch` and other legacy clients depend on its current behavior.**

## Wire format (READ BEFORE WRITING A SINGLE LINE OF CODE)

All `/ws/v2` frames sent from server to client fall into one of two categories:

1. **Handshake-response frames** (bare JSON, no envelope): the first frame after the client's subscribe handshake is always a `ReplayResponse`:
   - `{"replay":"ok"}` on success
   - `{"replay":"out_of_range","oldest_seq":<u64>}` when the client's `last_seq` is older than the buffer's oldest event
   Clients distinguish handshake frames from event frames by the presence of the top-level `"replay"` field.

2. **Event frames** (wrapped in envelope, identical shape for historical replay AND live tail): every `AgentStreamEvent` goes through `daemon_core::encode_ws_event("agent_stream", serde_json::to_value(event).unwrap())`. The resulting envelope is `{"type":"agent_stream","ts_ms":<unix_ms>,"payload":<event json>}`. Clients parse the envelope, match `type=="agent_stream"`, and deserialize `payload` back into `AgentStreamEvent`.

**One wire format. One envelope. No exceptions.** The Phase 5.3 Round 1 audit explicitly flagged that the earlier reference implementation sent bare replay events followed by envelope-wrapped live events — this would break clients at the replay→live transition. Do not introduce that bug again.

## Files you may create or modify

ONLY these files:

- `daemon/crates/triumvirate/src/main.rs` — add 2 new handler functions (`api_state`, `ws_v2`), register them in the `app = Router::new()...` chain, add an in-file `#[cfg(test)] mod pantheon_ws_replay_tests` test module
- (no other files)

## Files you MUST NOT modify

- `daemon/crates/shared-types/src/api.rs` — `StateResponse`, `ReplayRequest`, `ReplayResponse` are FROZEN.
- `daemon/crates/daemon-core/src/replay.rs` — `EventReplayBuffer` and `ReplayResult` are FROZEN by T-006.
- `daemon/crates/daemon-core/src/lib.rs` — DaemonState fields are FROZEN by T-007.5.
- `daemon/crates/daemon-http/src/lib.rs` — the legacy `ws_route` stays exactly as it is (other clients use it).
- The existing `/ws` route registration in main.rs.

## Public symbols you may use (verified to exist on `c14e057`)

```rust
use shared_types::{
    StateResponse, ReplayRequest, ReplayResponse,
    WorkerInfo, FleetBuild, SessionState, AgentStreamEvent,
};
use daemon_core::{
    DaemonState, EventReplayBuffer, ReplayResult,
    REPLAY_BUFFER_DEFAULT_CAPACITY, // 1000
};

// fields you'll read (all on state: DaemonRuntimeState):
state.token: String                                              // for auth
state.sessions: Arc<tokio::sync::Mutex<HashMap<String, SessionState>>>
state.abe_tasks: TaskTracker                                     // call .snapshot_workers().await
state.fleet_v2_states: Arc<tokio::sync::Mutex<HashMap<String, FleetBuild>>>
state.replay_buffer: Arc<EventReplayBuffer>                      // read via replay_since
state.last_event_seq: Arc<std::sync::atomic::AtomicU64>          // for last_event_seq field
state.started_at: std::time::Instant                             // for uptime_ms
state.ws_events: tokio::sync::broadcast::Sender<String>          // subscribe for live tail

// existing helper:
is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) -> bool
```

## Routes to add

| Verb | Path | Handler name | Auth | Notes |
|---|---|---|---|---|
| GET | `/api/state` | `api_state` | Bearer header (per-handler) | Returns `axum::Json<StateResponse>` |
| GET | `/ws/v2` | `ws_v2` | Bearer header on the upgrade request (per-handler) | WebSocket upgrade → handshake → replay → live tail |

## `/api/state` contract

Returns `StateResponse` populated as follows. The type is frozen by T-002 and has NO `sessions` field — named MCP sessions stay on the existing `/session/list` route.

| Field | Source |
|---|---|
| `version: String` | `daemon_core::VERSION.to_string()` (VERSION is `&'static str`, so `.to_string()` is required to coerce into `String`) |
| `uptime_ms: u64` | `state.started_at.elapsed().as_millis().min(u64::MAX as u128) as u64` (saturate, don't panic) |
| `workers: Vec<WorkerInfo>` | `state.abe_tasks.snapshot_workers().await` — ABE workers only, same source as T-008's `/api/workers`. Do NOT aggregate SessionState entries. |
| `fleet: Vec<FleetBuild>` | `state.fleet_v2_states.lock().await.values().cloned().collect()` |
| `last_event_seq: u64` | `state.last_event_seq.load(std::sync::atomic::Ordering::Relaxed)` |

## `/ws/v2` handshake protocol (the meat of T-009)

The protocol is the **subscribe-before-read** pattern Gemini's research confirmed is canonical for axum 0.8 + tokio::broadcast + replay buffers. Implementation order matters — get this wrong and clients will silently drop events.

### Step 1: Auth on the upgrade request

Before calling `ws.on_upgrade(...)`, check `is_bearer_authorized` against the headers. Return 401 on miss. Do this in the route handler, not inside the upgraded socket — closing the socket on auth failure is too late.

### Step 2: Subscribe to broadcast BEFORE reading the buffer

This is the race-condition fix. Do it inside `on_upgrade` BEFORE you do anything else:

```rust
ws.on_upgrade(move |mut socket| async move {
    // CRITICAL: subscribe FIRST, before reading the snapshot.
    let mut live_rx = state.ws_events.subscribe();
    // ... handshake ...
})
```

### Step 3: Read the client's first message — the subscribe handshake

```rust
let first = match socket.recv().await {
    Some(Ok(axum::extract::ws::Message::Text(text))) => text,
    _ => { let _ = socket.close().await; return; }
};
let req: shared_types::ReplayRequest = match serde_json::from_str(&first) {
    Ok(r) if r.action == "subscribe" => r,
    _ => { let _ = socket.close().await; return; }
};
```

### Step 4: Read the replay buffer snapshot

```rust
let replay = state.replay_buffer.replay_since(req.last_seq);
```

### Step 5: Branch on `ReplayResult`

```rust
match replay {
    ReplayResult::OutOfRange { oldest_seq } => {
        // Send a single ReplayResponse, then close.
        let resp = shared_types::ReplayResponse {
            replay: "out_of_range".into(),
            oldest_seq: Some(oldest_seq),
        };
        let _ = socket.send(axum::extract::ws::Message::Text(
            serde_json::to_string(&resp).unwrap().into()
        )).await;
        let _ = socket.close().await;
        return;
    }
    ReplayResult::Events(events) => {
        // Send "ok" ack (bare ReplayResponse, no envelope — handshake frame).
        let ack = shared_types::ReplayResponse { replay: "ok".into(), oldest_seq: None };
        if socket.send(axum::extract::ws::Message::Text(
            serde_json::to_string(&ack).unwrap().into()
        )).await.is_err() { return; }

        // Track max seq sent so we can dedupe overlap with the live stream.
        let mut max_sent = req.last_seq;

        // CRITICAL: wrap EVERY historical event in the SAME envelope shape
        // the live tail uses. The client parses envelopes uniformly; sending
        // bare AgentStreamEvent JSON here would break the replay→live
        // transition and was the Round 1 audit's critical finding.
        for event in &events {
            let payload_value = match serde_json::to_value(event) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let envelope = daemon_core::encode_ws_event("agent_stream", payload_value);
            if socket.send(axum::extract::ws::Message::Text(envelope.into())).await.is_err() {
                return;
            }
            max_sent = max_sent.max(event.seq());
        }

        // Switch to live tail. Live envelopes are already encoded by the
        // publisher (TaskTracker etc.) so we forward them unchanged after
        // dedup. Parse to extract seq for the dedup check, but ALWAYS send
        // the raw envelope string — do NOT re-serialize.
        loop {
            match live_rx.recv().await {
                Ok(envelope) => {
                    let mut skip = false;
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&envelope) {
                        if value.get("type").and_then(|v| v.as_str()) == Some("agent_stream") {
                            if let Some(payload) = value.get("payload") {
                                if let Ok(event) = serde_json::from_value::<shared_types::AgentStreamEvent>(payload.clone()) {
                                    let seq = event.seq();
                                    if seq <= max_sent {
                                        skip = true; // dedup the overlap
                                    } else {
                                        max_sent = seq;
                                    }
                                }
                            }
                        }
                    }
                    if skip { continue; }
                    if socket.send(axum::extract::ws::Message::Text(envelope.into())).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Canonical pattern: close on lag. Client reconnects with
                    // its current last_seq and the handshake starts over.
                    // Do NOT try to recover in place.
                    let _ = socket.close().await;
                    break;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}
```

You may simplify, refactor, or extract helpers. The above is a reference implementation showing the expected control flow — your implementation must match the SEMANTICS but may differ in style.

## Reality tests (≥6, all in `mod pantheon_ws_replay_tests` at the bottom of main.rs)

Use a real ephemeral Axum server bound to a random port (`TcpListener::bind("127.0.0.1:0")`) and a real `tokio_tungstenite` client. The `tokio-tungstenite = "0.28"` dep is already in `daemon/crates/triumvirate/Cargo.toml`.

Required tests:

1. **`api_state_returns_full_snapshot_with_version_and_uptime`** — populate state with 2 abe_tasks (via state.abe_tasks.register) + 1 fleet build (via the fleet_v2_states lock pattern). Sleep ~5ms so `state.started_at.elapsed()` is nonzero. GET `/api/state` with bearer. Assert parsed `StateResponse.version == daemon_core::VERSION.to_string()` (do NOT hardcode — compare against the constant), `uptime_ms > 0`, `workers.len() == 2` (ABE workers only, sessions are NOT aggregated), `fleet.len() == 1`, `last_event_seq` is a valid u64 (starts at 0 in an empty-events test).

2. **`api_state_rejects_missing_bearer`** — GET `/api/state` no Authorization → 401.

3. **`ws_v2_replays_events_within_range_wrapped_in_envelope`** — start ephemeral server (`TcpListener::bind("127.0.0.1:0")`). Pre-load `state.replay_buffer` with 5 AgentStreamEvent::ToolCall events with seq 1..=5 (call `state.replay_buffer.push(event)` directly). Connect via `tokio_tungstenite`. Send `{"action":"subscribe","last_seq":0}`. First client frame → parse as `ReplayResponse`, assert `replay == "ok"`. Next 5 frames → parse each as a `serde_json::Value`, assert `value["type"] == "agent_stream"`, assert `value["payload"]` deserializes into `AgentStreamEvent` with seq 1, 2, 3, 4, 5 in that order. **A stub that sends bare AgentStreamEvent JSON (no envelope) fails the `type` assertion; a stub that returns 0 events fails the count.**

4. **`ws_v2_returns_out_of_range_when_client_too_far_behind`** — pre-load buffer via push() with 1500 events seq 1..=1500 (the 1000-capacity buffer will evict down to 501..=1500). Connect, send `{"action":"subscribe","last_seq":200}`. First (and only) client frame → parse as `ReplayResponse`, assert `replay == "out_of_range"` AND `oldest_seq == Some(501)`. Assert the connection then closes (next recv returns None or a Close frame). Critically, assert the out-of-range frame is a BARE JSON object without the envelope (no `type` field, no `payload` field at top level). **A stub that returns events anyway fails this.**

5. **`ws_v2_at_boundary_replays_correctly_with_envelope`** — pre-load with seq 50..=60. Connect with `last_seq:50`. Drain ack. Drain the next 10 frames → each must be an envelope (`type == "agent_stream"`) with payload seqs 51..=60 in order (NOT seq=50 itself).

6. **`ws_v2_live_tail_after_historical_replay_preserves_envelope`** — pre-load with seq 1..=3. Connect, send `{"action":"subscribe","last_seq":0}`. Drain ack + 3 envelope-wrapped historical events. THEN publish a NEW event by calling `state.ws_events.send(daemon_core::encode_ws_event("agent_stream", serde_json::to_value(&make_event(4)).unwrap()))`. Wait up to 500ms. Drain the next frame → assert it's an envelope with payload seq=4. **This proves replay and live use the SAME wire format.** Note: the buffer-fill task from T-007.5 will also push it into replay_buffer in the background — that's fine, the test doesn't assert anything about the buffer after the fact.

7. **`ws_v2_dedups_overlap_between_historical_and_live`** — pre-load buffer with seq 1..=5. Connect with `last_seq:0`. Drain ack + the 5 envelope-wrapped historical events. After draining, manually publish a duplicate envelope for seq=3 via `state.ws_events.send(encode_ws_event("agent_stream", ...))`. Wait ~200ms. Use a tokio::select! with a timeout to try to read one more frame; assert the read times out (no frame arrives). The dedup check by max_sent should catch the overlap. **A stub without dedup fails this — the client would receive a second seq=3 frame.**

8. **`ws_v2_rejects_missing_bearer_on_upgrade`** — attempt WebSocket upgrade to `/ws/v2` with no Authorization header. Assert the HTTP response status is 401 BEFORE the WebSocket switches protocols (the tokio_tungstenite connect call should return an Err, not a successful upgrade). A handler that auths inside `on_upgrade` (after the protocol switch) fails this test because the switch already happened.

9. **`legacy_ws_route_unchanged`** — connect to `/ws` (NOT `/ws/v2`) and assert it emits the 4 hardcoded bootstrap events from `daemon_http::ws_route` (`agent_state`, `fleet_progress`, `ledger_health`, `review_completed`) — same behavior as before T-009 landed. This is the backwards-compat regression check for the `triumvirate watch` CLI.

## Verify commands

```bash
cargo check -p triumvirate
cargo test -p triumvirate --bin triumvirate -- pantheon_ws_replay_tests
cargo test -p triumvirate --bin triumvirate -- http_mcp::tests pantheon_stdio_meta_tests abe::task_tracker::tests
cargo test -p daemon-core -- replay_fill_tests pantheon_session::tests replay::tests
```

All four must pass. The first three are the same shape as T-008's verify list. The fourth confirms no daemon-core regression.

## Done when

- `/api/state` and `/ws/v2` routes registered in `run_daemon`.
- Both handlers implemented per the contract above.
- Subscribe-before-read pattern implemented correctly (subscribe FIRST, replay SECOND, dedup THIRD, live tail FOURTH).
- 9+ reality tests in `mod pantheon_ws_replay_tests` PASS.
- Legacy `/ws` regression test passes (existing route untouched).
- `cargo check --workspace --tests` clean.
- Commit message starts with `T-009:` and references REQ-020.

## Forbidden actions

- Do NOT modify the existing `/ws` route or `daemon-http::ws_route`.
- Do NOT modify EventReplayBuffer or ReplayResult.
- Do NOT modify shared_types.
- Do NOT modify DaemonState fields.
- Do NOT add a new crate dependency. tokio-tungstenite, axum, futures-util, tower are all already deps.
- Do NOT use `:build_id` syntax.
- Do NOT add middleware.
- Do NOT handle `RecvError::Lagged` by trying to recover — close the connection per the canonical pattern.
- Do NOT include any `TODO`/`FIXME`/`unimplemented!()`/`todo!()`.
- Do NOT call `state.ws_events.subscribe()` AFTER reading the buffer — that's the exact race condition this whole task is built to avoid.

## How to start

1. Read `/Users/you/projects/triumvirate/daemon/crates/daemon-http/src/lib.rs` lines 350-395 — that's the existing `/ws` route. Understand what it does. Do NOT copy it; do NOT modify it.
2. Read `/Users/you/projects/triumvirate/daemon/crates/daemon-core/src/replay.rs` — `EventReplayBuffer.push` and `replay_since` are the only public methods you'll call.
3. Read `/Users/you/projects/triumvirate/daemon/crates/shared-types/src/api.rs` — confirm the exact field names and types of `StateResponse`, `ReplayRequest`, `ReplayResponse`.
4. Read `/Users/you/projects/triumvirate/daemon/crates/shared-types/src/streaming.rs` — `AgentStreamEvent::seq()` is how you get the sequence number for dedup.
5. Read `/Users/you/projects/triumvirate/daemon/crates/triumvirate/src/main.rs` lines 1395-1445 — the existing per-handler bearer auth pattern.
6. Implement the two handlers.
7. Register the routes in the chain inside `run_daemon`.
8. Write the test module.
9. Run the verify commands.
10. Commit with `T-009: GET /api/state + /ws/v2 replay handshake — REQ-020 — FEAT-013`.

## The bake-off

Two implementers (Apollo and Athena) will independently produce a diff against this manifest. Gemini will review BOTH diffs and pick the winner. Don't try to outsmart your peer — implement faithfully against the contract. Style variation is fine; scope variation is not.
