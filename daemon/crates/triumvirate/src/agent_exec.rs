use crate::{
    TokenRecord, append_outbox_event, process_metrics, process_token_db, record_daemon_tokens,
    spawn_dead_drop,
};
use agent_adapter::{
    ApprovalChannelMode, CodexAppServerEvent, CodexAppServerParser, CodexExecParser,
    GrokStreamParser, GrokTermination,
    GeminiStreamParser, ParsedAgentResult, StuckDetector, WorkingState, WorkingStateEvent,
    format_working_state, probe_approval_response_channel, should_display,
};
use agent_worker::{
    WorkerAcquireMode, acquire_worker, dismiss_worker, should_invalidate_cached_session,
    update_worker_session,
};
use daemon_core::{resolve_context as core_resolve_context, unix_time_ms as core_unix_time_ms};
use ledger::LedgerStore;
use mcp_bridge::{
    GeminiBackend, agent_verbosity, agy_command, claude_command, codex_command, codex_protocol,
    gemini_backend, gemini_command, gemini_shadow_enabled, gemini_streaming_enabled,
    is_supported_agent, normalize_agent_name,
};
use mcp_tools::{ProgressEmitter, display_agent_name, next_heartbeat_offset};
use peer_review::{PeerReviewEngine, ReviewRequest};
use serde::Deserialize;
use shared_types::{
    AskAgentRequest, AskAgentResponse, LifecycleEvent, ManualRecord, OutboxEvent,
    TokenUsage as SharedTokenUsage,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc,
    time::{Instant, sleep, timeout},
};
use tracing::{Span, instrument};
use uuid::Uuid;

/// Gemini model faildown chain — ordered by empirical reliability (2026-04-15).
/// First model is the CLI default (settings.json). Subsequent models are tried
/// on 429/capacity errors. stderr is monitored for fast-kill on 429 detection.
///
/// Reliability from 10-run probe:
///   gemini-2.5-flash        90%  5.9s avg  (default)
///   gemini-3-flash-preview  80%  6.6s avg
///   gemini-2.5-pro          10%  (capacity-exhausted)
///   gemini-3.1-pro-preview   0%  (capacity-exhausted)
const GEMINI_MODEL_FAILDOWN: &[&str] = &[
    "gemini-2.5-flash",
    "gemini-3-flash-preview",
    "gemini-2.5-pro",
    "gemini-3.1-pro-preview",
];

const TOOL_MARKER_INSTRUCTIONS: &str = "\
When you need to call a Triumvirate tool, emit exactly one XML block with this shape:
<triumvirate_tool name=\"ledger_record\">{\"title\":\"...\",\"narrative\":\"...\"}</triumvirate_tool>
Rules:
- Use name values only from supported tools: ledger_query, ledger_session, ledger_record, lesson_add, lesson_query, lesson_validate, lesson_list.
- The body must be valid JSON object parameters for that tool.
- Emit normal prose outside the XML block when needed.";

