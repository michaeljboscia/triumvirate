use crate::{append_outbox_event, spawn_dead_drop};
use agent_adapter::{
    CodexExecParser, GeminiStreamParser, ParsedAgentResult, StuckDetector,
    WorkingState, WorkingStateEvent, format_working_state, should_display,
};
use agent_worker::{
    WorkerAcquireMode, acquire_worker, dismiss_worker, should_invalidate_cached_session,
    update_worker_session,
};
use daemon_core::{resolve_context as core_resolve_context, unix_time_ms as core_unix_time_ms};
use mcp_bridge::{
    agent_verbosity, codex_command, gemini_command, gemini_streaming_enabled, is_supported_agent,
};
use mcp_tools::{ProgressEmitter, display_agent_name, next_heartbeat_offset};
use shared_types::{AskAgentRequest, AskAgentResponse, LifecycleEvent, OutboxEvent};
use std::{fs, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc,
    time::{Instant, sleep, timeout},
};
use tracing::{Span, instrument};
use uuid::Uuid;

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

#[instrument(
    name = "ask_agent",
    skip(req, progress),
    fields(
        agent.type = %req.agent,
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
    }) {
        tracing::warn!("failed to append outbox event: {e}");
    }
    if let Some(emitter) = progress.as_ref() {
        emitter.emit(format!("→ {agent_display}: working...")).await;
    }

    let backoffs = [Duration::from_millis(250), Duration::from_secs(1), Duration::from_secs(2)];
    let verbosity = agent_verbosity();
    let mut last_err: Option<String> = None;

    for (idx, backoff) in backoffs.iter().enumerate() {
        let session_for_attempt = worker_session_id.clone();
        let (events_tx, mut events_rx) = mpsc::channel::<WorkingStateEvent>(1024);
        let mut stuck_detector = StuckDetector::default();
        let mut attempt = Box::pin(run_named_agent_with_session(
            &agent,
            &execution_prompt,
            &exec_cwd,
            session_for_attempt.as_deref(),
            Some(events_tx),
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
                span.record("agent.outcome", "success");
                span.record("agent.tokens", tokens);
                span.record("agent.duration_ms", started.elapsed().as_millis() as u64);
                let next_session_id = parsed.session_id.clone();
                update_worker_session(&agent, &exec_cwd, next_session_id).await;
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
                }) {
                    tracing::warn!("failed to append outbox event: {e}");
                }
                if let Some(emitter) = progress.as_ref() {
                    emitter.emit(format!("→ {agent_display}: responded ✓")).await;
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
                lifecycle.push(LifecycleEvent {
                    state: "RETRY".to_string(),
                    detail: format!(
                        "Retrying {} ({}/{}) after {}",
                        agent,
                        idx + 1,
                        backoffs.len(),
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
                }) {
                    tracing::warn!("failed to append outbox event: {e}");
                }
                if let Some(emitter) = progress.as_ref() {
                    emitter
                        .emit(format!("→ {agent_display}: retrying ({}/{})...", idx + 1, backoffs.len()))
                        .await;
                }
                last_err = Some(msg);
                sleep(*backoff).await;
            }
        }
    }

    lifecycle.push(LifecycleEvent {
        state: "FAILED".to_string(),
        detail: format!("{} failed after {} attempts", agent, backoffs.len()),
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
    }) {
        tracing::warn!("failed to append outbox event: {e}");
    }
    if let Some(emitter) = progress.as_ref() {
        emitter
            .emit(format!("→ {agent_display}: FAILED after {} attempts", backoffs.len()))
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

async fn run_named_agent_with_session(
    agent: &str,
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
    events_tx: Option<mpsc::Sender<WorkingStateEvent>>,
) -> anyhow::Result<ParsedAgentResult> {
    match agent {
        "gemini" => {
            let (bin, args) = gemini_command();
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

    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                tracing::debug!("gemini stderr: {trimmed}");
            }
        }
    });

    let mut parser = GeminiStreamParser::new();
    let mut reader = BufReader::new(stdout).lines();
    let mut raw_output = String::new();
    let timeout_duration = connector_timeout();
    let read = async {
        while let Some(line) = reader.next_line().await? {
            raw_output.push_str(&line);
            raw_output.push('\n');
            if let Some(event) = parser.parse_line(&line) {
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
    let mut reader = BufReader::new(stdout).lines();
    let mut raw_output = String::new();
    let timeout_duration = connector_timeout();
    let read = async {
        while let Some(line) = reader.next_line().await? {
            raw_output.push_str(&line);
            raw_output.push('\n');
            if let Some(event) = parser.parse_line(&line) {
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

    let mut parsed = parser.finish();
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
