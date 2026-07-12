// Pre-existing lint debt acknowledged in PR #29; remove allow once cleaned up.
#![allow(clippy::unnecessary_sort_by)]

use axum::{
    Json as AxumJson,
    body::Body,
    extract::{Path, Query, State, WebSocketUpgrade, ws::Message},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
};
use daemon_core::{
    QueueRegistry, acquire_project_queue as core_acquire_project_queue,
    append_memory_entry as core_append_memory_entry, ensure_daemon_token as core_ensure_daemon_token,
    list_scratchpad as core_list_scratchpad, metrics::DaemonMetrics, project_queue_key as core_project_queue_key,
    read_memory_entries as core_read_memory_entries, triumvirate_home_dir as core_triumvirate_home_dir,
    unix_time_ms as core_unix_time_ms, write_scratchpad as core_write_scratchpad,
};
use prometheus::{Encoder, TextEncoder};
use fallback_outbox::{
    acknowledge_fallback_path, gc_fallbacks, list_pending_fallback_paths, read_outbox_events,
};
use ledger::LedgerStore;
use mcp_bridge::{
    daemon_ask_agent_url, daemon_fallback_ack_url, daemon_fallback_gc_url, daemon_fallback_list_url,
    daemon_autostart_enabled, daemon_health_url, daemon_memory_read_url, daemon_memory_write_url,
    daemon_lesson_add_url, daemon_lesson_list_url, daemon_lesson_query_url, daemon_lesson_validate_url,
    daemon_ledger_gc_url, daemon_ledger_query_url, daemon_ledger_record_url, daemon_ledger_session_url,
    daemon_outbox_recent_url, daemon_scratchpad_list_url, daemon_scratchpad_write_url,
    daemon_session_ask_url, daemon_session_dismiss_url, daemon_session_list_url, daemon_session_spawn_url,
    daemon_status_url, should_use_daemon_proxy,
    is_bearer_authorized,
};
use serde::Deserialize;
use shared_types::{
    AskAgentRequest, AskAgentResponse, AskSessionRequest, DaemonHealthResponse, DaemonStatusSnapshot,
    DismissSessionRequest, FallbackAckRequest, FallbackGcRequest, FallbackGcResponse, FallbackListRequest,
    FallbackListResponse, GcResult, HealthStatus, LedgerQueryRequest, LedgerQueryResponse, LedgerSessionRequest,
    Lesson, LessonAddResponse,
    LessonListRequest, LessonListResponse, LessonQueryRequest, LessonQueryResponse, LessonValidateRequest, ManualRecord,
    MemoryEntry, MemoryReadRequest, MemoryReadResponse, MemoryWriteRequest, MemoryWriteResponse, NewLesson,
    SessionDetail, Summary,
    OutboxRecentRequest, OutboxRecentResponse, ScratchpadListRequest, ScratchpadListResponse,
    ScratchpadWriteRequest, ScratchpadWriteResponse, SessionListResponse, SpawnSessionRequest,
};
use std::{
    collections::VecDeque,
    fs,
    future::Future,
    path::Component,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        LazyLock,
    },
};
use tokio::{
    sync::{Mutex, broadcast},
    time::{Duration, Instant, sleep},
};
use token_economics::{BuildCostBreakdown, SessionTokenBreakdown, SummaryQueryFilters};
use tracing::warn;
use uuid::Uuid;

static DAEMON_AUTOSTART_ATTEMPTED: AtomicBool = AtomicBool::new(false);
static DAEMON_HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .pool_max_idle_per_host(8)
        .pool_idle_timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build shared daemon HTTP client")
});
const DEFAULT_DAEMON_HTTP_TIMEOUT_SECS: u64 = 30;
const DEFAULT_DAEMON_ASK_TIMEOUT_SECS: u64 = 180;

pub fn reset_daemon_autostart_flag_for_tests() {
    DAEMON_AUTOSTART_ATTEMPTED.store(false, Ordering::SeqCst);
}

pub fn attempt_daemon_autostart_once() -> anyhow::Result<bool> {
    if !daemon_autostart_enabled(std::env::var("TRIUMVIRATE_DAEMON_AUTOSTART").ok().as_deref()) {
        return Ok(false);
    }
    if DAEMON_AUTOSTART_ATTEMPTED.swap(true, Ordering::SeqCst) {
        return Ok(false);
    }

    if should_use_daemon_proxy(std::env::var("TRIUMVIRATE_DAEMON_AUTOSTART_DRYRUN").ok().as_deref()) {
        return Ok(true);
    }

    let exe = std::env::current_exe()?;
    let _child = std::process::Command::new(exe)
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(true)
}