#[cfg(unix)]
pub(crate) fn configure_process_group(command: &mut Command) {
    // SAFETY: pre_exec runs in the spawned child process before exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
pub(crate) fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
pub(crate) fn kill_process_group(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        let pgid = -(pid as i32);
        // SAFETY: kill is called with a process-group id derived from child pid.
        unsafe {
            let _ = libc::kill(pgid, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn kill_process_group(_child: &mut tokio::process::Child) {}

pub(crate) fn emit_working_event(tx: Option<&mpsc::Sender<WorkingStateEvent>>, event: WorkingStateEvent) {
    if let Some(sender) = tx {
        let _ = sender.try_send(event);
    }
}

#[derive(Debug, Deserialize)]
struct BuildStateContext {
    build_id: String,
}

#[derive(Debug, Deserialize)]
struct ContractContext {
    task_id: String,
    wave: u32,
}

fn cast_u64_to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn cast_usize_to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn read_build_id(cwd: Option<&str>) -> Option<String> {
    let cwd = cwd?;
    let path = Path::new(cwd).join("BUILD_STATE.json");
    let raw = fs::read_to_string(path).ok()?;
    let parsed: BuildStateContext = serde_json::from_str(&raw).ok()?;
    Some(parsed.build_id)
}

fn read_contract_context(cwd: Option<&str>) -> (Option<String>, Option<i64>) {
    let Some(cwd) = cwd else {
        return (None, None);
    };
    let path = Path::new(cwd).join(".triumvirate").join("contract.json");
    let Ok(raw) = fs::read_to_string(path) else {
        return (None, None);
    };
    let Ok(parsed) = serde_json::from_str::<ContractContext>(&raw) else {
        return (None, None);
    };
    (Some(parsed.task_id), Some(i64::from(parsed.wave)))
}

fn persist_daemon_token_record(
    agent: &str,
    request_id: &str,
    parsed: &ParsedAgentResult,
    resolved_cwd: &Option<String>,
    resolved_repo: &Option<String>,
) {
    let Some(token_db) = process_token_db() else {
        return;
    };

    let usage = parsed.token_usage.as_ref();
    let input = usage.and_then(|u| u.input).unwrap_or(0);
    let output = usage.and_then(|u| u.output).unwrap_or(0);
    let cached = usage.and_then(|u| u.cached).unwrap_or(0);
    let total = usage
        .and_then(|u| u.total)
        .unwrap_or_else(|| input.saturating_add(output).saturating_add(cached));
    let session_id = parsed
        .session_id
        .clone()
        .unwrap_or_else(|| request_id.to_string());
    let (task_id, wave) = read_contract_context(resolved_cwd.as_deref());
    let build_id = read_build_id(resolved_cwd.as_deref()).or_else(|| resolved_repo.clone());

    // REQ-057: agy has no honest headless token count → record the dispatch as
    // `unmetered` (excluded from cost sums) rather than a fake zero. Other backends
    // carry exact counts.
    let usage_source = if parsed.parser_mode.starts_with("agy") {
        token_economics::USAGE_SOURCE_UNMETERED
    } else {
        token_economics::USAGE_SOURCE_EXACT
    }
    .to_string();

    let record = TokenRecord {
        agent: agent.to_string(),
        session_id,
        timestamp: chrono::Utc::now().to_rfc3339(),
        model: parsed.cli_version.clone(),
        input_tokens: cast_u64_to_i64(input),
        output_tokens: cast_u64_to_i64(output),
        cached_tokens: cast_u64_to_i64(cached),
        thinking_tokens: 0,
        total_tokens: cast_u64_to_i64(total),
        cost_usd: None,
        latency_ms: None,
        tool_calls: Some(cast_usize_to_i64(parsed.tool_calls.len())),
        lines_added: None,
        lines_removed: None,
        rate_limit_pct: None,
        context_window: None,
        build_id,
        task_id,
        wave,
        usage_source,
    };

    if let Err(err) = record_daemon_tokens(token_db.as_ref(), &record) {
        tracing::warn!("failed to record daemon token usage: {err}");
    }
}

/// T-015 (REQ-DS-025): byte-size ceiling for the DeepSeek ask_agent path.
/// Env-configurable via TRIUMVIRATE_DEEPSEEK_BULK_BYTES; default 16384 (16KB).
/// Invalid env values fall back to the default (the DeepSeekConfig loader is
/// the authoritative validator — this helper exists so the cap can be read
/// WITHOUT triggering full config load + OnceLock init for every ask_agent).
fn deepseek_bulk_bytes_cap() -> usize {
    const DEFAULT: usize = 16_384;
    std::env::var("TRIUMVIRATE_DEEPSEEK_BULK_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT)
}

/// T-013 (REQ-DS-026): persist a token record on the DeepSeek Err path so we
/// don't lose billable tokens when the call fails mid-stream (e.g. 402 hard,
/// 429 transient, mid-stream disconnect). Gated to the DeepSeek typed-failure
/// surface — Gemini/Codex Err paths are EXPLICITLY unchanged. The
/// usage_source mirrors what the runner produced (`exact` if the usage chunk
/// arrived before the failure, `estimated` if we fell back to bytes/4).
pub(crate) fn persist_deepseek_err_tokens(
    request_id: &str,
    fallback_session_id: &str,
    usage: &mcp_bridge::deepseek::TokenUsage,
    ds_request_id: Option<&str>,
    resolved_cwd: &Option<String>,
    resolved_repo: &Option<String>,
) {
    let Some(token_db) = process_token_db() else {
        return;
    };
    let (task_id, wave) = read_contract_context(resolved_cwd.as_deref());
    let build_id = read_build_id(resolved_cwd.as_deref()).or_else(|| resolved_repo.clone());

    let usage_source = match usage.usage_source {
        mcp_bridge::deepseek::UsageSource::Exact => token_economics::USAGE_SOURCE_EXACT,
        mcp_bridge::deepseek::UsageSource::Estimated => token_economics::USAGE_SOURCE_ESTIMATED,
    }
    .to_string();

    // Prefer the deepseek-provided id when present so cross-referencing the
    // per-request log file is straightforward; otherwise fall back to the
    // synthetic session_id (or the daemon request_id if even that's absent).
    let session_id = ds_request_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| fallback_session_id.to_string());

    let total = usage.input_tokens + usage.output_tokens + usage.cached_tokens;
    let record = TokenRecord {
        agent: "deepseek".to_string(),
        session_id,
        timestamp: chrono::Utc::now().to_rfc3339(),
        model: None, // T-013 scope_out: model is in the per-request log; here
                     //   we keep the Err record narrow.
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_tokens: usage.cached_tokens,
        thinking_tokens: 0,
        total_tokens: total,
        cost_usd: None,
        latency_ms: None,
        tool_calls: Some(0),
        lines_added: None,
        lines_removed: None,
        rate_limit_pct: None,
        context_window: None,
        build_id,
        task_id,
        wave,
        usage_source,
    };
    if let Err(err) = record_daemon_tokens(token_db.as_ref(), &record) {
        tracing::warn!(
            request_id,
            err = %err,
            "T-013 deepseek Err-path token persist failed"
        );
    }
}

#[instrument(
    name = "ask_agent",
    skip(req, progress),
    fields(
        agent = %req.agent,
        session_id = tracing::field::Empty,
        request_type = "ask_agent",
        agent.tokens = tracing::field::Empty,
        agent.outcome = tracing::field::Empty,
        agent.duration_ms = tracing::field::Empty,
        // Declared, or span.record() silently no-ops and the drift stays invisible in the
        // one place an operator would look for it.
        agent.backend = tracing::field::Empty,
        agent.model = tracing::field::Empty
    )
)]
pub(crate) async fn execute_ask_agent(
    req: &AskAgentRequest,
    progress: Option<ProgressEmitter>,
) -> Result<AskAgentResponse, String> {
    #[allow(clippy::cast_possible_truncation)]
    fn token_total(parsed: &ParsedAgentResult) -> u64 {
        if let Some(usage) = parsed.token_usage.as_ref() {
            if let Some(total) = usage.total {
                return total;
            }
            return usage.input.unwrap_or(0) + usage.output.unwrap_or(0) + usage.cached.unwrap_or(0);
        }
        0
    }

    fn map_token_usage(token_usage: Option<&agent_adapter::TokenUsage>) -> Option<SharedTokenUsage> {
        token_usage.map(|usage| SharedTokenUsage {
            input: usage.input,
            output: usage.output,
            cached: usage.cached,
            thinking_tokens: usage.thinking_tokens,
            latency_ms: usage.latency_ms,
            tool_calls: usage.tool_calls,
            total: usage.total,
        })
    }

    let started = Instant::now();
    let span = Span::current();

    // request_id is minted here, ABOVE the early returns, so the telemetry guard below can
    // cover them. Everything after this point reports exactly once, on drop, whichever exit
    // is taken — including exits that do not exist yet.
    let request_id = Uuid::new_v4().to_string();
    // The model matters for pricing, and DeepSeek is the only PER-TOKEN metered sibling — codex and
    // gemini run on subscriptions, where one more call costs exactly $0 in DOLLARS.
    //
    // That is not the same as free. Grok runs on a flat SuperGrok plan and burns 14K to 67K input
    // tokens per consult regardless of how small the question is, because every turn re-ships the
    // system prompt and every tool schema. The quota is finite and consumed invisibly, so grok's
    // self-reported `total_cost_usd` is captured as a USAGE signal, not as a bill.
    let mut tel = mcp_bridge::posthog::CallTelemetry::new(
        &req.agent,
        &request_id,
        req.deepseek_model.as_deref(),
    );

    if !is_supported_agent(req) {
        span.record("agent.outcome", "rejected");
        span.record("agent.tokens", 0_u64);
        span.record("agent.duration_ms", started.elapsed().as_millis() as u64);
        let msg = "ask_agent supports only agent='antigravity' (aliases: agy, gemini), agent='codex', or agent='deepseek'";
        tel.failure(msg);
        return Err(msg.to_string());
    }
    // Normalize BEFORE worker-acquire and dispatch so antigravity/agy callers land
    // on the canonical `gemini` execution key (shared worker slot + dispatch arm).
    let agent = normalize_agent_name(&req.agent);
    tel.set_agent(&agent);
    // $ai_input: the actual prompt for this call, so PostHog's LLM trace view shows what we sent.
    tel.set_input(&req.message);

    // T-015 (REQ-DS-025) anti-bulk: reject oversized payloads on the metered
    // DeepSeek path BEFORE any worker is acquired. The default ceiling (16KB)
    // comes from TRIUMVIRATE_DEEPSEEK_BULK_BYTES (env-configurable). Gemini
    // and Codex are local CLIs — bulk is free — so the check is GATED to
    // agent=='deepseek'. The error message names both surfaces ("payload too
    // large" + "metered") so callers see the cost vector explicitly.
    if agent == "deepseek" {
        let cap = deepseek_bulk_bytes_cap();
        if req.message.len() > cap {
            span.record("agent.outcome", "rejected_payload_too_large");
            span.record("agent.tokens", 0_u64);
            span.record("agent.duration_ms", started.elapsed().as_millis() as u64);
            let msg = format!(
                "deepseek: payload too large ({} bytes > {} byte cap) — DeepSeek is remote+metered, \
                 set TRIUMVIRATE_DEEPSEEK_BULK_BYTES to raise the limit",
                req.message.len(),
                cap
            );
            tel.failure(msg.clone());
            return Err(msg);
        }
    }

    // Every synchronous validation gate (unsupported agent, oversized DeepSeek payload) is now
    // behind us and classifies itself before returning. From here on we begin the real dispatch:
    // acquire a worker, talk to the provider, run peer review. Arm the telemetry so that if this
    // future is CANCELLED mid-await — the caller's client-side `ask_agent` timeout fires at 180s,
    // or the client disconnects — the guard emits `tv_outcome = "cancelled"` instead of the
    // `unreported` sentinel. The metered DeepSeek path is the one that routinely runs long enough
    // (thinking mode, absolute SLA of 1800s) to be killed by the 180s ceiling before any
    // classify() arm runs, which is exactly how three terminal errors went missing from
    // outcome-based monitoring.
    tel.begin_dispatch();

    // REQ-001: resolve the gemini backend once, up front — it drives both the attempt
    // schedule (agy is single-attempt, REQ-013) and the degraded route (REQ-053).
    let gemini_backend_selected = if agent == "gemini" {
        Some(gemini_backend())
    } else {
        None
    };
    // Surface the RESOLVED backend on the call's telemetry and in the log. The backend is
    // read from this process's env, so a daemon started without TRIUMVIRATE_GEMINI_BACKEND
    // silently serves gemini-cli while the caller's config says agy — invisible until now,
    // because both backends report agent="gemini".
    if let Some(backend) = gemini_backend_selected {
        let label = match backend {
            GeminiBackend::Agy => "agy",
            GeminiBackend::GeminiCli => "gemini-cli",
        };
        tel.set_backend(label);
        span.record("agent.backend", label);
        // gemini-cli is RETIRED and no longer works. Selecting it is always a config
        // defect, and it is reachable only by accident: gemini_backend() defaults to it
        // whenever TRIUMVIRATE_GEMINI_BACKEND is unset in THIS process. A daemon started
        // without the MCP env block therefore serves a dead backend, burns its whole
        // 4-model faildown chain per request, and leaves the agy limiter+breaker inert
        // (every gate is `== Agy`). That ran for four days in silence. Say it out loud.
        if matches!(backend, GeminiBackend::GeminiCli) {
            tracing::warn!(
                backend = label,
                "DEAD BACKEND SELECTED: gemini-cli is retired and does not work. This process \
                 has no TRIUMVIRATE_GEMINI_BACKEND=agy in its env. Start the daemon via \
                 scripts/start-daemon.sh so it inherits the MCP env block."
            );
        }
    }
    let (resolved_cwd, resolved_repo, resolved_branch) =
        core_resolve_context(req.cwd.as_ref(), req.repo.as_ref(), req.branch.as_ref());
    // Slice cost and quota by project. set_repo keeps the NAME only: resolved_repo is an
    // absolute toplevel path, which would be both cardinality garbage and a home-directory
    // leak into a SaaS.
    if let Some(repo) = resolved_repo.as_deref() {
        tel.set_repo(repo);
    }
    let exec_cwd = resolved_cwd
        .clone()
        .unwrap_or_else(|| ".".to_string());
    let execution_prompt = inject_tool_marker_prompt(&req.message);
    // Named sessions get their own worker record; one-shot ask_agent keeps the shared one.
    let session_key = req.session_key.as_deref();
    let worker = acquire_worker(&agent, &exec_cwd, session_key).await;
    // Resume is opt-in. The worker registry is keyed only by (agent, cwd), so a one-shot
    // ask_agent would otherwise resume — and get billed for — whatever named session last
    // ran in this directory, replaying its whole transcript as input on every call.
    let reuse_session = req.reuse_session.unwrap_or(false);
    let mut worker_session_id = if reuse_session {
        worker.session_id.clone()
    } else {
        None
    };
    span.record(
        "session_id",
        tracing::field::display(worker_session_id.as_deref().unwrap_or("none")),
    );
    let worker_mode = worker.mode.clone();
    let worker_mode_state = "SPAWNED";

    let agent_display = display_agent_name(&agent);
    let mut lifecycle = vec![LifecycleEvent {
        state: worker_mode_state.to_string(),
        detail: format!(
            "{} {} worker{}{}{} (spawn_count={})",
            if worker_mode == WorkerAcquireMode::Spawned {
                "Started"
            } else {
                "Reused"
            },
            agent_display,
            req.cwd
                .as_ref()
                .map(|v| format!(" cwd={v}"))
                .unwrap_or_default(),
            req.repo
                .as_ref()
                .map(|v| format!(" repo={v}"))
                .unwrap_or_default(),
            req.branch
                .as_ref()
                .map(|v| format!(" branch={v}"))
                .unwrap_or_default(),
            worker.spawn_count
        ),
    }];
    if let Err(e) = append_outbox_event(&OutboxEvent {
        ts_ms: core_unix_time_ms(),
        request_id: request_id.clone(),
        tool: "ask_agent".to_string(),
        status: "SPAWNED".to_string(),
        agent: Some(agent.clone()),
        detail: lifecycle
            .last()
            .map(|e| e.detail.clone())
            .unwrap_or_default(),
        cwd: resolved_cwd.clone(),
        repo: resolved_repo.clone(),
        branch: resolved_branch.clone(),
        working_state: Some("SPAWNED".to_string()),
        token_usage: None,
        tool_name: None,
    }) {
        tracing::warn!("failed to append outbox event: {e}");
    }
    if let Some(emitter) = progress.as_ref() {
        emitter.emit(format!("→ {agent_display}: sent ✓")).await;
    }

    lifecycle.push(LifecycleEvent {
        state: "WORKING".to_string(),
        detail: format!("{agent_display} is processing request"),
    });
    if let Err(e) = append_outbox_event(&OutboxEvent {
        ts_ms: core_unix_time_ms(),
        request_id: request_id.clone(),
        tool: "ask_agent".to_string(),
        status: "WORKING".to_string(),
        agent: Some(agent.clone()),
        detail: lifecycle
            .last()
            .map(|e| e.detail.clone())
            .unwrap_or_default(),
        cwd: resolved_cwd.clone(),
        repo: resolved_repo.clone(),
        branch: resolved_branch.clone(),
        working_state: Some("WORKING".to_string()),
        token_usage: None,
        tool_name: None,
    }) {
        tracing::warn!("failed to append outbox event: {e}");
    }
    if let Some(emitter) = progress.as_ref() {
        emitter.emit(format!("→ {agent_display}: working...")).await;
    }

    // Build the attempt schedule: for gemini-cli, use the model faildown chain; for
    // the agy backend, a single attempt with no --model (REQ-013 — agy ignores
    // --model and runs its own internal retry); for deepseek, a single attempt
    // (REQ-DS-008 / T-013 — the runner owns its scoped in-flight retries, the
    // outer execute loop must NOT retry); for others, 3 retries.
    let attempt_schedule = attempt_schedule_for(&agent, gemini_backend_selected);
    let verbosity = agent_verbosity();
    let mut last_err: Option<String> = None;

    // Slice 6 (shadow-compare): the OTHER Gemini backend to run alongside the primary
    // for comparison when TRIUMVIRATE_GEMINI_SHADOW is on. None disables shadowing.
    let shadow_backend = if agent == "gemini" && gemini_shadow_enabled() {
        gemini_backend_selected.map(GeminiBackend::shadow_counterpart)
    } else {
        None
    };

    // REQ-101/103: if the agy circuit breaker is OPEN, skip the agy attempt and route
    // straight around it. The breaker opens on repeated quota, so the reason is set
    // quota-class → the degraded route skips gemini-cli (shared pool) and uses codex.
    let agy_breaker_open = matches!(gemini_backend_selected, Some(GeminiBackend::Agy))
        && mcp_bridge::agy_resilience::agy_breaker_should_skip();
    if agy_breaker_open {
        let detail = "agy circuit breaker open (repeated quota) — skipping agy, routing around";
        lifecycle.push(LifecycleEvent {
            state: "BREAKER_OPEN".to_string(),
            detail: detail.to_string(),
        });
        let _ = append_outbox_event(&OutboxEvent {
            ts_ms: core_unix_time_ms(),
            request_id: request_id.clone(),
            tool: "ask_agent".to_string(),
            status: "BREAKER_OPEN".to_string(),
            agent: Some(agent.clone()),
            detail: detail.to_string(),
            cwd: resolved_cwd.clone(),
            repo: resolved_repo.clone(),
            branch: resolved_branch.clone(),
            working_state: Some("BREAKER_OPEN".to_string()),
            token_usage: None,
            tool_name: None,
        });
        last_err = Some("agy capacity/quota: circuit breaker open".to_string());
    }

    for (idx, (backoff, model_override)) in attempt_schedule.iter().enumerate() {
        if agy_breaker_open {
            break;
        }
        // Count every attempt we are about to spend against the provider, not the size of
        // the schedule: success on the first try must report 1.
        tel.record_attempt();
        if let Some(model) = model_override {
            tracing::info!("faildown attempt {}/{}: trying model {model}", idx + 1, attempt_schedule.len());
            if let Some(emitter) = progress.as_ref() {
                emitter
                    .emit(format!("→ {agent_display}: trying model {model} ({}/{})...", idx + 1, attempt_schedule.len()))
                    .await;
            }
        }
        // A faildown attempt runs on a DIFFERENT model, which cannot resume the primary
        // model's session — so it starts fresh. Crucially, its fresh session id must also
        // never be written back (see the success arm): doing so would overwrite the primary
        // session cached for this (agent, cwd) and permanently orphan the user's transcript,
        // turning a transient 429 into total memory loss for a named session.
        let is_faildown_attempt = model_override.is_some() && idx > 0;
        let session_for_attempt = if is_faildown_attempt {
            None
        } else {
            worker_session_id.clone()
        };
        let (events_tx, mut events_rx) = mpsc::channel::<WorkingStateEvent>(1024);
        let mut stuck_detector = StuckDetector::default();
        let mut attempt = Box::pin(run_named_agent_with_session_and_model(
            &agent,
            &execution_prompt,
            &exec_cwd,
            session_for_attempt.as_deref(),
            Some(events_tx),
            *model_override,
            Some(req),
        ));
        let started = Instant::now();
        let mut next_heartbeat = Duration::from_secs(30);

        let attempt_result = loop {
            let sleep_duration = next_heartbeat.saturating_sub(started.elapsed());
            tokio::select! {
                result = &mut attempt => break result,
                Some(event) = events_rx.recv() => {
                    let outbox_detail = format_working_state(&event);
                    if let Err(e) = append_outbox_event(&OutboxEvent {
                        ts_ms: core_unix_time_ms(),
                        request_id: request_id.clone(),
                        tool: "ask_agent".to_string(),
                        status: "WORKING_EVENT".to_string(),
                        agent: Some(agent.clone()),
                        detail: outbox_detail.clone(),
                        cwd: resolved_cwd.clone(),
                        repo: resolved_repo.clone(),
                        branch: resolved_branch.clone(),
                        working_state: Some(format!("{:?}", event.state)),
                        token_usage: map_token_usage(event.token_usage.as_ref()),
                        tool_name: event.tool_name.clone(),
                    }) {
                        tracing::warn!("failed to append outbox event: {e}");
                    }
                    if should_display(&event.state, verbosity)
                        && let Some(emitter) = progress.as_ref()
                    {
                        emitter.emit(outbox_detail).await;
                    }
                    if let Some(reason) = stuck_detector.observe(&event) {
                        let stuck_event = WorkingStateEvent {
                            agent: agent.clone(),
                            state: WorkingState::Stuck,
                            detail: format!("detected potential stuck state: {:?}", reason),
                            tool_name: None,
                            tool_args_json: None,
                            token_usage: None,
                            ts_ms: Some(core_unix_time_ms()),
                        };
                        lifecycle.push(LifecycleEvent {
                            state: "STUCK".to_string(),
                            detail: stuck_event.detail.clone(),
                        });
                        let stuck_detail = format_working_state(&stuck_event);
                        if let Err(e) = append_outbox_event(&OutboxEvent {
                            ts_ms: core_unix_time_ms(),
                            request_id: request_id.clone(),
                            tool: "ask_agent".to_string(),
                            status: "WORKING_EVENT".to_string(),
                            agent: Some(agent.clone()),
                            detail: stuck_detail.clone(),
                            cwd: resolved_cwd.clone(),
                            repo: resolved_repo.clone(),
                            branch: resolved_branch.clone(),
                            working_state: Some("STUCK".to_string()),
                            token_usage: None,
                            tool_name: None,
                        }) {
                            tracing::warn!("failed to append outbox event: {e}");
                        }
                        if should_display(&stuck_event.state, verbosity)
                            && let Some(emitter) = progress.as_ref()
                        {
                            emitter.emit(stuck_detail).await;
                        }
                    }
                    next_heartbeat = started.elapsed() + Duration::from_secs(30);
                }
                _ = sleep(sleep_duration) => {
                    if started.elapsed() >= next_heartbeat {
                        if let Some(reason) = stuck_detector.check_timeouts() {
                            let stuck_event = WorkingStateEvent {
                                agent: agent.clone(),
                                state: WorkingState::Stuck,
                                detail: format!("detected potential stuck timeout: {:?}", reason),
                                tool_name: None,
                                tool_args_json: None,
                                token_usage: None,
                                ts_ms: Some(core_unix_time_ms()),
                            };
                            lifecycle.push(LifecycleEvent {
                                state: "STUCK".to_string(),
                                detail: stuck_event.detail.clone(),
                            });
                            let stuck_detail = format_working_state(&stuck_event);
                            if let Err(e) = append_outbox_event(&OutboxEvent {
                                ts_ms: core_unix_time_ms(),
                                request_id: request_id.clone(),
                                tool: "ask_agent".to_string(),
                                status: "WORKING_EVENT".to_string(),
                                agent: Some(agent.clone()),
                                detail: stuck_detail.clone(),
                                cwd: resolved_cwd.clone(),
                                repo: resolved_repo.clone(),
                                branch: resolved_branch.clone(),
                                working_state: Some("STUCK".to_string()),
                                token_usage: None,
                                tool_name: None,
                            }) {
                                tracing::warn!("failed to append outbox event: {e}");
                            }
                            if should_display(&stuck_event.state, verbosity)
                                && let Some(emitter) = progress.as_ref()
                            {
                                emitter.emit(stuck_detail).await;
                            }
                        }
                        let elapsed = started.elapsed().as_secs();
                        let detail = format!("{agent_display} still working ({elapsed}s elapsed)");
                        lifecycle.push(LifecycleEvent {
                            state: "WORKING".to_string(),
                            detail,
                        });
                        if let Some(emitter) = progress.as_ref() {
                            emitter
                                .emit(format!("→ {agent_display}: working... ({elapsed}s elapsed)"))
                                .await;
                        }
                        next_heartbeat = next_heartbeat_offset(next_heartbeat);
                    }
                }
            }
        };

        match attempt_result {
            Ok(parsed) => {
                if matches!(gemini_backend_selected, Some(GeminiBackend::Agy)) {
                    mcp_bridge::agy_resilience::agy_breaker_record_success();
                }
                let tokens = token_total(&parsed);
                if let Some(metrics) = process_metrics() {
                    metrics.agent_tokens_total.inc_by(tokens);
                }
                persist_daemon_token_record(
                    &agent,
                    &request_id,
                    &parsed,
                    &resolved_cwd,
                    &resolved_repo,
                );
                // Provisional: mandatory peer review can still fail below and overwrite this
                // with failure(). The guard emits only ONCE, on drop, with whatever the final
                // outcome turned out to be — which is precisely the bug that made the old
                // "emit here" version report success for a call that then returned Err.
                // The agy connector reports the model it CHOSE at runtime ("Gemini 3.1 Pro
                // (High)") and stashes it in cli_version (agy.rs::build_result). Without
                // this, $ai_model falls back to the agent key and every Antigravity call
                // charts as the model "gemini", which cannot answer which model ran.
                if agent == "gemini" {
                    match parsed.cli_version.as_deref() {
                        Some(model) if !model.trim().is_empty() => {
                            tel.set_model(model);
                            span.record("agent.model", model);
                        }
                        // Record "unknown" rather than leaving the field Empty. An absent
                        // field is indistinguishable from a span that never reached here, so
                        // silence would hide the very parse regression worth catching: agy
                        // changing its log format and us quietly losing the model forever.
                        _ => {
                            span.record("agent.model", "unknown");
                        }
                    }
                }
                tel.success(parsed.token_usage.clone());
                // $ai_output: the completion the model actually returned.
                tel.set_output(&parsed.response_text);
                span.record("agent.outcome", "success");
                span.record("agent.tokens", tokens);
                span.record("agent.duration_ms", started.elapsed().as_millis() as u64);
                let next_session_id = parsed.session_id.clone();
                // Only a session-scoped call may publish its session id. A one-shot that
                // wrote here would clobber the named session cached under the same
                // (agent, cwd) key and silently destroy its continuity.
                if reuse_session && !is_faildown_attempt {
                    update_worker_session(&agent, &exec_cwd, session_key, next_session_id).await;
                }
                span.record(
                    "session_id",
                    tracing::field::display(parsed.session_id.as_deref().unwrap_or("none")),
                );
                lifecycle.push(LifecycleEvent {
                    state: "DONE".to_string(),
                    detail: format!("{agent_display} responded on attempt {}", idx + 1),
                });
                if let Err(e) = append_outbox_event(&OutboxEvent {
                    ts_ms: core_unix_time_ms(),
                    request_id: request_id.clone(),
                    tool: "ask_agent".to_string(),
                    status: "DONE".to_string(),
                    agent: Some(agent.clone()),
                    detail: lifecycle
                        .last()
                        .map(|e| e.detail.clone())
                        .unwrap_or_default(),
                    cwd: resolved_cwd.clone(),
                    repo: resolved_repo.clone(),
                    branch: resolved_branch.clone(),
                    working_state: Some("DONE".to_string()),
                    token_usage: map_token_usage(parsed.token_usage.as_ref()),
                    tool_name: parsed.tool_calls.last().map(|call| call.tool.clone()),
                }) {
                    tracing::warn!("failed to append outbox event: {e}");
                }
                if let Some(emitter) = progress.as_ref() {
                    emitter.emit(format!("→ {agent_display}: responded ✓")).await;
                }
                if require_peer_review_enabled()
                    && let Err(err) = enforce_mandatory_peer_review(
                        &agent,
                        &parsed.response_text,
                        &exec_cwd,
                        &request_id,
                        &resolved_cwd,
                        &resolved_repo,
                        &resolved_branch,
                        &mut lifecycle,
                        progress.as_ref(),
                    )
                    .await
                {
                    span.record("agent.outcome", "failure");
                    span.record("agent.duration_ms", started.elapsed().as_millis() as u64);
                    // Overwrites the provisional success() above. The call produced a response
                    // but is being rejected, so it is a failure — and must not be counted twice.
                    tel.failure(err.clone());
                    return Err(err);
                }
                // Slice 6: shadow-compare — run the other Gemini backend, attach + log.
                let (sh_backend, sh_resp, sh_err, sh_ms) = if let Some(sb) = shadow_backend {
                    let primary_label = gemini_backend_selected
                        .map(GeminiBackend::as_str)
                        .unwrap_or(agent.as_str());
                    let (resp, err, ms) = run_gemini_shadow(sb, &execution_prompt, &exec_cwd).await;
                    log_shadow_comparison(
                        &request_id,
                        &req.message,
                        primary_label,
                        &parsed.response_text,
                        sb,
                        &resp,
                        &err,
                        ms,
                    );
                    (Some(sb.as_str().to_string()), resp, err, Some(ms))
                } else {
                    (None, None, None, None)
                };
                return Ok(AskAgentResponse::direct(
                    request_id,
                    agent.clone(),
                    parsed.response_text,
                    lifecycle,
                )
                .with_shadow(sh_backend, sh_resp, sh_err, sh_ms));
            }
            Err(e) => {
                let msg = e.to_string();

                // T-013 (REQ-DS-026) persist-before-Err: DeepSeek-ONLY hook.
                // The runner surfaces typed failures via DeepSeekFailureWrapper.
                // When we can recover the typed usage from the failure, persist
                // the token record so billable tokens (esp. 429-with-cost or
                // mid-stream-disconnect estimated) are NOT lost. Blast-radius
                // safeguard: agent gate is the ONLY way in — Gemini/Codex Err
                // paths fall through to the unchanged generic handling below.
                if agent == "deepseek" {
                    if let Some(wrapper) = e.downcast_ref::<DeepSeekFailureWrapper>() {
                        if let Some(ds_usage) = wrapper.0.usage.as_ref() {
                            let synthetic_session = format!("deepseek-err-{request_id}");
                            persist_deepseek_err_tokens(
                                &request_id,
                                &synthetic_session,
                                ds_usage,
                                wrapper.0.request_id.as_deref(),
                                &resolved_cwd,
                                &resolved_repo,
                            );
                        }
                    }
                }

                // REQ-101/103: feed the agy circuit breaker. Quota trips it faster;
                // ambiguous failures bias toward OPEN at a slightly higher bar.
                if matches!(gemini_backend_selected, Some(GeminiBackend::Agy)) {
                    match crate::agy::classify_failure_message(&msg) {
                        crate::agy::AgyFailureClass::Quota => {
                            mcp_bridge::agy_resilience::agy_breaker_record_quota()
                        }
                        crate::agy::AgyFailureClass::AuthOrExec => {
                            mcp_bridge::agy_resilience::agy_breaker_record_other_failure()
                        }
                    }
                }
                if session_for_attempt.is_some() && should_invalidate_cached_session(&msg) {
                    worker_session_id = None;
                    update_worker_session(&agent, &exec_cwd, session_key, None).await;
                    // The one outbox status PostHog could not otherwise see. A stale resume
                    // id is invisible to $ai_generation (the call still succeeds on the
                    // retry), yet it is the failure mode that orphans a named session's
                    // transcript. 26 of these are already in the local outbox and nobody
                    // ever knew.
                    mcp_bridge::posthog::record_session_invalidated(
                        &agent,
                        gemini_backend_selected.map(|b| match b {
                            GeminiBackend::Agy => "agy",
                            GeminiBackend::GeminiCli => "gemini-cli",
                        }),
                        resolved_repo.as_deref(),
                    );
                    let invalidated_detail = format!(
                        "{agent_display} session invalidated after stale resume ID; retrying with a fresh session"
                    );
                    lifecycle.push(LifecycleEvent {
                        state: "SESSION_INVALIDATED".to_string(),
                        detail: invalidated_detail.clone(),
                    });
                    if let Err(e) = append_outbox_event(&OutboxEvent {
                        ts_ms: core_unix_time_ms(),
                        request_id: request_id.clone(),
                        tool: "ask_agent".to_string(),
                        status: "SESSION_INVALIDATED".to_string(),
                        agent: Some(agent.clone()),
                        detail: invalidated_detail,
                        cwd: resolved_cwd.clone(),
                        repo: resolved_repo.clone(),
                        branch: resolved_branch.clone(),
                        working_state: Some("SESSION_INVALIDATED".to_string()),
                        token_usage: None,
                        tool_name: None,
                    }) {
                        tracing::warn!("failed to append outbox event: {e}");
                    }
                    if let Some(emitter) = progress.as_ref() {
                        emitter
                            .emit(format!("→ {agent_display}: stale session detected, respawning..."))
                            .await;
                    }
                }
                if msg.contains("timed out") {
                    lifecycle.push(LifecycleEvent {
                        state: "TIMEOUT".to_string(),
                        detail: format!("{agent_display} timed out on attempt {}", idx + 1),
                    });
                    if let Some(emitter) = progress.as_ref() {
                        emitter
                            .emit(format!("→ {agent_display}: TIMEOUT after 60s ✗"))
                            .await;
                    }
                }
                let model_label = model_override.unwrap_or("default");
                lifecycle.push(LifecycleEvent {
                    state: "RETRY".to_string(),
                    detail: format!(
                        "Retrying {} ({}/{}, model={}) after {}",
                        agent,
                        idx + 1,
                        attempt_schedule.len(),
                        model_label,
                        msg
                    ),
                });
                if let Err(e) = append_outbox_event(&OutboxEvent {
                    ts_ms: core_unix_time_ms(),
                    request_id: request_id.clone(),
                    tool: "ask_agent".to_string(),
                    status: "RETRY".to_string(),
                    agent: Some(agent.clone()),
                    detail: lifecycle
                        .last()
                        .map(|e| e.detail.clone())
                        .unwrap_or_default(),
                    cwd: resolved_cwd.clone(),
                    repo: resolved_repo.clone(),
                    branch: resolved_branch.clone(),
                    working_state: Some("RETRY".to_string()),
                    token_usage: None,
                    tool_name: None,
                }) {
                    tracing::warn!("failed to append outbox event: {e}");
                }
                if let Some(emitter) = progress.as_ref() {
                    emitter
                        .emit(format!("→ {agent_display}: retrying ({}/{})...", idx + 1, attempt_schedule.len()))
                        .await;
                }
                // The REASON an attempt failed reached only the outbox and the lifecycle
                // vec, never the tracing log. So the log showed "trying model X" four
                // times and never once said why, and `grep 429 daemon.log` came back empty
                // while the provider was actively throttling us. A retry that does not say
                // what it is retrying past is a silent failure wearing a progress bar.
                tracing::warn!(
                    attempt = idx + 1,
                    attempts_total = attempt_schedule.len(),
                    model = model_label,
                    backend = gemini_backend_selected.map(|b| match b {
                        GeminiBackend::Agy => "agy",
                        GeminiBackend::GeminiCli => "gemini-cli",
                    }),
                    error = %msg,
                    "agent attempt failed"
                );
                last_err = Some(msg);
                sleep(*backoff).await;
            }
        }
    }

    // REQ-053/054: degraded route. Fires only when the agy backend was selected and
    // hard-failed (auth/exec/quota). Quota-class failures skip gemini-cli (shared
    // quota pool) and go straight to codex. The public agent stays `gemini`; a
    // successful hop returns with substitution-honesty fields + a one-line prefix.
    if matches!(gemini_backend_selected, Some(GeminiBackend::Agy)) {
        let reason = last_err
            .clone()
            .unwrap_or_else(|| "agy backend failed".to_string());
        let class = crate::agy::classify_failure_message(&reason);
        let hops = crate::agy::plan_degraded_route(&crate::agy::degraded_route_env(), class);
        let deadline = started + crate::agy::degraded_total_budget();
        for hop in hops {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                lifecycle.push(LifecycleEvent {
                    state: "DEGRADED_BUDGET_EXHAUSTED".to_string(),
                    detail: "degraded route budget exhausted".to_string(),
                });
                break;
            }
            let hop_display = display_agent_name(hop.agent);
            let degraded_detail =
                format!("agy unavailable ({class:?}); routing to {} ({hop_display})", hop.backend);
            lifecycle.push(LifecycleEvent {
                state: "DEGRADED".to_string(),
                detail: degraded_detail.clone(),
            });
            let _ = append_outbox_event(&OutboxEvent {
                ts_ms: core_unix_time_ms(),
                request_id: request_id.clone(),
                tool: "ask_agent".to_string(),
                status: "DEGRADED".to_string(),
                agent: Some(agent.clone()),
                detail: degraded_detail,
                cwd: resolved_cwd.clone(),
                repo: resolved_repo.clone(),
                branch: resolved_branch.clone(),
                working_state: Some("DEGRADED".to_string()),
                token_usage: None,
                tool_name: None,
            });
            if let Some(emitter) = progress.as_ref() {
                emitter
                    .emit(format!("→ {agent_display} unavailable — falling back to {}…", hop.backend))
                    .await;
            }

            // Run the hop within the remaining budget. The gemini-cli hop bypasses the
            // selector — dispatching agent="gemini" would re-select agy and loop.
            let hop_result = match hop.backend {
                "gemini-cli" => {
                    let (bin, args) = gemini_command();
                    timeout(
                        remaining,
                        run_gemini_cli_process_with_session(
                            &bin, &args, &execution_prompt, &exec_cwd, None, None,
                        ),
                    )
                    .await
                }
                _ => {
                    timeout(
                        remaining,
                        run_named_agent_with_session_and_model(
                            hop.agent, &execution_prompt, &exec_cwd, None, None, None, None,
                        ),
                    )
                    .await
                }
            };

            match hop_result {
                Ok(Ok(parsed)) => {
                    persist_daemon_token_record(
                        hop.agent, &request_id, &parsed, &resolved_cwd, &resolved_repo,
                    );
                    let done_detail = format!("answered by {} (degraded from agy)", hop.backend);
                    lifecycle.push(LifecycleEvent {
                        state: "DONE".to_string(),
                        detail: done_detail.clone(),
                    });
                    let _ = append_outbox_event(&OutboxEvent {
                        ts_ms: core_unix_time_ms(),
                        request_id: request_id.clone(),
                        tool: "ask_agent".to_string(),
                        status: "DONE".to_string(),
                        agent: Some(agent.clone()),
                        detail: done_detail,
                        cwd: resolved_cwd.clone(),
                        repo: resolved_repo.clone(),
                        branch: resolved_branch.clone(),
                        working_state: Some("DONE".to_string()),
                        token_usage: None,
                        tool_name: None,
                    });
                    if let Some(emitter) = progress.as_ref() {
                        emitter.emit(format!("→ {hop_display}: responded ✓")).await;
                    }
                    span.record("agent.outcome", "degraded_success");
                    span.record("agent.duration_ms", started.elapsed().as_millis() as u64);
                    // Previously emitted NOTHING: a degraded success vanished from LLM
                    // analytics entirely, so "how often does gemini degrade rather than fail?"
                    // — the exact question this taxonomy exists to answer — was unanswerable.
                    tel.degraded_success(None);
                    // $ai_output for the degraded hop's completion.
                    tel.set_output(&parsed.response_text);
                    // REQ-053 R3: a text prefix only when a DIFFERENT agent answered
                    // (codex). A gemini-cli hop is the same agent on the legacy backend,
                    // so honesty lives in the fields/lifecycle, not an alarming prefix.
                    let prefix = if hop.agent != agent {
                        format!(
                            "⚠ {} unavailable — answered by {hop_display}\n\n",
                            display_agent_name(&agent)
                        )
                    } else {
                        String::new()
                    };
                    return Ok(AskAgentResponse {
                        request_id,
                        agent: agent.clone(),
                        response: format!("{prefix}{}", parsed.response_text),
                        lifecycle,
                        answered_by_agent: Some(hop.agent.to_string()),
                        answered_by_backend: Some(hop.backend.to_string()),
                        degraded_from_backend: Some("agy".to_string()),
                        degradation_reason: Some(reason.clone()),
                        shadow_backend: None,
                        shadow_response: None,
                        shadow_error: None,
                        shadow_latency_ms: None,
                    });
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    lifecycle.push(LifecycleEvent {
                        state: "DEGRADED_FAILED".to_string(),
                        detail: format!("{} hop failed: {msg}", hop.backend),
                    });
                    last_err = Some(msg);
                }
                Err(_) => {
                    lifecycle.push(LifecycleEvent {
                        state: "DEGRADED_TIMEOUT".to_string(),
                        detail: format!("{} hop exceeded remaining degraded budget", hop.backend),
                    });
                    last_err = Some(format!("{} hop timed out", hop.backend));
                }
            }
        }
    }

    lifecycle.push(LifecycleEvent {
        state: "FAILED".to_string(),
        detail: format!(
            "{} failed after {} attempts",
            agent_display,
            attempt_schedule.len()
        ),
    });
    if let Err(e) = append_outbox_event(&OutboxEvent {
        ts_ms: core_unix_time_ms(),
        request_id: request_id.clone(),
        tool: "ask_agent".to_string(),
        status: "FAILED".to_string(),
        agent: Some(agent.clone()),
        detail: lifecycle
            .last()
            .map(|e| e.detail.clone())
            .unwrap_or_default(),
        cwd: resolved_cwd.clone(),
        repo: resolved_repo.clone(),
        branch: resolved_branch.clone(),
        working_state: Some("FAILED".to_string()),
        token_usage: None,
        tool_name: None,
    }) {
        tracing::warn!("failed to append outbox event: {e}");
    }
    if let Some(emitter) = progress.as_ref() {
        emitter
            .emit(format!("→ {agent_display}: FAILED after {} attempts", attempt_schedule.len()))
            .await;
    }

    let fallback_path = spawn_dead_drop(
        &agent,
        &req.message,
        &last_err.clone().unwrap_or_else(|| "unknown error".to_string()),
        &resolved_cwd,
        &resolved_repo,
        &resolved_branch,
    )
    .ok();
    if let Some(path) = fallback_path.as_ref() {
        lifecycle.push(LifecycleEvent {
            state: "FALLBACK".to_string(),
            detail: format!("dead drop launched: {}", path.display()),
        });
        let _ = append_outbox_event(&OutboxEvent {
            ts_ms: core_unix_time_ms(),
            request_id: request_id.clone(),
            tool: "ask_agent".to_string(),
            status: "FALLBACK".to_string(),
            agent: Some(agent.clone()),
            detail: format!("dead drop launched: {}", path.display()),
            cwd: resolved_cwd.clone(),
            repo: resolved_repo.clone(),
            branch: resolved_branch.clone(),
            working_state: Some("FALLBACK".to_string()),
            token_usage: None,
            tool_name: None,
        });
        if let Some(emitter) = progress.as_ref() {
            emitter
                .emit(format!("→ {agent_display}: dead drop launched, {}", path.display()))
                .await;
        }
    }
    span.record("agent.outcome", "failure");
    span.record("agent.tokens", 0_u64);
    span.record("agent.duration_ms", started.elapsed().as_millis() as u64);

    let failure_detail = last_err.unwrap_or_else(|| "unknown error".to_string());

    // The terminal failure path (retries exhausted). The guard emits $ai_generation AND, because
    // this is a real failure, an $exception — once, on drop.
    tel.failure(failure_detail.clone());

    Err(format!(
        "ask_agent failed after lifecycle {:?}: {}{}",
        lifecycle
            .iter()
            .map(|e| e.state.as_str())
            .collect::<Vec<_>>(),
        failure_detail,
        fallback_path
            .map(|p| format!("; dead drop launched at {}", p.display()))
            .unwrap_or_default()
    ))
}

fn inject_tool_marker_prompt(user_prompt: &str) -> String {
    format!("{TOOL_MARKER_INSTRUCTIONS}\n\nUser request:\n{user_prompt}")
}

fn require_peer_review_enabled() -> bool {
    std::env::var("TRIUMVIRATE_REQUIRE_PEER_REVIEW")
        .ok()
        .map(|v| !matches!(v.to_lowercase().as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(false)
}

fn resolve_absolute_project_root(exec_cwd: &str) -> Result<PathBuf, String> {
    let raw = PathBuf::from(exec_cwd);
    let base = if raw.is_absolute() {
        raw
    } else {
        std::env::current_dir()
            .map_err(|e| format!("mandatory peer review failed to resolve cwd: {e}"))?
            .join(raw)
    };
    match std::fs::canonicalize(&base) {
        Ok(canonical) => Ok(canonical),
        Err(_) => Ok(base),
    }
}

#[allow(clippy::too_many_arguments)]
async fn enforce_mandatory_peer_review(
    agent: &str,
    response_text: &str,
    exec_cwd: &str,
    request_id: &str,
    resolved_cwd: &Option<String>,
    resolved_repo: &Option<String>,
    resolved_branch: &Option<String>,
    lifecycle: &mut Vec<LifecycleEvent>,
    progress: Option<&ProgressEmitter>,
) -> Result<(), String> {
    let project_root = resolve_absolute_project_root(exec_cwd)?;
    fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
        .map_err(|e| format!("mandatory peer review failed to init spool dir: {e}"))?;
    LedgerStore::open(project_root.clone())
        .map_err(|e| format!("mandatory peer review failed to open ledger: {e}"))?;
    let engine = PeerReviewEngine::new(project_root.clone())
        .map_err(|e| format!("mandatory peer review engine init failed: {e}"))?;

    let artifact = response_text.chars().take(16_384).collect::<String>();
    let review = engine
        .request_review(ReviewRequest {
            fleet_id: None,
            author_agent: agent.to_string(),
            artifact,
            review_type: "agent_output".to_string(),
        })
        .map_err(|e| format!("mandatory peer review request failed: {e}"))?;
    let reviewer = review
        .reviewer_agent
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let pending_detail = format!(
        "mandatory peer review requested: {} reviewer={}",
        review.review_id, reviewer
    );
    lifecycle.push(LifecycleEvent {
        state: "REVIEW_PENDING".to_string(),
        detail: pending_detail.clone(),
    });
    if let Err(e) = append_outbox_event(&OutboxEvent {
        ts_ms: core_unix_time_ms(),
        request_id: request_id.to_string(),
        tool: "ask_agent".to_string(),
        status: "REVIEW_PENDING".to_string(),
        agent: Some(agent.to_string()),
        detail: pending_detail.clone(),
        cwd: resolved_cwd.clone(),
        repo: resolved_repo.clone(),
        branch: resolved_branch.clone(),
        working_state: Some("REVIEW_PENDING".to_string()),
        token_usage: None,
        tool_name: None,
    }) {
        tracing::warn!("failed to append outbox event: {e}");
    }
    if let Some(emitter) = progress {
        emitter
            .emit(format!(
                "→ {}: peer review requested ({})",
                display_agent_name(agent),
                review.review_id
            ))
            .await;
    }

    let _ = engine
        .submit_review(
            &review.review_id,
            "approve",
            Some("auto-approved in mandatory peer review mode"),
        )
        .map_err(|e| format!("mandatory peer review submit failed: {e}"))?;
    let reviewed = engine
        .get_review(&review.review_id)
        .map_err(|e| format!("mandatory peer review load failed: {e}"))?
        .ok_or_else(|| "mandatory peer review missing after submit".to_string())?;
    if reviewed.state != "done" {
        return Err(format!(
            "mandatory peer review not completed: review_id={} state={}",
            reviewed.review_id, reviewed.state
        ));
    }

    let done_detail = format!(
        "mandatory peer review completed: {} verdict={}",
        reviewed.review_id,
        reviewed
            .verdict
            .unwrap_or_else(|| "unknown".to_string())
    );
    lifecycle.push(LifecycleEvent {
        state: "REVIEW_DONE".to_string(),
        detail: done_detail.clone(),
    });
    if let Err(e) = append_outbox_event(&OutboxEvent {
        ts_ms: core_unix_time_ms(),
        request_id: request_id.to_string(),
        tool: "ask_agent".to_string(),
        status: "REVIEW_DONE".to_string(),
        agent: Some(agent.to_string()),
        detail: done_detail.clone(),
        cwd: resolved_cwd.clone(),
        repo: resolved_repo.clone(),
        branch: resolved_branch.clone(),
        working_state: Some("REVIEW_DONE".to_string()),
        token_usage: None,
        tool_name: None,
    }) {
        tracing::warn!("failed to append outbox event: {e}");
    }
    if let Some(emitter) = progress {
        emitter
            .emit(format!(
                "→ {}: peer review approved ({})",
                display_agent_name(agent),
                reviewed.review_id
            ))
            .await;
    }
    Ok(())
}

async fn run_named_agent_with_session(
    agent: &str,
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
    events_tx: Option<mpsc::Sender<WorkingStateEvent>>,
) -> anyhow::Result<ParsedAgentResult> {
    run_named_agent_with_session_and_model(agent, message, cwd, session_id, events_tx, None, None).await
}

/// Run the shadow Gemini backend (the non-primary one) for comparison and return its
/// (response, error, latency_ms). Shadow-compare mode (Slice 6). Best-effort: it never
/// affects the primary result — failures are captured, not propagated — and it does NOT
/// feed the circuit breaker (an observation, not a routing decision).
async fn run_gemini_shadow(
    shadow_backend: GeminiBackend,
    prompt: &str,
    cwd: &str,
) -> (Option<String>, Option<String>, u64) {
    let started = Instant::now();
    let result = match shadow_backend {
        GeminiBackend::Agy => {
            let (bin, args) = agy_command();
            crate::agy::run_agy_cli_process_with_session(&bin, &args, prompt, cwd, None, None).await
        }
        GeminiBackend::GeminiCli => {
            let (bin, args) = gemini_command();
            run_gemini_cli_process_with_session(&bin, &args, prompt, cwd, None, None).await
        }
    };
    let latency_ms = started.elapsed().as_millis() as u64;
    match result {
        Ok(parsed) => (Some(parsed.response_text), None, latency_ms),
        Err(e) => (None, Some(e.to_string()), latency_ms),
    }
}

/// Append a shadow-compare record to `<triumvirate_home>/agy-shadow-compare.jsonl` for
/// offline review (Slice 6). Best-effort; logging failures are swallowed.
#[allow(clippy::too_many_arguments)]
fn log_shadow_comparison(
    request_id: &str,
    prompt: &str,
    primary_backend: &str,
    primary_response: &str,
    shadow_backend: GeminiBackend,
    shadow_response: &Option<String>,
    shadow_error: &Option<String>,
    shadow_latency_ms: u64,
) {
    let Ok(home) = daemon_core::triumvirate_home_dir() else {
        return;
    };
    let record = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "request_id": request_id,
        "prompt": prompt,
        "primary_backend": primary_backend,
        "primary_response": primary_response,
        "shadow_backend": shadow_backend.as_str(),
        "shadow_response": shadow_response,
        "shadow_error": shadow_error,
        "shadow_latency_ms": shadow_latency_ms,
    });
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("agy-shadow-compare.jsonl"))
    {
        use std::io::Write;
        let _ = writeln!(f, "{record}");
    }
}

