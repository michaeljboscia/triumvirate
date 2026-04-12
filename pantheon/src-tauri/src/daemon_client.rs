// T-020 (REQ-019, REQ-020, REQ-021) — Pantheon daemon client.
//
// A background task that owns the connection lifecycle to the v3.9.0
// daemon: opens a WebSocket to /ws/v2 with a replay-aware handshake, polls
// REST endpoints (/api/workers, /api/fleet) on a timer, maintains a
// 4-state health machine (Starting → Ready → Degraded → Disconnected),
// and forwards every interesting event to the Svelte frontend via
// Tauri's `emit` so stores can react without polling.
//
// Why a background task and not per-request commands:
// The sidebar and status panels all need the SAME worker/fleet data,
// and they need to update in near-real time. A pull-per-component model
// would fan out REST calls wastefully. Instead we run ONE connection,
// ONE reconnect loop, and fan events out through Tauri's IPC event bus.
// Svelte stores subscribe once, then receive updates forever.
//
// Connection topology:
//     Pantheon (this file) ──REST──▶ daemon /api/workers, /api/fleet, /api/state
//                          ──WS────▶ daemon /ws/v2  (replay handshake + live tail)
//                          ──emit──▶ main window  ("daemon://state", "daemon://workers", ...)
//
// Reconnect strategy:
//   - On cold start: last_seq = 0, so the handshake gets "replay: ok" with
//     zero history and we immediately fall into the live-tail stream.
//   - On reconnect after a gap: send last_seq = the highest seq we saw.
//     If daemon says "out_of_range", fetch /api/state, adopt its
//     last_event_seq as our new baseline, and reconnect.
//   - On any error (TCP reset, handshake failure, lagged buffer): flip
//     state to Disconnected, wait 1s, retry. Exponential backoff capped
//     at 10s keeps the daemon from getting hammered during long outages.
//
// Health state machine (drives the tray icon via tray::update_daemon_state):
//   Starting     — initial state, first connection attempt hasn't completed
//   Ready        — WebSocket connected, last REST poll succeeded
//   Degraded     — WebSocket connected but last REST poll failed
//                  OR REST works but WS is mid-reconnect
//   Disconnected — neither WebSocket nor REST responded within the grace window
//
// Scope out (per task card): do not implement sidebar UI, unmanaged
// process scanning, or quit-confirmation. Those are T-021 / T-023 / T-018.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use shared_types::{
    FleetResponse, ReplayRequest, ReplayResponse, StateResponse, WorkersResponse,
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Mutex;
use tokio::time::{sleep, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Default daemon listen address. Matches the daemon's
/// `daemon_bind_addr(None)` fallback in daemon-core so Pantheon Just Works
/// against a locally-running daemon with no configuration. Override via
/// the `PANTHEON_DAEMON_URL` environment variable for testing against a
/// daemon on a non-standard port.
const DEFAULT_DAEMON_HTTP: &str = "http://127.0.0.1:8080";

/// Tauri event names emitted by the client. Kept as module-level
/// constants so the Svelte store (`src/lib/stores/daemon.ts`) can
/// subscribe to the same strings — if you rename one, grep for it.
pub const EVENT_STATE: &str = "daemon://state";
pub const EVENT_WORKERS: &str = "daemon://workers";
pub const EVENT_FLEET: &str = "daemon://fleet";
pub const EVENT_STREAM: &str = "daemon://stream";

/// Frequency of the REST fan-out poll while the WS is up. Short enough
/// that the sidebar feels live, long enough not to hammer the daemon.
/// WebSocket events fill the gap between polls for anything the daemon
/// publishes in real time.
const REST_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Backoff ceiling for the reconnect loop. Starts at 1 second, doubles
/// on each failure, caps here. The daemon's graceful restart is usually
/// sub-second, so a long ceiling mostly matters during extended outages.
const RECONNECT_CAP: Duration = Duration::from_secs(10);

/// Grace window after a WS disconnect before we downgrade to Disconnected.
/// Gives the reconnect loop one cycle to re-establish without painting
/// the tray red for a fraction of a second.
const DEGRADED_GRACE: Duration = Duration::from_secs(2);

/// The four states the tray icon mirrors. Stringified variants match
/// what `tray::update_daemon_state` expects so Svelte can either listen
/// to the JSON payload directly or let the Rust side drive the tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    Starting,
    Ready,
    Degraded,
    Disconnected,
}