async fn daemon_get_json<T: serde::de::DeserializeOwned>(url: String) -> anyhow::Result<T> {
    let token = core_ensure_daemon_token(&core_triumvirate_home_dir()?)?;
    let client = &*DAEMON_HTTP_CLIENT;
    let timeout = daemon_http_timeout();

    let first = client.get(&url).bearer_auth(&token).timeout(timeout).send().await;
    match first {
        Ok(response) => {
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                let body = body.trim();
                if body.is_empty() {
                    anyhow::bail!("daemon responded with HTTP {status}");
                }
                anyhow::bail!("daemon responded with HTTP {status}: {}", &body[..body.len().min(300)]);
            }
            Ok(response.json::<T>().await?)
        }
        Err(e) => {
            if attempt_daemon_autostart_once().unwrap_or(false) {
                sleep(Duration::from_millis(300)).await;
                let retry = client.get(&url).bearer_auth(token).timeout(timeout).send().await?;
                if !retry.status().is_success() {
                    let status = retry.status();
                    let body = retry.text().await.unwrap_or_default();
                    let body = body.trim();
                    if body.is_empty() {
                        anyhow::bail!("daemon responded with HTTP {status}");
                    }
                    anyhow::bail!("daemon responded with HTTP {status}: {}", &body[..body.len().min(300)]);
                }
                return Ok(retry.json::<T>().await?);
            }
            anyhow::bail!("daemon request failed: {e}")
        }
    }
}

async fn daemon_post_json<TReq: serde::Serialize, TResp: serde::de::DeserializeOwned>(
    url: String,
    payload: &TReq,
) -> anyhow::Result<TResp> {
    daemon_post_json_with_timeout(url, payload, daemon_http_timeout()).await
}

/// Carries the current trace context into outbound headers as W3C `traceparent`.
///
/// The MCP bridge and the daemon are SEPARATE PROCESSES, so without this every call produces two
/// unrelated traces: the bridge's spans in one, the daemon's `ask_agent` in another, with no way
/// to know they were the same request. Injecting `traceparent` lets the daemon adopt the bridge's
/// trace as its parent, and the whole call finally reads as one trace end to end.
struct HeaderInjector<'a>(&'a mut reqwest::header::HeaderMap);

impl opentelemetry::propagation::Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(key.as_bytes()),
            reqwest::header::HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, val);
        }
    }
}

/// Reads W3C `traceparent` off an inbound request.
struct HeaderExtractor<'a>(&'a HeaderMap);

impl opentelemetry::propagation::Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

/// Make the current span a child of the caller's trace, if the caller sent one.
///
/// No-op when there is no `traceparent` (a direct HTTP client, or tracing not configured): the
/// propagator returns an invalid context and `set_parent` leaves the span as a root. So this is
/// safe on every path, not just the MCP one.
pub fn adopt_remote_trace_parent(headers: &HeaderMap) {
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    let parent_cx = opentelemetry::global::get_text_map_propagator(|prop| {
        prop.extract(&HeaderExtractor(headers))
    });
    tracing::Span::current().set_parent(parent_cx);
}

/// Headers carrying the current span's trace context. Empty when tracing is not configured —
/// `TraceContextPropagator` simply injects nothing for an invalid/absent context.
fn trace_headers() -> reqwest::header::HeaderMap {
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    let mut headers = reqwest::header::HeaderMap::new();
    let cx = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|prop| {
        prop.inject_context(&cx, &mut HeaderInjector(&mut headers));
    });
    headers
}

async fn daemon_post_json_with_timeout<TReq: serde::Serialize, TResp: serde::de::DeserializeOwned>(
    url: String,
    payload: &TReq,
    timeout: Duration,
) -> anyhow::Result<TResp> {
    let token = core_ensure_daemon_token(&core_triumvirate_home_dir()?)?;
    let client = &*DAEMON_HTTP_CLIENT;

    let first = client
        .post(&url)
        .bearer_auth(&token)
        .headers(trace_headers())
        .json(payload)
        .timeout(timeout)
        .send()
        .await;
    match first {
        Ok(response) => {
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                let body = body.trim();
                if body.is_empty() {
                    anyhow::bail!("daemon responded with HTTP {status}");
                }
                anyhow::bail!("daemon responded with HTTP {status}: {}", &body[..body.len().min(300)]);
            }
            Ok(response.json::<TResp>().await?)
        }
        Err(e) => {
            if attempt_daemon_autostart_once().unwrap_or(false) {
                sleep(Duration::from_millis(300)).await;
                let retry = client
                    .post(&url)
                    .bearer_auth(token)
                    .headers(trace_headers())
                    .json(payload)
                    .timeout(timeout)
                    .send()
                    .await?;
                if !retry.status().is_success() {
                    let status = retry.status();
                    let body = retry.text().await.unwrap_or_default();
                    let body = body.trim();
                    if body.is_empty() {
                        anyhow::bail!("daemon responded with HTTP {status}");
                    }
                    anyhow::bail!("daemon responded with HTTP {status}: {}", &body[..body.len().min(300)]);
                }
                return Ok(retry.json::<TResp>().await?);
            }
            anyhow::bail!("daemon request failed: {e}")
        }
    }
}