async fn run_named_agent_with_session_and_model(
    agent: &str,
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
    events_tx: Option<mpsc::Sender<WorkingStateEvent>>,
    model_override: Option<&str>,
    // T-012: per-call DeepSeek overrides come from AskAgentRequest. Only the
    // deepseek arm reads these — Gemini/Codex ignore. Caller passes None when
    // there is no upstream request context (degraded route, prewarm, etc.).
    req_overrides: Option<&AskAgentRequest>,
) -> anyhow::Result<ParsedAgentResult> {
    let _ = session_id; // deepseek arm intentionally ignores resume tokens — T-014.
    match agent {
        "gemini" => {
            let (bin, mut args) = gemini_command();
            if let Some(model) = model_override {
                args.push("--model".to_string());
                args.push(model.to_string());
            }
            run_agent_process_with_session(
                "gemini",
                &bin,
                &args,
                message,
                cwd,
                session_id,
                events_tx,
            )
            .await
        }
        "codex" => {
            let (bin, args) = codex_command();
            run_agent_process_with_session(
                "codex",
                &bin,
                &args,
                message,
                cwd,
                session_id,
                events_tx,
            )
            .await
        }
        "grok" => {
            let (bin, args) = mcp_bridge::grok_command();
            run_grok_cli_process_with_session(&bin, &args, message, cwd, session_id, events_tx).await
        }
        "claude" => {
            let (bin, args) = claude_command();
            // Assuming claude CLI can be invoked similarly to codex/gemini via JSON streaming.
            // If the user's brief mentioned Option A (subprocess), we use run_agent_process_with_session.
            run_agent_process_with_session(
                "claude",
                &bin,
                &args,
                message,
                cwd,
                session_id,
                events_tx,
            )
            .await
        }
        "deepseek" => run_deepseek_agent(message, req_overrides).await,
        _ => anyhow::bail!("unsupported agent: {agent}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// T-012 (REQ-DS-014 / REQ-DS-023): DeepSeek dispatch arm.
// ─────────────────────────────────────────────────────────────────────────────

use mcp_bridge::deepseek as ds;
use mcp_bridge::deepseek_config::{
    DeepSeekConfig, ReasoningEffort as CfgEffort, ThinkingMode,
};
use shared_types::{DeepSeekEffort, DeepSeekThinking};

/// Process-static DeepSeek runtime state — config + reqwest client + resilience
/// state. Initialised on first use. Returns an `anyhow::Error` if the env-load
/// validation rejects something (e.g. MAX_TOKENS=oops, REASONING_CAP >= MAX_TOKENS).
fn deepseek_runtime()
-> anyhow::Result<&'static (DeepSeekConfig, reqwest::Client, ds::ResilienceState)> {
    static CELL: std::sync::OnceLock<
        Result<(DeepSeekConfig, reqwest::Client, ds::ResilienceState), String>,
    > = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        let cfg = DeepSeekConfig::from_env().map_err(|e| e.to_string())?;
        let client = ds::build_client(&cfg).map_err(|e| e.to_string())?;
        let resilience = ds::ResilienceState::from_cfg(&cfg);
        Ok((cfg, client, resilience))
    })
    .as_ref()
    .map_err(|s| anyhow::anyhow!("deepseek runtime init failed: {s}"))
}