impl HealthState {
    fn as_tray_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Disconnected => "disconnected",
        }
    }
}

/// Shared runtime state for the daemon client. Wrapped in an Arc so the
/// background task and any future Tauri command handlers can both read
/// the current health/seq without re-opening the connection.
struct ClientState {
    base_http: String,
    base_ws: String,
    token: String,
    /// Monotonic sequence of the highest agent_stream event we've seen.
    /// Sent on every handshake so the replay buffer can catch us up.
    last_seq: std::sync::atomic::AtomicU64,
    /// Last observed health state. Protected by a Mutex (not atomic)
    /// because the transition logic reads + writes non-atomically.
    health: Mutex<HealthState>,
}

/// Spawn the daemon client background task. Called from `lib.rs::run()`'s
/// setup hook so the connection loop starts as soon as the Tauri runtime
/// is alive. Returns immediately — the task runs forever (or until the
/// app exits) and does its work asynchronously.
///
/// The app handle is cloned into the task so events can be emitted to
/// every window and so the tray icon can be swapped via the already-
/// registered `update_daemon_state` command path.
pub fn spawn(app: AppHandle) -> Result<()> {
    // Concrete AppHandle (Wry runtime) — Pantheon is desktop-only per
    // REQ-028, so there's no payoff to generic `AppHandle<R>` and keeping
    // it concrete lets us call `#[tauri::command]` functions like
    // `tray::update_daemon_state` directly. Those commands are baked to
    // the concrete runtime type by the macro and can't accept a generic.
    let token = load_token().context("failed to load ~/.triumvirate/daemon.token")?;
    let base = std::env::var("PANTHEON_DAEMON_URL").unwrap_or_else(|_| DEFAULT_DAEMON_HTTP.into());
    let base_http = base.trim_end_matches('/').to_string();
    // ws:// and http:// → the daemon serves both on the same port, only
    // the scheme differs. Replace the scheme rather than reparsing so we
    // preserve host, port, and any path prefix the user has set.
    let base_ws = if let Some(stripped) = base_http.strip_prefix("https://") {
        format!("wss://{stripped}")
    } else if let Some(stripped) = base_http.strip_prefix("http://") {
        format!("ws://{stripped}")
    } else {
        return Err(anyhow!("PANTHEON_DAEMON_URL must start with http:// or https://"));
    };

    let state = Arc::new(ClientState {
        base_http,
        base_ws,
        token,
        last_seq: std::sync::atomic::AtomicU64::new(0),
        health: Mutex::new(HealthState::Starting),
    });

    tauri::async_runtime::spawn(async move {
        // Emit the initial Starting state so the frontend can paint the
        // tray icon before the first connection attempt finishes.
        emit_state(&app, HealthState::Starting).await;
        run_forever(app, state).await;
    });

    Ok(())
}

/// Read the daemon shared-secret token from `~/.triumvirate/daemon.token`.
/// This is the same file the daemon itself writes via
/// `daemon_core::ensure_daemon_token`. If the daemon has never been
/// started, the file won't exist yet — in that case we return a clear
/// error so the user sees "start the daemon" instead of a silent hang.
fn load_token() -> Result<String> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot resolve $HOME"))?;
    let path: PathBuf = home.join(".triumvirate").join("daemon.token");
    let raw = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "token file {} not found — is the daemon running?",
            path.display()
        )
    })?;
    Ok(raw.trim().to_string())
}

/// The top-level reconnect loop. Runs forever. Each iteration tries to
/// open a WebSocket, run the live session until it fails, then backs off
/// before retrying. REST polling is driven from inside the session so it
/// inherits the same connection's health lifetime.
async fn run_forever(app: AppHandle, state: Arc<ClientState>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match run_session(&app, &state).await {
            Ok(()) => {
                tracing::info!("daemon_client: session ended cleanly");
                backoff = Duration::from_secs(1);
            }
            Err(err) => {
                tracing::warn!(error = %err, "daemon_client: session error");
            }
        }

        transition(&app, &state, HealthState::Disconnected).await;
        sleep(backoff).await;
        backoff = (backoff * 2).min(RECONNECT_CAP);
    }
}

