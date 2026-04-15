use crate::{
    TokenRecord, append_outbox_event, process_metrics, process_token_db, record_daemon_tokens,
    spawn_dead_drop,
};
use agent_adapter::{
    ApprovalChannelMode, CodexAppServerEvent, CodexAppServerParser, CodexExecParser,
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
    agent_verbosity, codex_command, codex_protocol, gemini_command, gemini_streaming_enabled,
    is_supported_agent,
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
fn configure_process_group(command: &mut Command) {
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
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn kill_process_group(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        let pgid = -(pid as i32);
        // SAFETY: kill is called with a process-group id derived from child pid.
        unsafe {
            let _ = libc::kill(pgid, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_process_group(_child: &mut tokio::process::Child) {}

fn emit_working_event(tx: Option<&mpsc::Sender<WorkingStateEvent>>, event: WorkingStateEvent) {
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
    };

    if let Err(err) = record_daemon_tokens(token_db.as_ref(), &record) {
        tracing::warn!("failed to record daemon token usage: {err}");
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
        agent.duration_ms = tracing::field::Empty
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

    if !is_supported_agent(req) {
        span.record("agent.outcome", "rejected");
        span.record("agent.tokens", 0_u64);
        span.record("agent.duration_ms", started.elapsed().as_millis() as u64);
        return Err("ask_agent supports only agent='gemini' or agent='codex'".to_string());
    }
    let agent = req.agent.to_lowercase();
    let request_id = Uuid::new_v4().to_string();
    let (resolved_cwd, resolved_repo, resolved_branch) =
        core_resolve_context(req.cwd.as_ref(), req.repo.as_ref(), req.branch.as_ref());
    let exec_cwd = resolved_cwd
        .clone()
        .unwrap_or_else(|| ".".to_string());
    let execution_prompt = inject_tool_marker_prompt(&req.message);
    let worker = acquire_worker(&agent, &exec_cwd).await;
    let mut worker_session_id = worker.session_id.clone();
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
            agent,
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
        detail: format!("{agent} is processing request"),
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

    // Build the attempt schedule: for gemini, use model faildown chain; for others, 3 retries.
    let attempt_schedule: Vec<(Duration, Option<&str>)> = if agent == "gemini" {
        GEMINI_MODEL_FAILDOWN
            .iter()
            .enumerate()
            .map(|(i, model)| {
                let backoff = if i == 0 {
                    Duration::ZERO
                } else {
                    Duration::from_millis(500)
                };
                (backoff, Some(*model))
            })
            .collect()
    } else {
        vec![
            (Duration::from_millis(250), None),
            (Duration::from_secs(1), None),
            (Duration::from_secs(2), None),
        ]
    };
    let verbosity = agent_verbosity();
    let mut last_err: Option<String> = None;

    for (idx, (backoff, model_override)) in attempt_schedule.iter().enumerate() {
        if let Some(model) = model_override {
            tracing::info!("faildown attempt {}/{}: trying model {model}", idx + 1, attempt_schedule.len());
            if let Some(emitter) = progress.as_ref() {
                emitter
                    .emit(format!("→ {agent_display}: trying model {model} ({}/{})...", idx + 1, attempt_schedule.len()))
                    .await;
            }
        }
        let session_for_attempt = if model_override.is_some() && idx > 0 {
            // Don't reuse cached sessions across model faildown — different models can't resume each other's sessions
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
                        let detail = format!("{agent} still working ({elapsed}s elapsed)");
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
                span.record("agent.outcome", "success");
                span.record("agent.tokens", tokens);
                span.record("agent.duration_ms", started.elapsed().as_millis() as u64);
                let next_session_id = parsed.session_id.clone();
                update_worker_session(&agent, &exec_cwd, next_session_id).await;
                span.record(
                    "session_id",
                    tracing::field::display(parsed.session_id.as_deref().unwrap_or("none")),
                );
                lifecycle.push(LifecycleEvent {
                    state: "DONE".to_string(),
                    detail: format!("{agent} responded on attempt {}", idx + 1),
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
                    return Err(err);
                }
                return Ok(AskAgentResponse {
                    request_id,
                    agent: agent.clone(),
                    response: parsed.response_text,
                    lifecycle,
                });
            }
            Err(e) => {
                let msg = e.to_string();
                if session_for_attempt.is_some() && should_invalidate_cached_session(&msg) {
                    worker_session_id = None;
                    update_worker_session(&agent, &exec_cwd, None).await;
                    let invalidated_detail = format!(
                        "{agent} session invalidated after stale resume ID; retrying with a fresh session"
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
                        detail: format!("{agent} timed out on attempt {}", idx + 1),
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
                last_err = Some(msg);
                sleep(*backoff).await;
            }
        }
    }

    lifecycle.push(LifecycleEvent {
        state: "FAILED".to_string(),
        detail: format!("{} failed after {} attempts", agent, attempt_schedule.len()),
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
    Err(format!(
        "ask_agent failed after lifecycle {:?}: {}{}",
        lifecycle
            .iter()
            .map(|e| e.state.as_str())
            .collect::<Vec<_>>(),
        last_err.unwrap_or_else(|| "unknown error".to_string()),
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
    run_named_agent_with_session_and_model(agent, message, cwd, session_id, events_tx, None).await
}

async fn run_named_agent_with_session_and_model(
    agent: &str,
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
    events_tx: Option<mpsc::Sender<WorkingStateEvent>>,
    model_override: Option<&str>,
) -> anyhow::Result<ParsedAgentResult> {
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
        _ => anyhow::bail!("unsupported agent: {agent}"),
    }
}

async fn prewarm_worker(agent: &str, cwd: &str) {
    let worker = acquire_worker(agent, cwd).await;
    let warm_prompt = "Prewarm this session. Reply with only: ready";
    let warm_result = timeout(
        daemon_prewarm_timeout(),
        run_named_agent_with_session(agent, warm_prompt, cwd, worker.session_id.as_deref(), None),
    )
    .await;

    match warm_result {
        Ok(Ok(parsed)) => {
            update_worker_session(agent, cwd, parsed.session_id).await;
            tracing::info!("prewarm complete for {agent} cwd={cwd}");
        }
        Ok(Err(err)) => {
            tracing::warn!("prewarm failed for {agent} cwd={cwd}: {err}");
            let _ = dismiss_worker(agent, cwd).await;
        }
        Err(_) => {
            tracing::warn!("prewarm timeout for {agent} cwd={cwd}");
            let _ = dismiss_worker(agent, cwd).await;
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
        .ok_or_else(|| anyhow::anyhow!("gemini stdout missing"))?;
    let mut stderr_reader = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("gemini stderr missing"))?;
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
        .ok_or_else(|| anyhow::anyhow!("gemini stdout missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("gemini stderr missing"))?;

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
                    anyhow::bail!("gemini 429 capacity error (fast-fail): {err_msg}");
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
            anyhow::bail!("gemini connector timed out");
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("gemini connector failed: exited with status {status}");
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
    .map_err(|_| anyhow::anyhow!("gemini connector timed out"))??;

    if !output.status.success() {
        anyhow::bail!("gemini connector failed: exited with status {}", output.status);
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
    let protocol = codex_protocol();
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
    if should_use_full_auto && !has_any_arg(&final_args, &["--full-auto"]) {
        final_args.push("--full-auto".to_string());
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
        "gemini" => {
            run_gemini_cli_process_with_session(bin, args, message, cwd, session_id, events_tx)
                .await
        }
        "codex" => {
            run_codex_cli_process_with_session(bin, args, message, cwd, session_id, events_tx)
                .await
        }
        _ => anyhow::bail!("unsupported agent: {agent}"),
    }
}