fn daemon_http_timeout() -> Duration {
    Duration::from_secs(
        std::env::var("TRIUMVIRATE_DAEMON_HTTP_TIMEOUT_SECS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(DEFAULT_DAEMON_HTTP_TIMEOUT_SECS),
    )
}

fn daemon_ask_timeout() -> Duration {
    Duration::from_secs(
        std::env::var("TRIUMVIRATE_DAEMON_ASK_TIMEOUT_SECS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(DEFAULT_DAEMON_ASK_TIMEOUT_SECS),
    )
}

pub async fn fetch_daemon_status() -> anyhow::Result<DaemonHealthResponse> {
    if let Ok(health) = daemon_get_json::<DaemonHealthResponse>(daemon_health_url()).await {
        return Ok(health);
    }

    let status_json = daemon_get_json::<serde_json::Value>(daemon_status_url()).await?;
    Ok(DaemonHealthResponse {
        status: status_json
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("ok")
            .to_string(),
        version: status_json
            .get("version")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        service: status_json
            .get("service")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        mode: status_json
            .get("mode")
            .or_else(|| status_json.get("daemon_mode"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        daemon: status_json
            .get("daemon")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        auth: status_json
            .get("auth")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        daemon_bind_addr: status_json
            .get("daemon_bind_addr")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
    })
}

pub async fn fetch_daemon_status_snapshot() -> anyhow::Result<DaemonStatusSnapshot> {
    daemon_get_json::<DaemonStatusSnapshot>(daemon_status_url()).await
}

pub async fn fetch_daemon_ask_agent(req: &AskAgentRequest) -> anyhow::Result<AskAgentResponse> {
    daemon_post_json_with_timeout::<AskAgentRequest, AskAgentResponse>(daemon_ask_agent_url(), req, daemon_ask_timeout()).await
}

pub async fn fetch_daemon_session_spawn(req: &SpawnSessionRequest) -> anyhow::Result<String> {
    let json =
        daemon_post_json::<SpawnSessionRequest, serde_json::Value>(daemon_session_spawn_url(), req).await?;
    Ok(json
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("session spawned")
        .to_string())
}

pub async fn fetch_daemon_session_ask(req: &AskSessionRequest) -> anyhow::Result<String> {
    let json = daemon_post_json_with_timeout::<AskSessionRequest, serde_json::Value>(
        daemon_session_ask_url(),
        req,
        daemon_ask_timeout(),
    )
    .await?;
    Ok(json
        .get("response")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string())
}

pub async fn fetch_daemon_session_dismiss(req: &DismissSessionRequest) -> anyhow::Result<String> {
    let json =
        daemon_post_json::<DismissSessionRequest, serde_json::Value>(daemon_session_dismiss_url(), req).await?;
    Ok(json
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("session dismissed")
        .to_string())
}

pub async fn fetch_daemon_session_list() -> anyhow::Result<SessionListResponse> {
    daemon_get_json::<SessionListResponse>(daemon_session_list_url()).await
}

pub async fn fetch_daemon_memory_write(req: &MemoryWriteRequest) -> anyhow::Result<MemoryWriteResponse> {
    daemon_post_json::<MemoryWriteRequest, MemoryWriteResponse>(daemon_memory_write_url(), req).await
}

pub async fn fetch_daemon_memory_read(req: &MemoryReadRequest) -> anyhow::Result<MemoryReadResponse> {
    daemon_post_json::<MemoryReadRequest, MemoryReadResponse>(daemon_memory_read_url(), req).await
}

pub async fn fetch_daemon_scratchpad_write(
    req: &ScratchpadWriteRequest,
) -> anyhow::Result<ScratchpadWriteResponse> {
    daemon_post_json::<ScratchpadWriteRequest, ScratchpadWriteResponse>(daemon_scratchpad_write_url(), req).await
}

pub async fn fetch_daemon_scratchpad_list(
    req: &ScratchpadListRequest,
) -> anyhow::Result<ScratchpadListResponse> {
    daemon_post_json::<ScratchpadListRequest, ScratchpadListResponse>(daemon_scratchpad_list_url(), req).await
}

pub async fn fetch_daemon_outbox_recent(
    req: &OutboxRecentRequest,
) -> anyhow::Result<OutboxRecentResponse> {
    daemon_post_json::<OutboxRecentRequest, OutboxRecentResponse>(daemon_outbox_recent_url(), req).await
}

pub async fn fetch_daemon_fallback_list(req: &FallbackListRequest) -> anyhow::Result<FallbackListResponse> {
    daemon_post_json::<FallbackListRequest, FallbackListResponse>(daemon_fallback_list_url(), req).await
}

pub async fn fetch_daemon_fallback_ack(req: &FallbackAckRequest) -> anyhow::Result<String> {
    let json = daemon_post_json::<FallbackAckRequest, serde_json::Value>(daemon_fallback_ack_url(), req).await?;
    Ok(json
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("acknowledged")
        .to_string())
}

pub async fn fetch_daemon_fallback_gc(req: &FallbackGcRequest) -> anyhow::Result<FallbackGcResponse> {
    daemon_post_json::<FallbackGcRequest, FallbackGcResponse>(daemon_fallback_gc_url(), req).await
}

pub async fn fetch_daemon_ledger_query(req: &LedgerQueryRequest) -> anyhow::Result<LedgerQueryResponse> {
    daemon_post_json::<LedgerQueryRequest, LedgerQueryResponse>(daemon_ledger_query_url(), req).await
}

pub async fn fetch_daemon_ledger_session(req: &LedgerSessionRequest) -> anyhow::Result<SessionDetail> {
    daemon_post_json::<LedgerSessionRequest, SessionDetail>(daemon_ledger_session_url(), req).await
}

pub async fn fetch_daemon_ledger_record(req: &ManualRecord) -> anyhow::Result<String> {
    let json = daemon_post_json::<ManualRecord, serde_json::Value>(daemon_ledger_record_url(), req).await?;
    Ok(json
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("ok")
        .to_string())
}

pub async fn fetch_daemon_ledger_gc() -> anyhow::Result<GcResult> {
    daemon_post_json::<serde_json::Value, GcResult>(
        daemon_ledger_gc_url(),
        &serde_json::json!({}),
    )
    .await
}

pub async fn fetch_daemon_lesson_add(req: &NewLesson) -> anyhow::Result<LessonAddResponse> {
    daemon_post_json::<NewLesson, LessonAddResponse>(daemon_lesson_add_url(), req).await
}

pub async fn fetch_daemon_lesson_query(req: &LessonQueryRequest) -> anyhow::Result<LessonQueryResponse> {
    daemon_post_json::<LessonQueryRequest, LessonQueryResponse>(daemon_lesson_query_url(), req).await
}

pub async fn fetch_daemon_lesson_validate(req: &LessonValidateRequest) -> anyhow::Result<String> {
    let json =
        daemon_post_json::<LessonValidateRequest, serde_json::Value>(daemon_lesson_validate_url(), req).await?;
    Ok(json
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("ok")
        .to_string())
}

pub async fn fetch_daemon_lesson_list(req: &LessonListRequest) -> anyhow::Result<LessonListResponse> {
    daemon_post_json::<LessonListRequest, LessonListResponse>(daemon_lesson_list_url(), req).await
}

const LEDGER_PROJECT_LRU_CAPACITY: usize = 128;

pub type AskAgentRouteFuture<'a> = Pin<Box<dyn Future<Output = Result<AskAgentResponse, String>> + Send + 'a>>;
pub type AskAgentRouteExecutor =
    Arc<dyn for<'a> Fn(&'a AskAgentRequest) -> AskAgentRouteFuture<'a> + Send + Sync>;

#[derive(Clone)]
pub struct DaemonHttpState {
    pub token: String,
    pub queues: QueueRegistry,
    pub ledger_project_lru: Arc<Mutex<VecDeque<PathBuf>>>,
    pub marker_parse_window: Arc<Mutex<VecDeque<(Instant, bool)>>>,
    pub metrics: Arc<DaemonMetrics>,
    pub ws_events: broadcast::Sender<String>,
    pub token_db: Arc<token_economics::TokenDb>,
    pub ask_agent_executor: AskAgentRouteExecutor,
}

pub fn open_token_db(path: &std::path::Path) -> anyhow::Result<Arc<token_economics::TokenDb>> {
    Ok(Arc::new(token_economics::open(path)?))
}

fn encode_ws_event(event_type: &str, payload: serde_json::Value) -> String {
    serde_json::json!({
        "type": event_type,
        "ts_ms": core_unix_time_ms(),
        "payload": payload
    })
    .to_string()
}

fn publish_ws_event(state: &DaemonHttpState, event_type: &str, payload: serde_json::Value) {
    let _ = state.ws_events.send(encode_ws_event(event_type, payload));
}

pub async fn ws_route(
    State(state): State<DaemonHttpState>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |mut socket| async move {
        let mut rx = state.ws_events.subscribe();
        for bootstrap in [
            encode_ws_event(
                "agent_state",
                serde_json::json!({ "agent": "system", "state": "idle" }),
            ),
            encode_ws_event(
                "fleet_progress",
                serde_json::json!({ "active_fleets": 0, "state": "idle" }),
            ),
            encode_ws_event(
                "ledger_health",
                serde_json::json!({ "status": "unknown" }),
            ),
            encode_ws_event(
                "review_completed",
                serde_json::json!({ "review_id": null, "verdict": null }),
            ),
        ] {
            if socket.send(Message::Text(bootstrap.into())).await.is_err() {
                return;
            }
        }

        loop {
            match rx.recv().await {
                Ok(event) => {
                    if socket.send(Message::Text(event.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

fn dashboard_asset_response(path: &str) -> Option<Response<Body>> {
    let normalized = path.trim_start_matches('/');
    let mut full_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../dashboard/dist");
    for component in std::path::Path::new(normalized).components() {
        match component {
            Component::Normal(part) => full_path.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    let asset = fs::read(full_path).ok()?;
    let mime = mime_guess::from_path(normalized).first_or_octet_stream();
    let headers = [(axum::http::header::CONTENT_TYPE, mime.as_ref())];
    Some((headers, asset).into_response())
}

pub async fn dashboard_root_route() -> Response {
    dashboard_asset_response("index.html")
        .unwrap_or_else(|| (StatusCode::NOT_FOUND, "dashboard index not found").into_response())
}

pub async fn dashboard_assets_route(Path(path): Path<String>) -> Response {
    let asset_path = format!("assets/{path}");
    dashboard_asset_response(&asset_path)
        .unwrap_or_else(|| (StatusCode::NOT_FOUND, "asset not found").into_response())
}

pub async fn dashboard_spa_fallback_route(Path(path): Path<String>) -> Response {
    dashboard_asset_response(&path).unwrap_or_else(|| {
        dashboard_asset_response("index.html")
            .unwrap_or_else(|| (StatusCode::NOT_FOUND, "dashboard index not found").into_response())
    })
}

pub async fn metrics_route(
    State(state): State<DaemonHttpState>,
) -> Result<String, (StatusCode, AxumJson<serde_json::Value>)> {
    state.metrics.snapshot_keepalive();
    let metric_families = state.metrics.registry.gather();
    let mut body = Vec::<u8>::new();
    TextEncoder::new().encode(&metric_families, &mut body).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    String::from_utf8(body).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })
}

async fn record_marker_parse_result(state: &DaemonHttpState, response: &str) {
    let parse_ok = match agent_adapter::parse_tool_call_marker(response) {
        Ok(_) => true,
        Err(err) => {
            warn!("tool marker parse failed: {err}");
            false
        }
    };

    let mut window = state.marker_parse_window.lock().await;
    let now = Instant::now();
    window.push_back((now, parse_ok));
    while let Some((ts, _)) = window.front() {
        if now.duration_since(*ts) > Duration::from_secs(3600) {
            let _ = window.pop_front();
        } else {
            break;
        }
    }
    let total = window.len();
    if total == 0 {
        return;
    }
    let successes = window.iter().filter(|(_, ok)| *ok).count();
    let rate = successes as f64 / total as f64;
    state.metrics.marker_parse_success_rate.set(rate);
    if total >= 10 && rate < 0.5 {
        warn!(
            marker_parse_success_rate = rate,
            sample_count = total,
            "marker parse success rate degraded below 50% over rolling 1h window"
        );
    }
}

#[derive(Debug, Deserialize)]
pub struct LedgerWakeRequest {
    pub project_root: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenSummaryQuery {
    pub since: Option<String>,
    pub until: Option<String>,
    pub agent: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TokenByBuildQuery {
    pub build_id: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenBySessionQuery {
    pub session_id: String,
}

pub async fn token_summary_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    Query(query): Query<TokenSummaryQuery>,
) -> Result<AxumJson<serde_json::Value>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }

    if let Some(agent) = query.agent.as_deref()
        && !matches!(agent, "claude" | "codex" | "gemini")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            AxumJson(serde_json::json!({ "error": "agent must be one of claude|codex|gemini" })),
        ));
    }

    let token_db = state.token_db.clone();
    let filters = SummaryQueryFilters {
        since: query.since,
        until: query.until,
        agent: query.agent,
    };

    let response = tokio::task::spawn_blocking(move || token_economics::summary_query(&token_db, &filters))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AxumJson(serde_json::json!({ "error": format!("join error: {e}") })),
            )
        })?
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                AxumJson(serde_json::json!({ "error": e.to_string() })),
            )
        })?;

    Ok(AxumJson(response))
}

pub async fn token_by_build_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    Query(query): Query<TokenByBuildQuery>,
) -> Result<AxumJson<BuildCostBreakdown>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }

    let build_id = query.build_id.trim().to_string();
    if build_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            AxumJson(serde_json::json!({ "error": "build_id is required" })),
        ));
    }

    let token_db = state.token_db.clone();
    let response = tokio::task::spawn_blocking(move || token_economics::by_build_query(&token_db, &build_id))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AxumJson(serde_json::json!({ "error": format!("join error: {e}") })),
            )
        })?
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                AxumJson(serde_json::json!({ "error": e.to_string() })),
            )
        })?;

    Ok(AxumJson(response))
}