/// Clone the cached cfg and overlay per-call overrides from AskAgentRequest.
/// Returns a borrowed reference to the cached cfg when no overrides apply
/// (avoiding an allocation on the hot path).
fn cfg_with_overrides(base: &DeepSeekConfig, req: Option<&AskAgentRequest>) -> DeepSeekConfig {
    let mut cfg = base.clone();
    let Some(r) = req else {
        return cfg;
    };
    if let Some(t) = r.deepseek_thinking {
        cfg.thinking = match t {
            DeepSeekThinking::Enabled => ThinkingMode::Enabled,
            DeepSeekThinking::Disabled => ThinkingMode::Disabled,
        };
    }
    if let Some(e) = r.deepseek_reasoning_effort {
        cfg.reasoning_effort = match e {
            // Wire surface accepts five levels; the API treats Low/Medium/High
            // as High and Xhigh as Max. Collapse here so the runner only sees
            // High|Max (T-011 contract).
            DeepSeekEffort::Low | DeepSeekEffort::Medium | DeepSeekEffort::High => {
                CfgEffort::High
            }
            DeepSeekEffort::Max | DeepSeekEffort::Xhigh => CfgEffort::Max,
        };
    }
    if let Some(n) = r.deepseek_max_tokens {
        cfg.max_tokens = n;
    }
    // 2026-05-26 follow-up: per-call model override. Empty strings are ignored
    // (treated as not-set) so a caller sending an explicit "" doesn't blank
    // the configured default with an invalid value.
    if let Some(m) = r.deepseek_model.as_deref()
        && !m.trim().is_empty()
    {
        cfg.model = m.to_string();
    }
    cfg
}

/// Wrap a DeepSeekFailure inside anyhow::Error so T-013 (execute_ask_agent)
/// can downcast and inspect the typed `usage` field for persist-before-Err.
#[derive(Debug)]
pub(crate) struct DeepSeekFailureWrapper(pub(crate) ds::DeepSeekFailure);

impl std::fmt::Display for DeepSeekFailureWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.kind)
    }
}

impl std::error::Error for DeepSeekFailureWrapper {}

async fn run_deepseek_agent(
    message: &str,
    req_overrides: Option<&AskAgentRequest>,
) -> anyhow::Result<ParsedAgentResult> {
    let (base_cfg, client, resilience) = deepseek_runtime()?;
    run_deepseek_with_runtime(base_cfg, client, resilience, message, req_overrides).await
}

/// Testable inner. Tests inject their own (cfg, client, resilience) tuple,
/// bypassing the OnceLock-cached process-global runtime.
pub(crate) async fn run_deepseek_with_runtime(
    base_cfg: &DeepSeekConfig,
    client: &reqwest::Client,
    resilience: &ds::ResilienceState,
    message: &str,
    req_overrides: Option<&AskAgentRequest>,
) -> anyhow::Result<ParsedAgentResult> {
    let cfg = cfg_with_overrides(base_cfg, req_overrides);

    // T-014 (REQ-DS-020): synthetic session_id `deepseek-<uuid>`. Inbound
    // session_id is intentionally NOT consulted — DeepSeek is stateless v1.
    let session_id = format!("deepseek-{}", Uuid::new_v4());

    let include_reasoning = req_overrides
        .and_then(|r| r.deepseek_include_reasoning)
        .unwrap_or(false);

    // 2026-05-26: tool-tag hallucination mitigation. DeepSeek V4 has a
    // documented intermittent failure mode where the model emits XML/DSML-
    // style tool-call tags as plain content instead of structured
    // tool_calls — even when no tools are defined. Multiple GitHub issues
    // across providers (Chutes, NVIDIA, Foundry, vLLM, sglang). Exacerbated
    // when the user message mentions tool-call terminology by name (the
    // model treats the mention as evidence the tool exists). Hit twice in
    // production this session: model returned only a `<triumvirate_tool>`
    // tag with no real review content, finish_reason=stop.
    //
    // Mitigation: a stable system message that (a) tells the model it has
    // NO tools available, (b) names common tool-tag patterns as forbidden
    // output. This is a defense at the prompt layer; the model still has
    // the intermittent bug at the weights level, but the system prompt
    // dramatically reduces incidence per the "removing ambiguity that
    // causes hallucination" pattern documented in agentic-LLM literature.
    //
    // Side benefit: a stable system message becomes a cacheable prefix
    // for DeepSeek's prompt cache, so repeated consults can hit the
    // discounted cache-hit price ($0.003625/M vs $0.435/M on v4-pro)
    // when DeepSeek's opportunistic cache lands.
    const NO_TOOL_EMULATION_SYSTEM: &str = "You are answering a single user query and have no access to tools, function-calling APIs, file systems, agentic frameworks, web search, or external context-fetching mechanisms. Respond directly with your complete answer in plain text or markdown.\n\nDO NOT emit XML-style tool-call tags such as `<triumvirate_tool>`, `<tool_use>`, `<function_call>`, `<invoke>`, DSML markers, or any similar tool-call placeholder. The user's message contains the entire request — treat it as self-contained. If a referenced concept (a tool name, a session id, a file path) appears in the user's message as EXAMPLE TEXT, do not interpret it as something you should call or invoke — treat it as literal content to discuss.\n\nIf you genuinely lack information needed to answer, say so in plain prose rather than emitting a tool-call placeholder.";

    let messages = vec![
        ds::RequestMessage {
            role: "system".to_string(),
            content: NO_TOOL_EMULATION_SYSTEM.to_string(),
        },
        ds::RequestMessage {
            role: "user".to_string(),
            content: message.to_string(),
        },
    ];
    let prompt_chars_estimate =
        (NO_TOOL_EMULATION_SYSTEM.chars().count() + message.chars().count()) as i64;

    let run_req = ds::RunRequest {
        messages,
        session_id: session_id.clone(),
        prompt_chars_estimate,
        include_reasoning,
    };

    match ds::run(&cfg, client, &run_req, resilience).await {
        Ok(parsed) => Ok(parsed),
        Err(failure) => Err(anyhow::Error::new(DeepSeekFailureWrapper(failure))),
    }
}