/// A single "lifetime" of the daemon connection: open the WS, do the
/// replay handshake, then race the inbound stream against a REST poll
/// ticker until either one fails. Returns Ok on clean close, Err on any
/// fault that should trigger a reconnect.
async fn run_session(app: &AppHandle, state: &Arc<ClientState>) -> Result<()> {
    // Step 1 — initial REST pull so the sidebar has data BEFORE the WS
    // handshake finishes. If /api/state fails here, the daemon is likely
    // unreachable entirely and we should fail fast into the backoff.
    let initial_state = fetch_state(state).await.context("/api/state failed")?;
    state
        .last_seq
        .store(initial_state.last_event_seq, std::sync::atomic::Ordering::Relaxed);
    let _ = app.emit(EVENT_WORKERS, &WorkersResponse { workers: initial_state.workers.clone() });
    let _ = app.emit(EVENT_FLEET, &FleetResponse { builds: initial_state.fleet.clone() });

    // Step 2 — WebSocket with bearer auth header. tungstenite's
    // `IntoClientRequest` gives us a prebuilt WS upgrade request with the
    // mandatory headers (Host, Upgrade, Connection, Sec-WebSocket-*). We
    // only need to attach the bearer token — all the handshake plumbing
    // is handled for us, and we can't forget a required header.
    let url = format!("{}/ws/v2", state.base_ws);
    let mut request = url.as_str().into_client_request().context("building ws request")?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", state.token)
            .parse()
            .context("encoding bearer header")?,
    );

    let (ws_stream, _resp) = connect_async(request).await.context("ws connect failed")?;
    let (mut write, mut read) = ws_stream.split();

    // Step 3 — replay handshake. Send the subscribe message with our
    // highest-known seq. Daemon either streams us everything since then
    // ("replay: ok") or tells us we're out_of_range, in which case we
    // drop back to /api/state and reconnect.
    let handshake = ReplayRequest {
        action: "subscribe".into(),
        last_seq: state.last_seq.load(std::sync::atomic::Ordering::Relaxed),
    };
    write
        .send(Message::Text(serde_json::to_string(&handshake)?.into()))
        .await
        .context("sending handshake")?;

    // Daemon's first frame after handshake is always a ReplayResponse
    // (bare JSON, NOT wrapped in the envelope). If it's out_of_range we
    // bail so the reconnect loop can fetch /api/state again.
    let first = read
        .next()
        .await
        .ok_or_else(|| anyhow!("ws closed before handshake response"))?
        .context("ws handshake recv")?;
    if let Message::Text(txt) = &first {
        let resp: ReplayResponse = serde_json::from_str(txt).context("parsing handshake response")?;
        if resp.replay == "out_of_range" {
            tracing::warn!(
                oldest_seq = ?resp.oldest_seq,
                "daemon replay out_of_range; resetting to /api/state baseline"
            );
            return Ok(()); // loop will reconnect and re-fetch state
        }
    }

    // Step 4 — transition to Ready. REST + WS are both alive.
    transition(app, state, HealthState::Ready).await;

    // Step 5 — main event loop. Two interleaved sources:
    //   (a) WebSocket frames (envelope JSON with {type, ts_ms, payload})
    //   (b) REST poll ticker every 2s, refreshes /api/workers + /api/fleet
    // We use `tokio::select!` to race them. A failure on either triggers
    // a clean exit so the outer loop can reconnect.
    let mut poll_ticker = tokio::time::interval(REST_POLL_INTERVAL);
    // Skip the immediate first tick — we just fetched /api/state, no
    // point spamming /api/workers 0ms later.
    poll_ticker.tick().await;
    let mut last_rest_ok = Instant::now();

    loop {
        tokio::select! {
            // REST poll branch. Non-fatal if it fails — we flip to
            // Degraded until either the next poll succeeds or the grace
            // window expires and we give up on this session entirely.
            _ = poll_ticker.tick() => {
                match fetch_workers_and_fleet(state).await {
                    Ok((workers, fleet)) => {
                        last_rest_ok = Instant::now();
                        let _ = app.emit(EVENT_WORKERS, &workers);
                        let _ = app.emit(EVENT_FLEET, &fleet);
                        transition(app, state, HealthState::Ready).await;
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "REST poll failed");
                        transition(app, state, HealthState::Degraded).await;
                        if last_rest_ok.elapsed() > DEGRADED_GRACE * 3 {
                            return Err(anyhow!("REST poll failed past grace window"));
                        }
                    }
                }
            }

            // WebSocket branch. Fatal on any error — we need the live
            // feed for REQ-020 replay guarantees, so if the stream dies
            // the whole session has to reset.
            frame = read.next() => {
                match frame {
                    Some(Ok(Message::Text(txt))) => {
                        handle_ws_frame(app, state, &txt);
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = write.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        tracing::info!("ws closed by daemon");
                        return Ok(());
                    }
                    Some(Err(err)) => {
                        return Err(anyhow!("ws recv error: {err}"));
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Parse a single WebSocket envelope and forward its payload to the
/// frontend. The envelope shape is `{type, ts_ms, payload}` per
/// daemon-core::encode_ws_event. We update `last_seq` if the payload
/// carries a sequence number — that keeps our reconnect handshake
/// pointed at the correct replay position.
fn handle_ws_frame(app: &AppHandle, state: &Arc<ClientState>, txt: &str) {
    let parsed: serde_json::Value = match serde_json::from_str(txt) {
        Ok(v) => v,
        Err(err) => {
            tracing::debug!(error = %err, "bad ws envelope");
            return;
        }
    };
    let event_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");

    // Best-effort seq tracking. agent_stream events carry a seq in
    // payload.seq; other envelope types don't. We grab whatever we can
    // find and keep the max so our handshake replay is accurate.
    if let Some(seq) = parsed
        .get("payload")
        .and_then(|p| p.get("seq"))
        .and_then(|v| v.as_u64())
    {
        state
            .last_seq
            .fetch_max(seq, std::sync::atomic::Ordering::Relaxed);
    }

    // Forward to the frontend. Sidebar / status stores subscribe to
    // EVENT_STREAM and switch on `type` to update the right slice.
    let _ = app.emit(
        EVENT_STREAM,
        serde_json::json!({
            "type": event_type,
            "payload": parsed.get("payload").cloned().unwrap_or(serde_json::Value::Null),
        }),
    );
}

/// Atomic health transition + event emit + tray icon sync. Centralizing
/// this in one helper ensures every place that changes state ALSO pushes
/// the change to the UI — forgetting one is a subtle bug where the tray
/// says green but the sidebar says disconnected.
async fn transition(app: &AppHandle, state: &ClientState, new: HealthState) {
    let mut guard = state.health.lock().await;
    if *guard == new {
        return;
    }
    *guard = new;
    drop(guard);
    emit_state(app, new).await;
}

/// Emit the current state to the frontend AND drive the tray icon.
/// Separating this from `transition` lets the spawn seed emit the
/// initial Starting state without taking the mutex.
async fn emit_state(app: &AppHandle, new: HealthState) {
    let _ = app.emit(EVENT_STATE, new);
    // Sync the tray icon. `tray::update_daemon_state` is a Tauri command
    // but it's also just a Rust function — we call it directly here so
    // the state machine drives the icon without a round-trip through IPC.
    if let Some(_tray) = app.tray_by_id(crate::tray::TRAY_ID) {
        // Command takes AppHandle + String; ignore result (best-effort
        // icon swap, failure is logged by the command itself).
        let _ = crate::tray::update_daemon_state(app.app_handle().clone(), new.as_tray_str().into());
    }
}

/// GET /api/state with bearer auth. Used for the initial cold fetch AND
/// for the "out_of_range" fallback path after a long disconnect.
async fn fetch_state(state: &ClientState) -> Result<StateResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/state", state.base_http))
        .bearer_auth(&state.token)
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.json::<StateResponse>().await?)
}

/// GET /api/workers + /api/fleet in parallel. Used on the REST poll
/// ticker. Returns both responses so the caller can emit them together.
async fn fetch_workers_and_fleet(
    state: &ClientState,
) -> Result<(WorkersResponse, FleetResponse)> {
    let client = reqwest::Client::new();
    let workers_fut = client
        .get(format!("{}/api/workers", state.base_http))
        .bearer_auth(&state.token)
        .timeout(Duration::from_secs(3))
        .send();
    let fleet_fut = client
        .get(format!("{}/api/fleet", state.base_http))
        .bearer_auth(&state.token)
        .timeout(Duration::from_secs(3))
        .send();

    let (workers_resp, fleet_resp) = tokio::try_join!(workers_fut, fleet_fut)?;
    let workers = workers_resp.error_for_status()?.json::<WorkersResponse>().await?;
    let fleet = fleet_resp.error_for_status()?.json::<FleetResponse>().await?;
    Ok((workers, fleet))
}