pub async fn token_by_session_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    Query(query): Query<TokenBySessionQuery>,
) -> Result<AxumJson<SessionTokenBreakdown>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }

    let session_id = query.session_id.trim().to_string();
    if session_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            AxumJson(serde_json::json!({ "error": "session_id is required" })),
        ));
    }

    let token_db = state.token_db.clone();
    let response = tokio::task::spawn_blocking(move || token_economics::by_session_query(&token_db, &session_id))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AxumJson(serde_json::json!({ "error": format!("join error: {e}") })),
            )
        })?
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                AxumJson(serde_json::json!({ "error": e.to_string() })),
            )
        })?;

    Ok(AxumJson(response))
}

// This span is what the remote parent gets attached to. Without a span of its own,
// `Span::current()` here is disabled and `set_parent` silently does nothing — the traceparent
// would be extracted and thrown away, which is exactly the bug this comment exists to prevent.
// Everything the daemon then does (including the `ask_agent` span) nests underneath it.
#[tracing::instrument(skip_all, name = "daemon_ask_agent")]
pub async fn ask_agent_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<AskAgentRequest>,
) -> Result<AxumJson<AskAgentResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }
    // Adopt the MCP bridge's trace as our parent. Without this the daemon starts a brand-new trace
    // and one logical call shows up in PostHog as two unrelated ones — bridge spans over here,
    // ask_agent over there, with nothing tying them together.
    adopt_remote_trace_parent(&headers);
    // Serialize agent execution per project to keep ordering predictable for concurrent bridges.
    let queue =
        core_acquire_project_queue(&state.queues, core_project_queue_key(req.cwd.as_ref(), req.repo.as_ref()))
            .await;
    let _guard = queue.lock().await;
    let started = Instant::now();
    let result = (state.ask_agent_executor)(&req).await;
    state.metrics.agent_requests_total.inc();
    state.metrics.agent_duration_seconds.observe(started.elapsed().as_secs_f64());
    match result {
        Ok(response) => {
            record_marker_parse_result(&state, &response.response).await;
            publish_ws_event(
                &state,
                "agent_state",
                serde_json::json!({
                    "agent": response.agent,
                    "request_id": response.request_id,
                    "state": "done"
                }),
            );
            Ok(AxumJson(response))
        }
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            AxumJson(serde_json::json!({ "error": e })),
        )),
    }
}