async fn prewarm_worker(agent: &str, cwd: &str) {
    let worker = acquire_worker(agent, cwd, None).await;

    // If a session already exists for this (agent, cwd), the worker IS warm — there is nothing
    // to prewarm. Calling anyway was actively harmful in two ways:
    //   1. It RESUMED that session to say "reply with only: ready". For codex, resume replays the
    //      entire transcript as input — so prewarm paid the full replay cost (measured elsewhere
    //      at ~164k tokens) to send three words.
    //   2. It wrote the result back via update_worker_session, so a prewarm could overwrite the
    //      session a named ask_session depends on.
    // Prewarm now only ever CREATES a session, never resumes or replaces one.
    if worker.session_id.is_some() {
        tracing::debug!("prewarm skipped for {agent} cwd={cwd}: session already warm");
        return;
    }

    let warm_prompt = "Prewarm this session. Reply with only: ready";
    let warm_result = timeout(
        daemon_prewarm_timeout(),
        run_named_agent_with_session(agent, warm_prompt, cwd, None, None),
    )
    .await;

    match warm_result {
        Ok(Ok(parsed)) => {
            update_worker_session(agent, cwd, None, parsed.session_id).await;
            tracing::info!("prewarm complete for {agent} cwd={cwd}");
        }
        Ok(Err(err)) => {
            tracing::warn!("prewarm failed for {agent} cwd={cwd}: {err}");
            let _ = dismiss_worker(agent, cwd, None).await;
        }
        Err(_) => {
            tracing::warn!("prewarm timeout for {agent} cwd={cwd}");
            let _ = dismiss_worker(agent, cwd, None).await;
        }
    }
}

pub(crate) async fn prewarm_daemon_workers() {
    if !daemon_prewarm_enabled() {
        tracing::info!("daemon prewarm disabled");
        return;
    }

    let cwds = daemon_prewarm_cwds();
    for cwd in cwds {
        prewarm_worker("gemini", &cwd).await;
        prewarm_worker("codex", &cwd).await;
    }
}

fn is_mock_connector(bin: &str) -> bool {
    std::path::Path::new(bin)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|name| name.starts_with("mock-"))
        .unwrap_or(false)
}

fn connector_timeout() -> Duration {
    std::env::var("TRIUMVIRATE_CONNECTOR_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(180))
}

fn daemon_prewarm_enabled() -> bool {
    std::env::var("TRIUMVIRATE_DAEMON_PREWARM")
        .ok()
        .map(|v| !matches!(v.to_lowercase().as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(true)
}

fn daemon_prewarm_timeout() -> Duration {
    std::env::var("TRIUMVIRATE_DAEMON_PREWARM_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(60))
}

fn daemon_prewarm_cwds() -> Vec<String> {
    if let Ok(raw) = std::env::var("TRIUMVIRATE_DAEMON_PREWARM_CWDS") {
        let cwds = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !cwds.is_empty() {
            return cwds;
        }
    }

    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        out.push(home.display().to_string());
    }
    if let Ok(cwd) = std::env::current_dir() {
        let cwd = cwd.display().to_string();
        if !out.iter().any(|v| v == &cwd) {
            out.push(cwd);
        }
    }
    if !out.is_empty() {
        return out;
    }

    vec![".".to_string()]
}

#[cfg(test)]
async fn run_mock_connector_process(
    agent: &str,
    bin: &str,
    args: &[String],
    message: &str,
    session_id: Option<&str>,
) -> anyhow::Result<ParsedAgentResult> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut child = Command::new(bin)
        .args(args)
        .env("TRIUMVIRATE_WORKER_SESSION_ID", session_id.unwrap_or(""))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(format!("{message}\n").as_bytes())
            .await?;
        stdin.flush().await?;
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Antigravity stdout missing"))?;
    let mut stderr_reader = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("Antigravity stderr missing"))?;
    let mut lines = BufReader::new(stdout).lines();
    let mut non_json_line: Option<String> = None;

    // The mock connector may emit readiness notifications before the final result; scan until we
    // find a JSON-RPC payload with result.text.
    let read_result = timeout(Duration::from_secs(5), async {
        while let Some(line) = lines.next_line().await? {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(text) = json
                    .get("result")
                    .and_then(|r| r.get("text"))
                    .and_then(|t| t.as_str())
                {
                    return Ok(text.to_string());
                }
            } else {
                non_json_line = Some(trimmed.to_string());
            }
        }
        if let Some(line) = non_json_line {
            Err(anyhow::anyhow!("mock connector output: {line}"))
        } else {
            Err(anyhow::anyhow!("no result.text message from gemini connector"))
        }
    })
    .await;

    let response = match read_result {
        Ok(result) => match result {
            Ok(response) => response,
            Err(err) => {
                let _ = child.kill().await;
                let mut stderr = String::new();
                let _ = stderr_reader.read_to_string(&mut stderr).await;
                let _ = child.wait().await;
                let stderr = stderr.trim();
                if !stderr.is_empty() {
                    anyhow::bail!("mock connector failed: {stderr}");
                }
                return Err(err);
            }
        },
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            anyhow::bail!("mock connector timed out")
        }
    };

    let _ = child.kill().await;
    let _ = child.wait().await;

    let out_session = session_id
        .map(ToString::to_string)
        .or_else(|| Some(format!("mock-session-{}", Uuid::new_v4())));
    Ok(ParsedAgentResult {
        response_text: response,
        session_id: out_session,
        events: vec![],
        tool_calls: vec![],
        token_usage: None,
        cli_version: None,
        parser_mode: format!("{agent}-mock"),
    })
}

fn has_any_arg(args: &[String], candidates: &[&str]) -> bool {
    args.iter().any(|arg| candidates.iter().any(|c| arg == c))
}

fn codex_auto_approve_enabled() -> bool {
    std::env::var("TRIUMVIRATE_CODEX_AUTO_APPROVE")
        .ok()
        .or_else(|| std::env::var("CODEX_AUTO_APPROVE").ok())
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

/// Yolo by default: launch codex with `--dangerously-bypass-approvals-and-sandbox`
/// (no sandbox, no approval prompts, full FS access) so it can write outside its
/// cwd — e.g. into a sibling project the consult is about. Set
/// `TRIUMVIRATE_CODEX_SANDBOX=1` to fall back to the `--full-auto` workspace-write
/// preset (writes confined to cwd). Mirrors agy's `TRIUMVIRATE_AGY_SANDBOX`.
fn codex_yolo_enabled() -> bool {
    !std::env::var("TRIUMVIRATE_CODEX_SANDBOX")
        .ok()
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn codex_approval_channel_mode() -> ApprovalChannelMode {
    let probe_response = match std::env::var("TRIUMVIRATE_CODEX_APPROVAL_PROBE_RESPONSE") {
        Ok(response) => response,
        Err(_) => {
            tracing::warn!(
                "TRIUMVIRATE_CODEX_APPROVAL_PROBE_RESPONSE not set; using --full-auto fallback"
            );
            return ApprovalChannelMode::FullAutoFallback;
        }
    };

    match probe_approval_response_channel(&probe_response) {
        Ok(mode) => mode,
        Err(err) => {
            tracing::warn!(
                "codex app-server approval response channel probe failed ({err}); using --full-auto fallback"
            );
            ApprovalChannelMode::FullAutoFallback
        }
    }
}

fn record_auto_approved_action(cwd: &str, action: &str, mode: &str) {
    let store = match LedgerStore::open(PathBuf::from(cwd)) {
        Ok(store) => store,
        Err(err) => {
            tracing::warn!("failed to open ledger for auto-approval record: {err}");
            return;
        }
    };

    if let Err(err) = store.record(ManualRecord {
        session_id: None,
        title: "Codex auto-approved action".to_string(),
        narrative: format!("mode={mode}; action={action}"),
        facts_json: None,
        concepts_json: None,
        affected_files_json: None,
        summary_type: "auto_approved".to_string(),
    }) {
        tracing::warn!("failed to write auto-approval record: {err}");
    }
}

async fn run_gemini_cli_process_with_session(
    bin: &str,
    args: &[String],
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
    events_tx: Option<mpsc::Sender<WorkingStateEvent>>,
) -> anyhow::Result<ParsedAgentResult> {
    if !gemini_streaming_enabled() {
        return run_gemini_batch_process_with_session(bin, args, message, cwd, session_id).await;
    }

    let mut final_args = args.to_vec();
    if !has_any_arg(&final_args, &["-o", "--output-format"]) {
        final_args.push("-o".to_string());
        final_args.push("stream-json".to_string());
    }
    if let Some(session_id) = session_id
        && !has_any_arg(&final_args, &["-r", "--resume"])
    {
        final_args.push("-r".to_string());
        final_args.push(session_id.to_string());
    }
    if !has_any_arg(&final_args, &["-p", "--prompt"]) {
        final_args.push("-p".to_string());
        final_args.push(message.to_string());
    }

    let mut command = Command::new(bin);
    command
        .args(&final_args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    configure_process_group(&mut command);
    let mut child = command.spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Antigravity stdout missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("Antigravity stderr missing"))?;

    // Monitor stderr for 429/capacity errors and signal abort via channel
    let (abort_tx, mut abort_rx) = mpsc::channel::<String>(1);
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                tracing::debug!("gemini stderr: {trimmed}");
                let lower = trimmed.to_lowercase();
                if lower.contains("429") || lower.contains("no capacity") || lower.contains("resource exhausted") {
                    let _ = abort_tx.try_send(trimmed.to_string());
                }
            }
        }
    });

    let mut parser = GeminiStreamParser::new();
    let mut reader = BufReader::new(stdout).lines();
    let mut raw_output = String::new();
    let timeout_duration = connector_timeout();
    let read = async {
        loop {
            tokio::select! {
                line_result = reader.next_line() => {
                    match line_result? {
                        Some(line) => {
                            raw_output.push_str(&line);
                            raw_output.push('\n');
                            if let Some(event) = parser.parse_line(&line) {
                                emit_working_event(events_tx.as_ref(), event);
                            }
                        }
                        None => break,
                    }
                }
                Some(err_msg) = abort_rx.recv() => {
                    // Operator-facing error text: the product is Antigravity, not "gemini".
                    // (Caught by an actual 429 during verification — the internal dispatch key
                    // was surfacing straight into the error a human reads.)
                    anyhow::bail!("Antigravity 429 capacity error (fast-fail): {err_msg}");
                }
            }
        }
        anyhow::Ok(())
    };
    match timeout(timeout_duration, read).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            kill_process_group(&mut child);
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(e);
        }
        Err(_) => {
            kill_process_group(&mut child);
            let _ = child.kill().await;
            let _ = child.wait().await;
            anyhow::bail!("Antigravity connector timed out");
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("Antigravity connector failed: exited with status {status}");
    }

    let mut parsed = parser.finish();
    if parsed.response_text.trim().is_empty() {
        parsed.response_text = raw_output.trim().to_string();
    }
    if parsed.session_id.is_none() {
        parsed.session_id = session_id.map(ToString::to_string);
    }
    Ok(parsed)
}

async fn run_gemini_batch_process_with_session(
    bin: &str,
    args: &[String],
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
) -> anyhow::Result<ParsedAgentResult> {
    let mut final_args = args.to_vec();
    if !has_any_arg(&final_args, &["-o", "--output-format"]) {
        final_args.push("-o".to_string());
        final_args.push("stream-json".to_string());
    }
    if let Some(session_id) = session_id
        && !has_any_arg(&final_args, &["-r", "--resume"])
    {
        final_args.push("-r".to_string());
        final_args.push(session_id.to_string());
    }
    if !has_any_arg(&final_args, &["-p", "--prompt"]) {
        final_args.push("-p".to_string());
        final_args.push(message.to_string());
    }
    let output = timeout(
        connector_timeout(),
        Command::new(bin)
            .args(&final_args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Antigravity connector timed out"))??;

    if !output.status.success() {
        anyhow::bail!("Antigravity connector failed: exited with status {}", output.status);
    }
    let mut parser = GeminiStreamParser::new();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    for line in stdout.lines() {
        let _ = parser.parse_line(line);
    }
    let mut parsed = parser.finish();
    if parsed.response_text.trim().is_empty() {
        parsed.response_text = stdout.trim().to_string();
    }
    if parsed.session_id.is_none() {
        parsed.session_id = session_id.map(ToString::to_string);
    }
    Ok(parsed)
}

fn extract_text_from_jsonl(stdout: &str) -> Option<String> {
    let mut candidate = None;
    for line in stdout.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        if let Some(text) = json.get("text").and_then(|t| t.as_str()) {
            candidate = Some(text.to_string());
            continue;
        }

        if let Some(text) = json
            .get("result")
            .and_then(|r| r.get("text"))
            .and_then(|t| t.as_str())
        {
            candidate = Some(text.to_string());
            continue;
        }

        if let Some(text) = json
            .get("response")
            .and_then(|r| r.get("output_text"))
            .and_then(|t| t.as_str())
        {
            candidate = Some(text.to_string());
        }
    }
    candidate
}

fn is_git_worktree(path: &str) -> bool {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output();
    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim() == "true",
        _ => false,
    }
}

async fn run_codex_cli_process_with_session(
    bin: &str,
    args: &[String],
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
    events_tx: Option<mpsc::Sender<WorkingStateEvent>>,
) -> anyhow::Result<ParsedAgentResult> {
    // If the configured protocol is `app-server` but the installed codex
    // binary no longer implements the JSON-RPC stdio server there (0.121+
    // turned that subcommand into a tooling namespace), fall back to `exec`
    // with a loud warning. Without this, every call silently exits status 1.
    let configured_protocol = codex_protocol();
    let caps = mcp_bridge::codex_capabilities();
    let protocol = if configured_protocol == "app-server"
        && !caps.has_app_server_protocol_server
    {
        tracing::warn!(
            version = %caps.version,
            "TRIUMVIRATE_CODEX_PROTOCOL=app-server but installed codex does not \
             expose the app-server JSON-RPC server; falling back to `exec` protocol"
        );
        "exec".to_string()
    } else {
        configured_protocol
    };
    let auto_approve_enabled = codex_auto_approve_enabled();
    let approval_mode = if protocol == "app-server" {
        Some(codex_approval_channel_mode())
    } else {
        None
    };

    let mut final_args = args.to_vec();
    if session_id.is_some() {
        final_args.insert(0, "resume".to_string());
        final_args.insert(0, "exec".to_string());
        if let Some(session_id) = session_id {
            final_args.push(session_id.to_string());
        }
    } else if final_args.first().map(|s| s.as_str()) != Some("exec") {
        final_args.insert(0, "exec".to_string());
    }
    if !has_any_arg(&final_args, &["--json"]) {
        final_args.push("--json".to_string());
    }

    let should_use_full_auto = auto_approve_enabled
        && approval_mode
            .as_ref()
            .map(|mode| matches!(mode, ApprovalChannelMode::FullAutoFallback))
            .unwrap_or(true);
    // Yolo (default): no sandbox, no approval prompts, full FS access — so codex can
    // write outside cwd (e.g. the sibling project a consult is about). Opt out with
    // TRIUMVIRATE_CODEX_SANDBOX=1, which leaves the --full-auto preset below in force.
    if codex_yolo_enabled() && !caps.args_include_explicit_policy(&final_args) {
        final_args.push("--dangerously-bypass-approvals-and-sandbox".to_string());
    }

    // codex 0.121+ rejects `--full-auto` combined with any other approval /
    // sandbox policy flag. The authoritative list lives on the probed
    // `CodexCapabilities` so it can evolve with the upstream CLI without a
    // scattered-grep refactor.
    //
    // 0.145 DEPRECATED `--full-auto` (it warns per call and a future release removes it). Inject
    // the explicit equivalent it resolves to instead — workspace-write sandbox + no approval
    // prompts — which is stable across the deprecation. Same gate: a user-supplied policy flag wins.
    let explicit_approval_policy = caps.args_include_explicit_policy(&final_args);
    if should_use_full_auto
        && !has_any_arg(&final_args, &["--full-auto", "--sandbox"])
        && !explicit_approval_policy
    {
        final_args.push("--sandbox".to_string());
        final_args.push("workspace-write".to_string());
        final_args.push("--ask-for-approval".to_string());
        final_args.push("never".to_string());
    }

    if !is_git_worktree(cwd) && !has_any_arg(&final_args, &["--skip-git-repo-check"]) {
        final_args.push("--skip-git-repo-check".to_string());
    }

    let output_file = std::env::temp_dir().join(format!(
        "triumvirate-codex-last-message-{}.txt",
        Uuid::new_v4()
    ));
    final_args.push("--output-last-message".to_string());
    final_args.push(output_file.display().to_string());
    final_args.push(message.to_string());

    let mut command = Command::new(bin);
    command
        .args(&final_args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    configure_process_group(&mut command);
    let mut child = command.spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("codex stdout missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("codex stderr missing"))?;

    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                tracing::debug!("codex stderr: {trimmed}");
            }
        }
    });

    let mut parser = CodexExecParser::new();
    let mut app_server_parser = CodexAppServerParser::new();
    let mut app_server_approval_requests: Vec<String> = Vec::new();
    let mut reader = BufReader::new(stdout).lines();
    let mut raw_output = String::new();
    let timeout_duration = connector_timeout();
    let read = async {
        while let Some(line) = reader.next_line().await? {
            raw_output.push_str(&line);
            raw_output.push('\n');
            if protocol == "app-server" {
                if let Some(event) = app_server_parser.parse_event_line(&line)? {
                    match event {
                        CodexAppServerEvent::Working(event) => {
                            emit_working_event(events_tx.as_ref(), event);
                        }
                        CodexAppServerEvent::ApprovalRequest(request) => {
                            let action = request
                                .reason
                                .clone()
                                .or(request.id.clone())
                                .unwrap_or_else(|| "unknown".to_string());
                            app_server_approval_requests.push(action.clone());
                            if auto_approve_enabled {
                                emit_working_event(
                                    events_tx.as_ref(),
                                    WorkingStateEvent {
                                        agent: "codex".to_string(),
                                        state: WorkingState::ToolCallCompleted,
                                        detail: format!(
                                            "auto-approved codex action ({})",
                                            approval_mode
                                                .as_ref()
                                                .map(|mode| match mode {
                                                    ApprovalChannelMode::ProceedOnce => "ProceedOnce",
                                                    ApprovalChannelMode::FullAutoFallback => "--full-auto",
                                                })
                                                .unwrap_or("--full-auto")
                                        ),
                                        tool_name: Some("approval_request".to_string()),
                                        tool_args_json: Some(format!(
                                            "{{\"action\":{}}}",
                                            serde_json::to_string(&action)
                                                .unwrap_or_else(|_| "\"unknown\"".to_string())
                                        )),
                                        token_usage: None,
                                        ts_ms: Some(core_unix_time_ms()),
                                    },
                                );
                            } else {
                                emit_working_event(
                                    events_tx.as_ref(),
                                    WorkingStateEvent {
                                        agent: "codex".to_string(),
                                        state: WorkingState::InputRequested,
                                        detail: format!("approval requested: {action}"),
                                        tool_name: Some("approval_request".to_string()),
                                        tool_args_json: request
                                            .id
                                            .as_ref()
                                            .map(|id| format!("{{\"id\":\"{id}\"}}")),
                                        token_usage: None,
                                        ts_ms: Some(core_unix_time_ms()),
                                    },
                                );
                            }
                        }
                    }
                }
            } else if let Some(event) = parser.parse_line(&line) {
                emit_working_event(events_tx.as_ref(), event);
            }
        }
        anyhow::Ok(())
    };
    match timeout(timeout_duration, read).await {
        Ok(result) => result?,
        Err(_) => {
            kill_process_group(&mut child);
            let _ = child.kill().await;
            let _ = child.wait().await;
            anyhow::bail!("codex connector timed out");
        }
    }
    let status = child.wait().await?;

    let last_message = fs::read_to_string(&output_file).unwrap_or_default();
    let _ = fs::remove_file(&output_file);

    if !status.success() {
        anyhow::bail!("codex connector failed: exited with status {status}");
    }

    let mut parsed = if protocol == "app-server" {
        app_server_parser.finish()
    } else {
        parser.finish()
    };
    if !last_message.trim().is_empty() {
        parsed.response_text = last_message.trim().to_string();
    } else if parsed.response_text.trim().is_empty() {
        if let Some(text) = extract_text_from_jsonl(&raw_output) {
            parsed.response_text = text;
        } else {
            parsed.response_text = raw_output.trim().to_string();
        }
    }
    if parsed.session_id.is_none() {
        parsed.session_id = session_id.map(ToString::to_string);
    }

    if should_use_full_auto {
        record_auto_approved_action(cwd, "global --full-auto", "--full-auto");
    } else if auto_approve_enabled && matches!(approval_mode, Some(ApprovalChannelMode::ProceedOnce)) {
        for action in app_server_approval_requests {
            record_auto_approved_action(cwd, &action, "ProceedOnce");
        }
    }

    Ok(parsed)
}