pub async fn ledger_wake_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<LedgerWakeRequest>,
) -> Result<AxumJson<serde_json::Value>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }

    let project_root = PathBuf::from(&req.project_root);
    if !project_root.is_absolute() {
        return Err((
            StatusCode::BAD_REQUEST,
            AxumJson(serde_json::json!({ "error": "project_root must be absolute" })),
        ));
    }

    {
        let mut lru = state.ledger_project_lru.lock().await;
        if let Some(existing_index) = lru.iter().position(|p| p == &project_root) {
            lru.remove(existing_index);
        }
        lru.push_back(project_root.clone());
        while lru.len() > LEDGER_PROJECT_LRU_CAPACITY {
            lru.pop_front();
        }
    }

    let (drain_result, queue_lag_seconds) = tokio::task::spawn_blocking({
        let project_root = project_root.clone();
        move || -> anyhow::Result<(shared_types::DrainResult, f64)> {
            let store = LedgerStore::open(project_root.clone())?;
            let spool_dir = project_root.join(".triumvirate").join("spool");
            let drained = store.drain_spool(&spool_dir)?;
            let lag = store.queue_lag_seconds()?;
            Ok((drained, lag))
        }
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": format!("join error: {e}") })),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    state
        .metrics
        .ledger_events_ingested_total
        .inc_by(drain_result.ingested_count as u64);
    state.metrics.ledger_queue_lag_seconds.set(queue_lag_seconds);

    Ok(AxumJson(serde_json::json!({
        "status": "ok",
        "project_root": req.project_root,
        "drain_result": drain_result
    })))
}

pub async fn ledger_health_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
) -> Result<AxumJson<HealthStatus>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }

    let project_root = {
        let lru = state.ledger_project_lru.lock().await;
        lru.back().cloned()
    }
    .or_else(|| std::env::current_dir().ok())
    .ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": "unable to resolve project root" })),
        )
    })?;

    let (health, queue_lag_seconds) = tokio::task::spawn_blocking(move || -> anyhow::Result<(HealthStatus, f64)> {
        let store = LedgerStore::open(project_root)?;
        let health = store.health()?;
        let lag = store.queue_lag_seconds()?;
        Ok((health, lag))
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": format!("join error: {e}") })),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    state.metrics.ledger_queue_lag_seconds.set(queue_lag_seconds);
    if let Ok(payload) = serde_json::to_value(&health) {
        publish_ws_event(&state, "ledger_health", payload);
    }

    Ok(AxumJson(health))
}

pub async fn ledger_query_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<LedgerQueryRequest>,
) -> Result<AxumJson<LedgerQueryResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }

    let project_root = {
        let lru = state.ledger_project_lru.lock().await;
        lru.back().cloned()
    }
    .or_else(|| std::env::current_dir().ok())
    .ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": "unable to resolve project root" })),
        )
    })?;

    let summaries = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Summary>> {
        let store = LedgerStore::open(project_root)?;
        store.query(&req.query, req.limit.unwrap_or(10))
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": format!("join error: {e}") })),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    Ok(AxumJson(LedgerQueryResponse { summaries }))
}

pub async fn ledger_session_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<LedgerSessionRequest>,
) -> Result<AxumJson<SessionDetail>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }

    let project_root = {
        let lru = state.ledger_project_lru.lock().await;
        lru.back().cloned()
    }
    .or_else(|| std::env::current_dir().ok())
    .ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": "unable to resolve project root" })),
        )
    })?;

    let session = tokio::task::spawn_blocking(move || -> anyhow::Result<SessionDetail> {
        let store = LedgerStore::open(project_root)?;
        store.get_session(&req.session_id)
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": format!("join error: {e}") })),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    Ok(AxumJson(session))
}

pub async fn ledger_record_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<ManualRecord>,
) -> Result<AxumJson<serde_json::Value>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }

    let project_root = {
        let lru = state.ledger_project_lru.lock().await;
        lru.back().cloned()
    }
    .or_else(|| std::env::current_dir().ok())
    .ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": "unable to resolve project root" })),
        )
    })?;

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let store = LedgerStore::open(project_root)?;
        store.record(req)
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": format!("join error: {e}") })),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    Ok(AxumJson(serde_json::json!({ "status": "ok" })))
}

pub async fn ledger_gc_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
) -> Result<AxumJson<GcResult>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }

    let project_root = {
        let lru = state.ledger_project_lru.lock().await;
        lru.back().cloned()
    }
    .or_else(|| std::env::current_dir().ok())
    .ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": "unable to resolve project root" })),
        )
    })?;

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<GcResult> {
        let store = LedgerStore::open(project_root)?;
        store.gc()
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": format!("join error: {e}") })),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    Ok(AxumJson(result))
}