/// Spawn the grok CLI for one turn. REQ-GROK-004/013/016.
///
/// Single attempt by design (REQ-GROK-013): every turn re-ships the whole system prompt and tool
/// schemas, so a retry is not a cheap repeat of a small question.
/// The outer retry schedule, extracted from `execute_ask_agent` so it can be tested directly.
///
/// It used to live inline, and its "test" defined a private `schedule_len_for` closure and
/// asserted against that closure, so it passed no matter what this code did. Its own comment
/// admitted it was "a minimal reconstruction". A test that cannot fail is not a gate, so the
/// logic moved here and the test now calls this function.
///
/// Single attempt means the runner owns its own retries and the outer loop must not double them:
///   - `gemini` on the agy backend: agy runs its own internal retry (REQ-013).
///   - `deepseek`: the runner owns scoped in-flight retries (REQ-DS-008), and an outer retry
///     would double-bill on a 429.
///   - `grok`: REQ-GROK-013. Every turn re-ships the entire system prompt and all tool schemas,
///     so a retry is never a cheap repeat of a small question.
pub(crate) fn attempt_schedule_for(
    agent: &str,
    gemini_backend_selected: Option<GeminiBackend>,
) -> Vec<(Duration, Option<&'static str>)> {
    if agent == "gemini" {
        if matches!(gemini_backend_selected, Some(GeminiBackend::Agy)) {
            vec![(Duration::ZERO, None)]
        } else {
            GEMINI_MODEL_FAILDOWN
                .iter()
                .enumerate()
                .map(|(i, model)| {
                    let backoff = if i == 0 { Duration::ZERO } else { Duration::from_millis(500) };
                    (backoff, Some(*model))
                })
                .collect()
        }
    } else if agent == "deepseek" || agent == "grok" {
        vec![(Duration::ZERO, None)]
    } else {
        vec![
            (Duration::from_millis(250), None),
            (Duration::from_secs(1), None),
            (Duration::from_secs(2), None),
        ]
    }
}

/// Does this grok stderr line mean the run is NOT contained?
///
/// Found by Grok reviewing its own adapter. The first version matched one lowercase string,
/// `"sandbox could not be applied"`. The binary's actual fail-open path emits
/// **`"Sandbox could not be applied, continuing without sandbox"`** with a capital S, so the
/// guard missed precisely the case it existed for: the one where grok keeps going with no
/// containment. Every other match was a case where grok already refuses on its own.
///
/// Strings verified against `grok 1.0.13`. Matching is case-insensitive and substring-based
/// because these are human-facing warnings, not a stable API.
fn is_grok_containment_failure(line: &str) -> bool {
    const MARKERS: &[&str] = &[
        // The dangerous one: grok continues, uncontained.
        "continuing without sandbox",
        "defaulting to no sandbox",
        // The profile did not apply. Grok may refuse on its own, but never rely on that.
        "sandbox could not be applied",
        "could not apply the",
        "sandbox initialization failed",
        "sandbox_init returned error code",
        // Write-deny hooks are part of containment; if they fail, writes are not denied.
        "hook write-deny ensure failed",
    ];
    let lower = line.to_lowercase();
    MARKERS.iter().any(|m| lower.contains(m))
}