pub async fn lesson_add_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<NewLesson>,
) -> Result<AxumJson<LessonAddResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }
    let project_root = {
        let lru = state.ledger_project_lru.lock().await;
        lru.back().cloned()
    }
    .or_else(|| std::env::current_dir().ok())
    .ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": "unable to resolve project root" })),
        )
    })?;

    let lesson_id = tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
        let store = LedgerStore::open(project_root)?;
        store.add_lesson(req)
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": format!("join error: {e}") })),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    Ok(AxumJson(LessonAddResponse { lesson_id }))
}

pub async fn lesson_query_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<LessonQueryRequest>,
) -> Result<AxumJson<LessonQueryResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }
    let project_root = {
        let lru = state.ledger_project_lru.lock().await;
        lru.back().cloned()
    }
    .or_else(|| std::env::current_dir().ok())
    .ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": "unable to resolve project root" })),
        )
    })?;

    let lessons = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Lesson>> {
        let store = LedgerStore::open(project_root)?;
        store.query_lessons(&req.query, req.min_confidence.unwrap_or(0.0))
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": format!("join error: {e}") })),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    Ok(AxumJson(LessonQueryResponse { lessons }))
}

pub async fn lesson_validate_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<LessonValidateRequest>,
) -> Result<AxumJson<serde_json::Value>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }
    let project_root = {
        let lru = state.ledger_project_lru.lock().await;
        lru.back().cloned()
    }
    .or_else(|| std::env::current_dir().ok())
    .ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": "unable to resolve project root" })),
        )
    })?;

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let store = LedgerStore::open(project_root)?;
        store.validate_lesson(req.lesson_id)
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": format!("join error: {e}") })),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    Ok(AxumJson(serde_json::json!({ "status": "ok" })))
}

pub async fn lesson_list_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<LessonListRequest>,
) -> Result<AxumJson<LessonListResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }
    let project_root = {
        let lru = state.ledger_project_lru.lock().await;
        lru.back().cloned()
    }
    .or_else(|| std::env::current_dir().ok())
    .ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": "unable to resolve project root" })),
        )
    })?;

    let lessons = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Lesson>> {
        let store = LedgerStore::open(project_root)?;
        store.list_lessons(req.tags.as_deref(), req.stale_days)
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": format!("join error: {e}") })),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    Ok(AxumJson(LessonListResponse { lessons }))
}

pub async fn memory_write_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<MemoryWriteRequest>,
) -> Result<AxumJson<MemoryWriteResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }
    let home = core_triumvirate_home_dir().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    let id = Uuid::new_v4().to_string();
    let entry = MemoryEntry {
        id: id.clone(),
        namespace: req.namespace,
        key: req.key,
        value: req.value,
        ts_ms: core_unix_time_ms(),
    };
    core_append_memory_entry(&home, &entry).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    Ok(AxumJson(MemoryWriteResponse {
        id,
        status: "ok".to_string(),
    }))
}

pub async fn memory_read_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<MemoryReadRequest>,
) -> Result<AxumJson<MemoryReadResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }
    let home = core_triumvirate_home_dir().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    let mut entries = core_read_memory_entries(&home).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    entries.retain(|e| e.namespace == req.namespace);
    if let Some(key) = req.key {
        entries.retain(|e| e.key == key);
    }
    entries.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms));
    if let Some(limit) = req.limit {
        entries.truncate(limit);
    }
    Ok(AxumJson(MemoryReadResponse { entries }))
}

pub async fn scratchpad_write_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<ScratchpadWriteRequest>,
) -> Result<AxumJson<ScratchpadWriteResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }
    let home = core_triumvirate_home_dir().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    let path = core_write_scratchpad(
        &home,
        &req.project,
        &req.topic,
        &req.content,
        core_unix_time_ms(),
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    Ok(AxumJson(ScratchpadWriteResponse {
        path: path.display().to_string(),
    }))
}

pub async fn scratchpad_list_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<ScratchpadListRequest>,
) -> Result<AxumJson<ScratchpadListResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }
    let home = core_triumvirate_home_dir().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    let files = core_list_scratchpad(&home, &req.project)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AxumJson(serde_json::json!({ "error": e.to_string() })),
            )
        })?
        .into_iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>();
    Ok(AxumJson(ScratchpadListResponse { files }))
}

pub async fn outbox_recent_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<OutboxRecentRequest>,
) -> Result<AxumJson<OutboxRecentResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }
    let mut events = read_outbox_events().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    events.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms));
    events.truncate(req.limit.unwrap_or(50));
    Ok(AxumJson(OutboxRecentResponse { events }))
}

pub async fn fallback_list_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<FallbackListRequest>,
) -> Result<AxumJson<FallbackListResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }
    let tickets = list_pending_fallback_paths(req.limit.unwrap_or(20))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AxumJson(serde_json::json!({ "error": e.to_string() })),
            )
        })?
        .into_iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>();
    Ok(AxumJson(FallbackListResponse { tickets }))
}

pub async fn fallback_ack_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<FallbackAckRequest>,
) -> Result<AxumJson<serde_json::Value>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }
    acknowledge_fallback_path(&req.path).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    Ok(AxumJson(serde_json::json!({
        "status": "ok",
        "message": format!("acknowledged {}", req.path)
    })))
}

pub async fn fallback_gc_route(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    AxumJson(req): AxumJson<FallbackGcRequest>,
) -> Result<AxumJson<FallbackGcResponse>, (StatusCode, AxumJson<serde_json::Value>)> {
    if !is_bearer_authorized(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()), &state.token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            AxumJson(serde_json::json!({ "error": "unauthorized" })),
        ));
    }
    let removed = gc_fallbacks(req.max_age_days.unwrap_or(7)).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AxumJson(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    Ok(AxumJson(FallbackGcResponse { removed }))
}