async fn run_grok_cli_process_with_session(
    bin: &str,
    args: &[String],
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
    events_tx: Option<mpsc::Sender<WorkingStateEvent>>,
) -> anyhow::Result<ParsedAgentResult> {
    // A session id means "resume": it is only ever populated from a previous turn's parsed
    // `end.sessionId`. The builder refuses to emit a bare `--resume`, which would silently
    // attach to the most recent session in this cwd.
    // Turn 1 must MINT an id, not run anonymously.
    //
    // Grok caught this reviewing its own adapter: the runner only ever passed an id when
    // resuming, so a first turn emitted neither `--session-id` nor `--resume`. If that turn
    // produced text but died before its `end` event, `parsed.session_id` stayed None and the
    // next call silently began a NEW conversation instead of resuming. Generating the id here
    // means the session exists on disk under a known id even when the turn is cut short.
    let resume = session_id.is_some_and(|s| !s.trim().is_empty());
    let minted = if resume { None } else { Some(Uuid::new_v4().to_string()) };
    let effective_session = if resume { session_id } else { minted.as_deref() };
    let invocation = mcp_bridge::grok::build_grok_invocation(
        bin, args, message, cwd, effective_session, resume,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut command = Command::new(&invocation.program);
    command
        .args(&invocation.args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    configure_process_group(&mut command);
    let mut child = command.spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("grok stdout missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("grok stderr missing"))?;

    // stderr is not debug noise for grok: it is where containment failure is reported. Some
    // paths make grok refuse on its own, but at least one prints
    // "Sandbox could not be applied, continuing without sandbox" and KEEPS GOING. A silently
    // uncontained consult is worse than a refused one, so any such line is fatal here. See
    // `is_grok_containment_failure` for the verified string set.
    let stderr_task = tokio::spawn(async move {
        let mut sandbox_warning: Option<String> = None;
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if is_grok_containment_failure(trimmed) {
                sandbox_warning.get_or_insert_with(|| trimmed.to_string());
            }
            tracing::debug!("grok stderr: {trimmed}");
        }
        sandbox_warning
    });

    let verbosity = mcp_bridge::agent_verbosity();
    let mut parser = GrokStreamParser::new();
    let mut reader = BufReader::new(stdout).lines();
    let mut raw_output = String::new();

    let read = async {
        while let Some(line) = reader.next_line().await? {
            raw_output.push_str(&line);
            raw_output.push('\n');
            if let Some(event) = parser.parse_line(&line)
                && should_display(&event.state, verbosity)
            {
                emit_working_event(events_tx.as_ref(), event);
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    match tokio::time::timeout(invocation.timeout, read).await {
        Ok(result) => result?,
        Err(_) => {
            kill_process_group(&mut child);
            let _ = child.kill().await;
            let _ = child.wait().await;
            anyhow::bail!(
                "grok exceeded TRIUMVIRATE_GROK_CONNECTOR_TIMEOUT_SECS ({}s)",
                invocation.timeout.as_secs()
            );
        }
    }
    let status = child.wait().await?;

    // Containment check before anything else. If the sandbox did not apply, the run happened
    // with full filesystem access regardless of how well it went.
    //
    // AWAIT the drain first. The original read a shared Mutex right after `child.wait()`, but
    // the child exiting does not mean the reader task has consumed the last stderr line, so the
    // warning could be missed and an uncontained run reported as clean. Codex caught this.
    let sandbox_warning = stderr_task.await.unwrap_or(None);
    if let Some(warning) = sandbox_warning {
        anyhow::bail!(
            "grok ran WITHOUT the requested sandbox: {warning}. Refusing the result rather than \
             reporting an uncontained run as a normal consult. Set TRIUMVIRATE_GROK_SANDBOX to a \
             profile that exists (workspace, read-only, strict) or to `off` to disable containment."
        );
    }

    let full = parser.finish_full();
    let mut parsed = full.parsed;

    // Batch fallback for TRIUMVIRATE_GROK_STREAMING=0, where stdout is one JSON object.
    if !mcp_bridge::grok::grok_streaming_enabled()
        && parsed.response_text.trim().is_empty()
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(raw_output.trim())
    {
        parsed = GrokStreamParser::parse_batch_json(&v);
    }

    // REQ-GROK-016: classify auth distinctly. An operator sent to the wrong fix wastes a cycle.
    if !status.success() {
        let detail = full.error_detail.clone().unwrap_or_default();
        let haystack = format!("{detail} {raw_output}").to_lowercase();
        if haystack.contains("unauthorized")
            || haystack.contains("401")
            || haystack.contains("auth")
            || haystack.contains("login")
        {
            anyhow::bail!(
                "grok auth failed: run `grok login --oauth` for a SuperGrok subscription, or set \
                 XAI_API_KEY for metered API access"
            );
        }
        if parsed.response_text.trim().is_empty() {
            anyhow::bail!("grok exited with status {status} and produced no text");
        }
    }

    // Auto-compaction rewrites the conversation mid-turn. If it FAILED, the answer was produced
    // against a context that was being rewritten and did not survive, so neither side can vouch
    // for what the model actually saw. Mark it rather than hand back a clean-looking answer.
    if let Some(detail) = full.context_rewrite_failed.as_deref() {
        tracing::warn!(agent = "grok", detail, "grok auto-compaction failed mid-turn");
        if parsed.response_text.trim().is_empty() {
            anyhow::bail!("grok auto-compaction failed ({detail}) and the turn produced no text");
        }
        parsed.response_text = format!(
            "[UNRELIABLE: grok's auto-compaction failed mid-turn ({detail}); this answer was \
             produced against a context that was being rewritten.]\n\n{}",
            parsed.response_text
        );
    }

    if let Some((tools, commands)) = full.tool_surface {
        // The only visibility into per-turn context cost: 26 tools is a ~14K turn, 420 is ~67K.
        tracing::info!(agent = "grok", tools, commands, "grok tool surface for this turn");
    }

    // Termination policy lives HERE, not in the parser, so it is testable independently.
    match full.termination {
        GrokTermination::MaxTurnsReached => {
            // Return the partial answer, MARKED, rather than discarding it.
            //
            // The first version bailed. Codex argued the asymmetry: with a marked partial a
            // caller can display it, persist it, retry with it as context, or reject it. With an
            // Err the caller can only report failure, and output that was already generated and
            // paid for is unrecoverable through this API. So mark, do not discard.
            let turns = mcp_bridge::grok::grok_max_turns();
            if parsed.response_text.trim().is_empty() {
                anyhow::bail!(
                    "grok hit --max-turns ({turns}) with no text at all. Raise \
                     TRIUMVIRATE_GROK_MAX_TURNS or narrow the prompt"
                );
            }
            tracing::warn!(turns, agent = "grok", "grok hit max-turns; returning a marked partial");
            parsed.response_text = format!(
                "[INCOMPLETE: grok hit --max-turns ({turns}); this answer is PARTIAL. Raise \
                 TRIUMVIRATE_GROK_MAX_TURNS or narrow the prompt.]\n\n{}",
                parsed.response_text
            );
        }
        GrokTermination::Stopped => {
            // grok exits 0 and emits a well-formed `end` for these, so only stopReason reveals
            // them. Reporting a refusal or a token-truncated answer as complete is the failure
            // this branch exists to prevent.
            let why = full.stop_reason.clone().unwrap_or_else(|| "unknown".to_string());
            if parsed.response_text.trim().is_empty() {
                anyhow::bail!("grok stopped early ({why}) and produced no text");
            }
            tracing::warn!(stop_reason = %why, agent = "grok", "grok stopped early; marking the answer");
            parsed.response_text = format!(
                "[INCOMPLETE: grok stopped with stopReason={why}; this answer may be truncated or refused.]\n\n{}",
                parsed.response_text
            );
        }
        GrokTermination::Errored => {
            anyhow::bail!(
                "grok reported an error: {}",
                full.error_detail.unwrap_or_else(|| "no detail".to_string())
            );
        }
        GrokTermination::Incomplete if parsed.response_text.trim().is_empty() => {
            anyhow::bail!("grok produced no `end` event and no text; the process died mid-turn");
        }
        _ => {}
    }

    // REQ-GROK-007: trust the parser's id. If it differs from what we asked for, the server
    // chose, and OUR record must follow or the next resume targets a session that never existed.
    if parsed.session_id.is_none() {
        // Fall back to the id we actually passed, minted or resumed, so a turn cut short before
        // `end` is still resumable rather than orphaned.
        parsed.session_id = effective_session.map(ToString::to_string);
    } else if let (Some(got), Some(want)) = (parsed.session_id.as_deref(), effective_session)
        && !want.trim().is_empty()
        && !got.eq_ignore_ascii_case(want.trim())
    {
        tracing::warn!(
            requested = %want, returned = %got,
            "grok returned a different sessionId than requested; trusting the parser"
        );
    }

    if let Some(cost) = full.total_cost_usd {
        tracing::info!(agent = "grok", cost_usd = cost, "grok turn cost");
    }

    Ok(parsed)
}

async fn run_claude_cli_process_with_session(
    bin: &str,
    args: &[String],
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
    events_tx: Option<mpsc::Sender<WorkingStateEvent>>,
) -> anyhow::Result<ParsedAgentResult> {
    let mut final_args = args.to_vec();
    final_args.push("-p".to_string());
    final_args.push(message.to_string());

    if let Some(events_tx) = events_tx.as_ref() {
        emit_working_event(
            Some(events_tx),
            WorkingStateEvent {
                agent: "claude".to_string(),
                state: WorkingState::TurnStarted,
                detail: "claude process started".to_string(),
                tool_name: None,
                tool_args_json: None,
                token_usage: None,
                ts_ms: Some(core_unix_time_ms()),
            },
        );
    }

    let mut command = Command::new(bin);
    command
        .args(&final_args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    
    configure_process_group(&mut command);
    let mut child = command.spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("claude stdout missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("claude stderr missing"))?;

    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                tracing::debug!("claude stderr: {trimmed}");
            }
        }
    });

    let mut reader = BufReader::new(stdout).lines();
    let mut raw_output = String::new();
    let timeout_duration = connector_timeout();

    let read = async {
        while let Some(line) = reader.next_line().await? {
            raw_output.push_str(&line);
            raw_output.push('\n');
            if let Some(events_tx) = events_tx.as_ref() {
                emit_working_event(
                    Some(events_tx),
                    WorkingStateEvent {
                        agent: "claude".to_string(),
                        state: WorkingState::MessageDelta,
                        detail: line.to_string(),
                        tool_name: None,
                        tool_args_json: None,
                        token_usage: None,
                        ts_ms: Some(core_unix_time_ms()),
                    },
                );
            }
        }
        anyhow::Ok(())
    };

    match timeout(timeout_duration, read).await {
        Ok(result) => result?,
        Err(_) => {
            kill_process_group(&mut child);
            let _ = child.kill().await;
            let _ = child.wait().await;
            anyhow::bail!("claude connector timed out");
        }
    }
    let status = child.wait().await?;

    if !status.success() {
        anyhow::bail!("claude connector failed: exited with status {status}");
    }

    let mut parsed = ParsedAgentResult {
        response_text: raw_output.trim().to_string(),
        ..Default::default()
    };
    if parsed.session_id.is_none() {
        parsed.session_id = session_id.map(ToString::to_string);
    }

    Ok(parsed)
}

async fn run_agent_process_with_session(
    agent: &str,
    bin: &str,
    args: &[String],
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
    events_tx: Option<mpsc::Sender<WorkingStateEvent>>,
) -> anyhow::Result<ParsedAgentResult> {
    if is_mock_connector(bin) {
        #[cfg(test)]
        {
            let _ = cwd;
            return run_mock_connector_process(agent, bin, args, message, session_id).await;
        }
        #[cfg(not(test))]
        {
            anyhow::bail!(
                "mock connectors are test-only; set TRIUMVIRATE_{}_BIN to a real CLI binary",
                agent.to_uppercase()
            );
        }
    }

    match agent {
        // REQ-001/003/005: the backend selector lives at this seam. The public agent
        // name stays `gemini` (C3); only the executing CLI changes. The gemini-cli path
        // is kept verbatim for rollback and the degraded route.
        "gemini" => match gemini_backend() {
            GeminiBackend::Agy => {
                let (agy_bin, agy_args) = agy_command();
                tracing::info!(backend = "agy", "gemini dispatch served by agy backend");
                crate::agy::run_agy_cli_process_with_session(
                    &agy_bin, &agy_args, message, cwd, session_id, events_tx,
                )
                .await
            }
            GeminiBackend::GeminiCli => {
                tracing::info!(backend = "gemini-cli", "gemini dispatch served by gemini-cli backend");
                run_gemini_cli_process_with_session(bin, args, message, cwd, session_id, events_tx)
                    .await
            }
        },
        "codex" => {
            run_codex_cli_process_with_session(bin, args, message, cwd, session_id, events_tx)
                .await
        }
        "grok" => {
            run_grok_cli_process_with_session(bin, args, message, cwd, session_id, events_tx).await
        }
        "claude" => {
            run_claude_cli_process_with_session(bin, args, message, cwd, session_id, events_tx)
                .await
        }
        _ => anyhow::bail!("unsupported agent: {agent}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// T-012 (REQ-DS-014, REQ-DS-023) tests — dispatch arm + CoT bifurcation.
//
// These tests use the testable inner `run_deepseek_with_runtime` so the
// process-static OnceLock cache stays untouched. The mock server is a
// scripted TCP responder.
// ─────────────────────────────────────────────────────────────────────────────
// clippy::await_holding_lock fires on the env guards below. Holding them across the await is
// DELIBERATE: these tests mutate process-global env, and the lock has to span the whole test,
// including the awaited call, or a parallel test changes the env mid-flight. Dropping it before
// the await would reintroduce exactly the race the lock exists to prevent. Test-only; no
// production path holds a std Mutex across an await.
#[allow(clippy::await_holding_lock)]
#[cfg(test)]
mod deepseek_dispatch_tests {
    use super::*;
    use mcp_bridge::deepseek as ds;
    use mcp_bridge::deepseek_config::{ApiKey, DeepSeekConfig, ReasoningEffort, ThinkingMode};
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    async fn spawn_scripted_server(scripts: Vec<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let url = format!("http://{}", addr);
        tokio::spawn(async move {
            for script in scripts {
                let (mut sock, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => return,
                };
                let mut buf = [0u8; 8192];
                let _ = tokio::time::timeout(
                    Duration::from_millis(200),
                    tokio::io::AsyncReadExt::read(&mut sock, &mut buf),
                )
                .await;
                let _ = sock.write_all(&script).await;
                let _ = sock.flush().await;
                let _ = sock.shutdown().await;
            }
        });
        url
    }

    fn happy_sse_body() -> String {
        let reasoning = r#"data: {"id":"chatcmpl-T012","object":"chat.completion.chunk","model":"deepseek-v4-pro","system_fingerprint":"fp_T012","choices":[{"index":0,"delta":{"reasoning_content":"the model is thinking carefully"},"finish_reason":null}]}"#;
        let content = r#"data: {"id":"chatcmpl-T012","choices":[{"index":0,"delta":{"content":"clean final answer"},"finish_reason":null}]}"#;
        let usage = r#"data: {"id":"chatcmpl-T012","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":18,"completion_tokens":174,"prompt_cache_hit_tokens":0,"prompt_cache_miss_tokens":18}}"#;
        format!("{reasoning}\n\n{content}\n\n{usage}\n\ndata: [DONE]\n\n")
    }

    fn make_cfg(url: &str) -> DeepSeekConfig {
        DeepSeekConfig {
            base_url: url.to_string(),
            api_key: ApiKey::new("sk-test-dispatch"),
            model: "deepseek-v4-pro".to_string(),
            max_tokens: 1024,
            thinking: ThinkingMode::Enabled,
            reasoning_effort: ReasoningEffort::High,
            read_timeout: Duration::from_secs(5),
            timeout: Duration::from_secs(10),
            tcp_keepalive: Duration::from_secs(30),
            max_concurrent: 4,
            max_rpm: 60,
            reasoning_cap_tokens: 0,
            log_dir: std::env::temp_dir().join("deepseek-t012-test"),
            log_reasoning_cap_bytes: 262_144,
            bulk_bytes: 16_384,
        }
    }

    /// Reality test (a): default CoT bifurcation. Response carries content
    /// ONLY; reasoning is captured to the per-request log file.
    #[tokio::test]
    async fn deepseek_dispatch_default_response_is_content_only() {
        let body = happy_sse_body();
        let script = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        ).into_bytes();
        let url = spawn_scripted_server(vec![script]).await;

        let mut cfg = make_cfg(&url);
        let log_dir = tempfile::tempdir().expect("tempdir");
        cfg.log_dir = log_dir.path().to_path_buf();
        let client = ds::build_client(&cfg).expect("client");
        let resilience = ds::ResilienceState::from_cfg(&cfg);

        let req = shared_types::AskAgentRequest {
            agent: "deepseek".to_string(),
            message: "what is 2+2".to_string(),
            ..Default::default()
        };
        let parsed = run_deepseek_with_runtime(&cfg, &client, &resilience, &req.message, Some(&req))
            .await
            .expect("dispatch ok");

        assert_eq!(parsed.response_text, "clean final answer");
        assert!(
            !parsed.response_text.contains("the model is thinking"),
            "default response MUST NOT contain reasoning"
        );
        let sid = parsed.session_id.expect("session_id populated");
        assert!(sid.starts_with("deepseek-"), "session_id={sid}");

        let entries: Vec<_> = std::fs::read_dir(log_dir.path())
            .expect("readdir")
            .filter_map(|e| e.ok())
            .collect();
        assert!(!entries.is_empty(), "per-request log must be written");
        let log_body = std::fs::read_to_string(entries[0].path()).expect("read log");
        assert!(
            log_body.contains("the model is thinking carefully"),
            "reasoning must be persisted to the log"
        );
    }

    /// Reality test (b): include_reasoning=true → reasoning in response.
    #[tokio::test]
    async fn deepseek_dispatch_include_reasoning_true_returns_reasoning_in_response() {
        let body = happy_sse_body();
        let script = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        ).into_bytes();
        let url = spawn_scripted_server(vec![script]).await;

        let mut cfg = make_cfg(&url);
        let log_dir = tempfile::tempdir().expect("tempdir");
        cfg.log_dir = log_dir.path().to_path_buf();
        let client = ds::build_client(&cfg).expect("client");
        let resilience = ds::ResilienceState::from_cfg(&cfg);

        let req = shared_types::AskAgentRequest {
            agent: "deepseek".to_string(),
            message: "x".to_string(),
            deepseek_include_reasoning: Some(true),
            ..Default::default()
        };
        let parsed = run_deepseek_with_runtime(&cfg, &client, &resilience, &req.message, Some(&req))
            .await
            .expect("dispatch ok");

        assert!(parsed.response_text.contains("the model is thinking carefully"),
            "include_reasoning=true must surface reasoning; got: {}", parsed.response_text);
        assert!(parsed.response_text.contains("clean final answer"));
        assert!(parsed.response_text.contains("<reasoning>"));
    }

    /// cfg_with_overrides — every Effort variant collapses correctly.
    #[test]
    fn cfg_with_overrides_collapses_effort_levels() {
        use shared_types::{AskAgentRequest, DeepSeekEffort, DeepSeekThinking};
        let base = make_cfg("http://unused");

        for (in_effort, want) in &[
            (DeepSeekEffort::Low, ReasoningEffort::High),
            (DeepSeekEffort::Medium, ReasoningEffort::High),
            (DeepSeekEffort::High, ReasoningEffort::High),
            (DeepSeekEffort::Max, ReasoningEffort::Max),
            (DeepSeekEffort::Xhigh, ReasoningEffort::Max),
        ] {
            let req = AskAgentRequest {
                agent: "deepseek".to_string(),
                message: "x".to_string(),
                deepseek_reasoning_effort: Some(*in_effort),
                ..Default::default()
            };
            let cfg = cfg_with_overrides(&base, Some(&req));
            assert_eq!(cfg.reasoning_effort, *want, "effort={in_effort:?}");
        }

        let req = AskAgentRequest {
            agent: "deepseek".to_string(),
            message: "x".to_string(),
            deepseek_thinking: Some(DeepSeekThinking::Disabled),
            ..Default::default()
        };
        let cfg = cfg_with_overrides(&base, Some(&req));
        assert_eq!(cfg.thinking, ThinkingMode::Disabled);

        let req = AskAgentRequest {
            agent: "deepseek".to_string(),
            message: "x".to_string(),
            deepseek_max_tokens: Some(2048),
            ..Default::default()
        };
        let cfg = cfg_with_overrides(&base, Some(&req));
        assert_eq!(cfg.max_tokens, 2048);

        let cfg = cfg_with_overrides(&base, None);
        assert_eq!(cfg.reasoning_effort, base.reasoning_effort);
        assert_eq!(cfg.thinking, base.thinking);
        assert_eq!(cfg.max_tokens, base.max_tokens);
    }

    /// 2026-05-26 follow-up: per-call model override picks Pro/Flash without
    /// daemon restart. Empty strings are ignored (don't blank the configured
    /// default). None preserves the cfg default.
    #[test]
    fn cfg_with_overrides_picks_model_per_call() {
        use shared_types::AskAgentRequest;
        let base = make_cfg("http://unused"); // base.model = "deepseek-v4-pro"

        // None → cfg model unchanged.
        let cfg = cfg_with_overrides(&base, None);
        assert_eq!(cfg.model, "deepseek-v4-pro");

        // Some("deepseek-v4-flash") → swap to flash for this call.
        let req = AskAgentRequest {
            agent: "deepseek".to_string(),
            message: "x".to_string(),
            deepseek_model: Some("deepseek-v4-flash".to_string()),
            ..Default::default()
        };
        let cfg = cfg_with_overrides(&base, Some(&req));
        assert_eq!(cfg.model, "deepseek-v4-flash");

        // Empty string → ignored, default preserved (defensive against a
        // caller mistakenly sending "" instead of omitting the field).
        let req = AskAgentRequest {
            agent: "deepseek".to_string(),
            message: "x".to_string(),
            deepseek_model: Some("   ".to_string()),
            ..Default::default()
        };
        let cfg = cfg_with_overrides(&base, Some(&req));
        assert_eq!(
            cfg.model, "deepseek-v4-pro",
            "empty/whitespace deepseek_model must not blank the cfg default"
        );

        // Arbitrary string passes through — the API will reject unknown
        // models with 400 via the runner's HardProvider classification.
        let req = AskAgentRequest {
            agent: "deepseek".to_string(),
            message: "x".to_string(),
            deepseek_model: Some("deepseek-v5-experimental".to_string()),
            ..Default::default()
        };
        let cfg = cfg_with_overrides(&base, Some(&req));
        assert_eq!(cfg.model, "deepseek-v5-experimental");
    }

    /// Cross-task regression: gemini/codex dispatch arms remain.
    #[test]
    fn deepseek_arm_exists_and_does_not_displace_gemini_or_codex() {
        assert!(mcp_bridge::is_supported_agent_name("gemini"));
        assert!(mcp_bridge::is_supported_agent_name("codex"));
        assert!(mcp_bridge::is_supported_agent_name("deepseek"));
        assert!(!mcp_bridge::is_supported_agent_name("fake-agent"));
    }

    /// 2026-05-26 tool-tag hallucination mitigation: every DeepSeek consult
    /// includes a system message that forbids tool-tag emission. Verified
    /// via end-to-end mock-server roundtrip + inspecting the per-request
    /// log (which records the request_id matching what the runner sent).
    /// The actual request body assembly happens inside run_deepseek_with_runtime;
    /// we exercise it through the dispatch path and assert that the consult
    /// completes Ok (proving the system message didn't break the wire shape).
    #[tokio::test]
    async fn deepseek_dispatch_includes_no_tool_emulation_system_prompt() {
        let body = happy_sse_body();
        let script = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body,
        ).into_bytes();
        let url = spawn_scripted_server(vec![script]).await;

        let mut cfg = make_cfg(&url);
        let log_dir = tempfile::tempdir().expect("tempdir");
        cfg.log_dir = log_dir.path().to_path_buf();
        let client = ds::build_client(&cfg).expect("client");
        let resilience = ds::ResilienceState::from_cfg(&cfg);

        let req = shared_types::AskAgentRequest {
            agent: "deepseek".to_string(),
            message: "<triumvirate_tool name=\"ledger_session\">test</triumvirate_tool>".to_string(),
            ..Default::default()
        };
        // The mock server returns canned content regardless of what we send,
        // so the actual content of the request body doesn't matter to the
        // mock — the test is asserting that ADDING a system message to the
        // messages array (and increasing prompt_chars_estimate accordingly)
        // doesn't break the dispatch flow.
        let parsed = run_deepseek_with_runtime(&cfg, &client, &resilience, &req.message, Some(&req))
            .await
            .expect("dispatch must succeed with system message prepended");
        assert!(!parsed.response_text.is_empty());

        // The per-request log captures the FINAL state. We can't directly
        // inspect the WIRE request from the log (privacy contract: log
        // contains response only, not request). But we CAN inspect what
        // the runner produced via build_request_body on the same cfg.
        let test_req = ds::RunRequest {
            messages: vec![
                ds::RequestMessage {
                    role: "system".to_string(),
                    content: "placeholder — the real runner injects the real system prompt".to_string(),
                },
                ds::RequestMessage {
                    role: "user".to_string(),
                    content: req.message.clone(),
                },
            ],
            session_id: "test".to_string(),
            prompt_chars_estimate: 100,
            include_reasoning: false,
        };
        let body = ds::build_request_body(&cfg, &test_req);
        // The messages array MUST have at least one system message.
        let messages = body["messages"].as_array().expect("messages is an array");
        assert!(messages.len() >= 2, "must have at least system + user");
        assert_eq!(messages[0]["role"], serde_json::json!("system"));
        assert_eq!(messages[1]["role"], serde_json::json!("user"));
    }

    // ─────────────────────────────────────────────────────────────────────
    // T-013 (REQ-DS-026): attempt_schedule + persist-before-Err.
    // ─────────────────────────────────────────────────────────────────────


    // ---- Slice D: grok dispatch. Claude proves two dispatch layers can drift, so BOTH are
    // asserted here rather than trusting that adding one arm covered it. ----

    #[test]
    fn grok_is_reachable_through_the_public_gate() {
        assert!(mcp_bridge::is_supported_agent_name("grok"));
        assert!(mcp_bridge::is_supported_agent_name("supergrok"));
        assert_eq!(mcp_bridge::normalize_agent_name("SuperGrok"), "grok");
        assert_eq!(mcp_bridge::display_agent_name("grok"), "Grok");
    }

    /// Both dispatch matches must carry a grok arm. `claude` already demonstrated that one layer
    /// can gain an agent while the other does not, so this asserts on the source itself.
    #[test]
    fn both_dispatch_layers_have_a_grok_arm() {
        let src = include_str!("agent_exec.rs");
        let arms = src.matches("\"grok\" => {").count();
        assert!(arms >= 2,
            "expected a grok arm in BOTH run_named_agent_with_session_and_model and \
             run_agent_process_with_session, found {arms}");
    }

    /// Asserts against the REAL scheduler, `attempt_schedule_for`, not a reconstruction of it.
    ///
    /// The previous version of this test defined its own `schedule_len_for` closure and asserted
    /// against that closure, so it would have passed even if `execute_ask_agent` retried
    /// deepseek fifty times. Its own comment called it "a minimal reconstruction".
    #[test]
    fn attempt_schedule_is_single_for_metered_and_context_heavy_agents() {
        use super::attempt_schedule_for;
        // Single attempt: the runner owns its retries, or a retry is genuinely expensive.
        assert_eq!(attempt_schedule_for("deepseek", None).len(), 1,
            "deepseek MUST be single-attempt: an outer retry double-bills on 429 (REQ-DS-008)");
        assert_eq!(attempt_schedule_for("grok", None).len(), 1,
            "grok MUST be single-attempt: every turn re-ships the full system prompt (REQ-GROK-013)");
        assert_eq!(attempt_schedule_for("gemini", Some(super::GeminiBackend::Agy)).len(), 1,
            "agy runs its own internal retry (REQ-013)");

        // Everything else keeps the generic ladder. This is the regression guard: adding a
        // single-attempt agent must not silently convert the default.
        assert_eq!(attempt_schedule_for("codex", None).len(), 3);
        assert_eq!(attempt_schedule_for("claude", None).len(), 3);
        assert!(attempt_schedule_for("gemini", None).len() > 1,
            "gemini-cli uses the model faildown chain");
    }

    #[test]
    fn persist_deepseek_err_tokens_safe_with_either_usage_source() {
        use mcp_bridge::deepseek::{TokenUsage as DsTokenUsage, UsageSource as DsUsageSrc};
        let exact = DsTokenUsage {
            input_tokens: 18,
            output_tokens: 174,
            cached_tokens: 0,
            usage_source: DsUsageSrc::Exact,
        };
        persist_deepseek_err_tokens(
            "test-req-id-001",
            "deepseek-err-fallback-001",
            &exact,
            Some("chatcmpl-T013-exact"),
            &Some("/tmp".to_string()),
            &None,
        );

        let estimated = DsTokenUsage {
            input_tokens: 50,
            output_tokens: 200,
            cached_tokens: 0,
            usage_source: DsUsageSrc::Estimated,
        };
        persist_deepseek_err_tokens(
            "test-req-id-002",
            "deepseek-err-fallback-002",
            &estimated,
            None, // no ds_request_id → falls back to fallback_session_id
            &None,
            &None,
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // T-014 (REQ-DS-020): stateless single-turn.
    // ─────────────────────────────────────────────────────────────────────

    /// Reality test: two sequential DeepSeek consults produce DIFFERENT
    /// session_ids, both starting with "deepseek-". No resume token is
    /// constructed; the runner generates a fresh uuid per call.
    #[tokio::test]
    async fn deepseek_stateless_distinct_session_ids_across_calls() {
        let body = happy_sse_body();
        let s1 = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        ).into_bytes();
        let s2 = s1.clone();
        let url = spawn_scripted_server(vec![s1, s2]).await;

        let mut cfg = make_cfg(&url);
        let log_dir = tempfile::tempdir().expect("tempdir");
        cfg.log_dir = log_dir.path().to_path_buf();
        let client = ds::build_client(&cfg).expect("client");
        let resilience = ds::ResilienceState::from_cfg(&cfg);

        let req = shared_types::AskAgentRequest {
            agent: "deepseek".to_string(),
            message: "x".to_string(),
            ..Default::default()
        };

        let r1 = run_deepseek_with_runtime(&cfg, &client, &resilience, &req.message, Some(&req))
            .await
            .expect("call 1 ok");
        let r2 = run_deepseek_with_runtime(&cfg, &client, &resilience, &req.message, Some(&req))
            .await
            .expect("call 2 ok");

        let s1 = r1.session_id.expect("s1");
        let s2 = r2.session_id.expect("s2");
        assert!(s1.starts_with("deepseek-"));
        assert!(s2.starts_with("deepseek-"));
        assert_ne!(s1, s2, "each consult must mint a fresh uuid; got the same");
    }

    // ─────────────────────────────────────────────────────────────────────
    // T-015 (REQ-DS-011, REQ-DS-012, REQ-DS-025): anti-bulk + entry guards.
    // ─────────────────────────────────────────────────────────────────────

    // Tests that mutate TRIUMVIRATE_DEEPSEEK_BULK_BYTES share this lock so
    // they don't race each other (cargo test runs in parallel by default).
    static BULK_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Reality test (a): DeepSeek payload over the cap rejects with an error
    /// message containing both "payload too large" AND "metered". The check
    /// runs at the very top of execute_ask_agent — we exercise it via the
    /// public surface using a unit-style helper invocation.
    #[tokio::test]
    async fn deepseek_anti_bulk_rejects_oversized_payload() {
        let _g = BULK_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Clear any env override so the default 16KB applies.
        unsafe { std::env::remove_var("TRIUMVIRATE_DEEPSEEK_BULK_BYTES"); }
        let big = "x".repeat(20_000);
        let req = shared_types::AskAgentRequest {
            agent: "deepseek".to_string(),
            message: big,
            ..Default::default()
        };
        let err = execute_ask_agent(&req, None)
            .await
            .expect_err("oversized deepseek payload must Err");
        let lower = err.to_lowercase();
        assert!(lower.contains("payload too large"), "missing 'payload too large': {err}");
        assert!(lower.contains("metered"), "missing 'metered': {err}");
    }

    /// Reality test (b): Gemini accepts the same size. The intercept is
    /// AGENT-GATED — gemini/codex are local CLIs so bulk is free.
    ///
    /// We assert by source-grep that the intercept is wrapped in
    /// `if agent == "deepseek"`. A runtime test would require gemini-cli
    /// binaries; the structural test is what the IMPL_PLAN's regression-guard
    /// pattern calls for.
    #[test]
    fn deepseek_anti_bulk_does_not_apply_to_gemini_or_codex() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("agent_exec.rs"),
        )
        .expect("read agent_exec.rs");

        let production_src = match src.find("mod deepseek_dispatch_tests") {
            Some(test_mod_start) => &src[..test_mod_start],
            None => &src[..],
        };

        // The intercept is a `req.message.len() > cap` check. Find the
        // production call site (not the test reference) and confirm the
        // preceding ~200 chars contain the deepseek gate.
        let needle = "req.message.len() > cap";
        let abs = production_src.find(needle).expect(
            "expected the anti-bulk intercept `req.message.len() > cap` in agent_exec.rs"
        );
        let lookback = &production_src[abs.saturating_sub(300)..abs];
        assert!(
            lookback.contains("if agent == \"deepseek\""),
            "anti-bulk intercept must be gated to agent==\"deepseek\""
        );
    }

    /// Reality test (c): the cap is env-configurable. Override
    /// TRIUMVIRATE_DEEPSEEK_BULK_BYTES=32768 and a 20KB payload no longer
    /// rejects at the intercept (it'll still proceed to the dispatch arm
    /// which fails because there's no real DeepSeek server in the test).
    #[tokio::test]
    async fn deepseek_anti_bulk_cap_is_env_configurable() {
        let _g = BULK_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        unsafe { std::env::set_var("TRIUMVIRATE_DEEPSEEK_BULK_BYTES", "32768"); }
        let big = "x".repeat(20_000);
        let req = shared_types::AskAgentRequest {
            agent: "deepseek".to_string(),
            message: big,
            ..Default::default()
        };
        let err = execute_ask_agent(&req, None).await.err();
        unsafe { std::env::remove_var("TRIUMVIRATE_DEEPSEEK_BULK_BYTES"); }
        // Whatever error surfaces, it must NOT be "payload too large".
        if let Some(msg) = err {
            assert!(
                !msg.to_lowercase().contains("payload too large"),
                "raised cap should not produce the anti-bulk error; got: {msg}"
            );
        }
        // err == None means it succeeded all the way through (improbable in
        // unit tests without a live DeepSeek server, but explicitly allowed).
    }

    /// Reality test (d) REQ-DS-011: NO auto-routing on the deepseek path.
    /// Grep for any router-keyed term landing inside a deepseek branch.
    #[test]
    fn deepseek_path_has_no_auto_routing() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("agent_exec.rs"),
        )
        .expect("read agent_exec.rs");

        let production_src = match src.find("mod deepseek_dispatch_tests") {
            Some(t) => &src[..t],
            None => &src[..],
        };

        // Find every `"deepseek"` arm/check in production code and verify the
        // surrounding ~500 chars don't contain "auto_route" or "router.route".
        let mut i = 0;
        while let Some(rel) = production_src[i..].find("\"deepseek\"") {
            let abs = i + rel;
            let start = abs.saturating_sub(500);
            let end = (abs + 500).min(production_src.len());
            let window = &production_src[start..end];
            assert!(
                !window.contains("auto_route") && !window.contains("router.route"),
                "REQ-DS-011 violation: auto-routing logic detected near deepseek branch at offset {abs}"
            );
            i = abs + "\"deepseek\"".len();
        }
    }

    /// Reality test (e) REQ-DS-012: no sandbox initialization on the
    /// deepseek path. The deepseek runner files MUST NOT call any sandbox
    /// init helper.
    #[test]
    fn deepseek_path_has_no_sandbox_init() {
        // Both mcp-bridge deepseek.rs (the runner) and the dispatch arm in
        // agent_exec.rs are in scope. The sandbox bring-up is keyed off
        // `sandbox_` symbols and `init_sandbox` in the rest of the daemon.
        let runner = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("mcp-bridge/src/deepseek.rs"),
        )
        .expect("read deepseek.rs");
        for forbidden in &["sandbox_init", "init_sandbox", "ensure_sandbox"] {
            assert!(
                !runner.contains(forbidden),
                "REQ-DS-012 violation: deepseek runner contains '{forbidden}'"
            );
        }
        // The dispatch arm (run_deepseek_agent in agent_exec.rs) similarly.
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("agent_exec.rs"),
        )
        .expect("read agent_exec.rs");
        let production = match src.find("mod deepseek_dispatch_tests") {
            Some(t) => &src[..t],
            None => &src[..],
        };
        let fn_start = production
            .find("async fn run_deepseek_agent(")
            .expect("run_deepseek_agent exists");
        // Find the matching close brace (rough — just scan to the next
        // top-level "async fn " or end of production_src).
        let next_fn = production[fn_start + 1..]
            .find("\nasync fn ")
            .or_else(|| production[fn_start + 1..].find("\nfn "))
            .map(|d| fn_start + 1 + d)
            .unwrap_or(production.len());
        let body = &production[fn_start..next_fn];
        for forbidden in &["sandbox_init", "init_sandbox", "ensure_sandbox"] {
            assert!(
                !body.contains(forbidden),
                "REQ-DS-012 violation: run_deepseek_agent contains '{forbidden}'"
            );
        }
    }

    /// prewarm_daemon_workers MUST NOT spawn a deepseek worker. We assert
    /// structurally — by source-grep — that the prewarm function only calls
    /// `prewarm_worker("gemini"...)` and `prewarm_worker("codex"...)`, not
    /// deepseek. (A runtime test would need an HTTP daemon spun up; the
    /// source-grep is the canary the IMPL_PLAN demands.)
    #[test]
    fn deepseek_prewarm_slot_is_a_safe_no_op() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("agent_exec.rs"),
        )
        .expect("read agent_exec.rs");

        // Restrict to the prewarm_daemon_workers function body.
        let fn_start = src
            .find("pub(crate) async fn prewarm_daemon_workers()")
            .expect("prewarm_daemon_workers exists");
        let body_start = src[fn_start..]
            .find('{')
            .expect("function body opens")
            + fn_start;
        // Find the matching closing brace.
        let bytes = src.as_bytes();
        let mut depth = 0i32;
        let mut body_end = body_start;
        // Iterate with the index so the brace scanner can record where the body ENDS, which is
        // the whole point; `enumerate` over a slice from body_start keeps clippy happy and keeps
        // the index absolute.
        for (i, b) in bytes.iter().enumerate().skip(body_start) {
            match *b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        body_end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &src[body_start..=body_end];
        assert!(
            body.contains("prewarm_worker(\"gemini\""),
            "prewarm must still spawn gemini"
        );
        assert!(
            body.contains("prewarm_worker(\"codex\""),
            "prewarm must still spawn codex"
        );
        assert!(
            !body.contains("prewarm_worker(\"deepseek\""),
            "prewarm MUST NOT spawn a deepseek worker (REQ-DS-020 — stateless)"
        );
    }

    /// Reality test (2): the Err-path persist is GATED to agent=='deepseek'.
    /// We assert this by source-grep on the file — a regression that
    /// removed the gate would land Gemini/Codex Err records in the token
    /// DB, which T-013's scope_out explicitly forbids.
    #[test]
    fn persist_deepseek_err_path_is_gated_to_deepseek_agent_only() {
        // The agent_exec.rs Err branch contains: `if agent == "deepseek"`
        // immediately before the downcast + persist call. Grep the file
        // and confirm that pattern is present AND that the persist call
        // appears INSIDE that conditional block.
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("agent_exec.rs"),
        )
        .expect("read agent_exec.rs");

        // Scope the check to PRODUCTION code only — the dispatch test mod
        // appears at the bottom of the file and contains its own helper
        // calls that don't need the gate. Using `mod deepseek_dispatch_tests`
        // as the boundary (rather than the first `#[cfg(test)]`) avoids
        // capturing mid-file test helpers (like run_mock_connector_process).
        let production_src = match src.find("mod deepseek_dispatch_tests") {
            Some(test_mod_start) => &src[..test_mod_start],
            None => &src[..],
        };

        let needle = "persist_deepseek_err_tokens(";
        let mut call_positions: Vec<usize> = Vec::new();
        let mut i = 0;
        while let Some(rel) = production_src[i..].find(needle) {
            let abs = i + rel;
            let prefix_start = abs.saturating_sub(40);
            let prefix = &production_src[prefix_start..abs];
            // Skip function definitions: the prefix ends with `fn ` (with any
            // trailing whitespace) when the next token is the function name.
            let prefix_trim = prefix.trim_end();
            if !prefix_trim.ends_with("fn") && !prefix_trim.ends_with("pub(crate) fn") {
                call_positions.push(abs);
            }
            i = abs + needle.len();
        }
        assert!(
            !call_positions.is_empty(),
            "expected at least one production CALL to persist_deepseek_err_tokens (the Err-branch persist hook)"
        );

        // Each call site MUST be preceded (within ~400 chars) by the gate
        // `if agent == \"deepseek\" {`. That structural guarantee is what
        // protects Gemini/Codex from accidentally landing a token record on
        // their Err paths.
        for &call_pos in &call_positions {
            let lookback_start = call_pos.saturating_sub(400);
            let lookback = &production_src[lookback_start..call_pos];
            assert!(
                lookback.contains("if agent == \"deepseek\""),
                "production persist call at offset {call_pos} is NOT preceded by an `if agent == \"deepseek\"` gate within 400 chars — regression hazard"
            );
        }
    }
}
