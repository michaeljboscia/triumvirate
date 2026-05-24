//! Antigravity CLI (`agy`) backend for the public `gemini` agent.
//!
//! Reality verified against the live binary (agy 1.0.1 + 1.0.2, macOS):
//! - **Subscription-OAuth only** — never an API key (C1). Auth is a plaintext file
//!   `~/.gemini/oauth_creds.json`; any process running as the user reads it.
//! - **Single-turn only** — no resumable multi-turn headless, so a passed-in
//!   `session_id` is ignored and `None` is returned; no resume/continue flags (REQ-040/042).
//! - **Pipe capture by default** — the non-TTY stdout-drop did NOT reproduce
//!   (7/7 on 1.0.1, clean on 1.0.2), so capture is a plain pipe (REQ-020). A PTY
//!   fallback is gated behind `TRIUMVIRATE_AGY_CAPTURE=pty` (Slice 2).
//! - **SIGKILL the process group on hang** — agy is a Go binary that ignores soft
//!   signals; a hung agy runs for hours at 0% CPU. Timeouts hard-kill the group and
//!   retry once (REQ-014/020/103).
//! - **Sandbox-exec containment** — agy `-p` auto-executes file-writing tools (Issue
//!   #45) and its own `--sandbox` is not a filesystem boundary, so every dispatch
//!   runs under a Triumvirate-controlled `sandbox-exec` profile that denies
//!   out-of-allowlist writes while leaving reads + network open (REQ-016, probe4).
//! - **`--log-file` is the observability substitute** — serving model + auth method
//!   are recoverable from a per-dispatch glog file; token counts are not (REQ-100/057).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use agent_adapter::{ParsedAgentResult, WorkingState, WorkingStateEvent};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::timeout;
use uuid::Uuid;

use crate::agent_exec::{configure_process_group, emit_working_event, kill_process_group};

/// Parser mode recorded on agy results captured over a pipe (REQ-025).
const PARSER_MODE_PIPE: &str = "agy-pipe-plain-text";

/// Verified `sandbox-exec` profile (probe4, 1.0.1 + 1.0.2). Constrains WRITES, leaves
/// READS + network open so staged artifacts/repo files stay readable and the Google
/// API stays reachable. Placeholders are substituted per dispatch. The canonical copy
/// lives at `research/antigravity/agy-verification/agy-sandbox.sb.template`; this is
/// the shipped duplicate (the research file is not present in a deployed binary).
const SANDBOX_PROFILE_TEMPLATE: &str = r#";; Triumvirate sandbox-exec profile for the agy backend.
;; Constrains WRITES; leaves READS + network open. Verified by probe4 (REQ-016/062b).
(version 1)
(allow default)
(deny file-write*)
(allow file-write* (subpath "@WORKSPACE@"))
(allow file-write* (subpath "@HOME@/.gemini"))
(allow file-write* (subpath "@HOME@/.antigravitycli"))
(allow file-write* (subpath "@TMPDIR@"))
(allow file-write* (subpath "/private/var/folders"))
(allow file-write* (subpath "/private/tmp"))
(allow file-write* (literal "/dev/null") (literal "/dev/stdout") (literal "/dev/stderr") (literal "/dev/dtracehelper") (literal "/dev/tty"))
@EXTRA_WRITABLE@
"#;

// ---------------------------------------------------------------------------
// Env knobs (REQ-014, 020, 058)
// ---------------------------------------------------------------------------

/// agy's dedicated connector timeout (REQ-014). Default 900s — agy is blocking and
/// non-streaming, so the 180s gemini/codex timeout is far too short for multi-tool
/// runs. The same value is passed to agy's own `--print-timeout`; the outer SIGKILL
/// bound adds a small grace so agy's clean exit wins the race when it works.
fn agy_connector_timeout() -> Duration {
    std::env::var("TRIUMVIRATE_AGY_CONNECTOR_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(900))
}

/// Fail-loud guard for the argv size limit (REQ-058). agy has no stdin/`@file` input
/// path, so an oversized prompt would otherwise hit an opaque OS `E2BIG`. macOS
/// ARG_MAX is ~1 MiB (a 280 KB prompt ran fine); default guard 900_000 bytes.
fn agy_max_prompt_bytes() -> usize {
    std::env::var("TRIUMVIRATE_AGY_MAX_PROMPT_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(900_000)
}

/// Capture strategy (REQ-020). `pipe` (default, verified) or `pty` (regression
/// fallback). The PTY path lands in Slice 2; until then `pty` fails loud rather than
/// silently degrading.
fn agy_capture_is_pty() -> bool {
    matches!(
        std::env::var("TRIUMVIRATE_AGY_CAPTURE").ok().as_deref(),
        Some("pty")
    )
}

// ---------------------------------------------------------------------------
// Degraded route + retry classification (REQ-053, 054, 103)
// ---------------------------------------------------------------------------

/// Failure class used to choose the degraded route and retry policy (REQ-053 R2/103).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgyFailureClass {
    /// Quota / 429 / capacity. gemini-cli shares the same Google subscription pool,
    /// so it would also be blocked → SKIP it and go straight to codex.
    Quota,
    /// Auth / exec / capture / protocol. A different failure mode from quota, so the
    /// gemini-cli hop is worth trying first (while it still serves).
    AuthOrExec,
}

/// Classify a surfaced failure message. Ambiguous errors are NOT treated as quota
/// here (REQ-053: "Ambiguous errors are NOT treated as quota"); the circuit breaker
/// biases ambiguous *repeated* failures toward quota separately (Slice 3b).
pub(crate) fn classify_failure_message(msg: &str) -> AgyFailureClass {
    let lower = msg.to_lowercase();
    if lower.contains("capacity/quota")
        || lower.contains("resource_exhausted")
        || lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
    {
        AgyFailureClass::Quota
    } else {
        AgyFailureClass::AuthOrExec
    }
}

/// One hop of the degraded route: which agent answers and the backend label for the
/// honesty fields (REQ-053 R3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DegradedHop {
    /// Agent to dispatch: `gemini` (executed as gemini-cli) or `codex`.
    pub agent: &'static str,
    /// Backend label surfaced to the client.
    pub backend: &'static str,
}

/// Plan the ordered degraded hops from `TRIUMVIRATE_GEMINI_DEGRADED_ROUTE` given the
/// failure class (REQ-053). `fail` (or empty) disables all fallback. Quota-class
/// failures skip the gemini-cli hop (shared quota pool). gemini-cli self-disables by
/// a failed exec when the binary retires on 2026-06-18 — no date check needed.
pub(crate) fn plan_degraded_route(route_env: &str, class: AgyFailureClass) -> Vec<DegradedHop> {
    let trimmed = route_env.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("fail") {
        return Vec::new();
    }
    let mut hops = Vec::new();
    for token in trimmed.split(',') {
        match token.trim().to_lowercase().as_str() {
            "" => {}
            "gemini-cli" => {
                if class != AgyFailureClass::Quota {
                    hops.push(DegradedHop { agent: "gemini", backend: "gemini-cli" });
                }
            }
            "codex" => hops.push(DegradedHop { agent: "codex", backend: "codex" }),
            other => tracing::warn!("ignoring unknown degraded-route token: {other}"),
        }
    }
    hops
}

/// The degraded route value (`TRIUMVIRATE_GEMINI_DEGRADED_ROUTE`, default
/// `gemini-cli,codex`; `fail` disables). REQ-053.
pub(crate) fn degraded_route_env() -> String {
    std::env::var("TRIUMVIRATE_GEMINI_DEGRADED_ROUTE")
        .unwrap_or_else(|_| "gemini-cli,codex".to_string())
}

/// Total wall-clock budget for the whole degraded route (REQ-054, default 900s).
pub(crate) fn degraded_total_budget() -> Duration {
    std::env::var("TRIUMVIRATE_GEMINI_DEGRADED_TOTAL_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(900))
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run a single-turn agy dispatch for the `gemini` agent. Mirrors the shape of
/// `run_gemini_cli_process_with_session` but bypasses `GeminiStreamParser` (no
/// stream-json) and returns plain ANSI-stripped text (REQ-010–026, 040–042).
pub(crate) async fn run_agy_cli_process_with_session(
    bin: &str,
    extra_args: &[String],
    message: &str,
    cwd: &str,
    session_id: Option<&str>,
    events_tx: Option<mpsc::Sender<WorkingStateEvent>>,
) -> anyhow::Result<ParsedAgentResult> {
    // REQ-058: fail loud over ARG_MAX — there is no in-band workaround (no stdin/@file).
    if message.len() > agy_max_prompt_bytes() {
        anyhow::bail!(
            "prompt too large for agy ({} bytes > {} limit): agy has no stdin/file input, so the caller must chunk this consult",
            message.len(),
            agy_max_prompt_bytes()
        );
    }

    // REQ-020: the PTY capture path is not wired yet (Slice 2). Be honest, not silent.
    if agy_capture_is_pty() {
        anyhow::bail!(
            "TRIUMVIRATE_AGY_CAPTURE=pty is not yet implemented; unset it to use the default verified pipe capture"
        );
    }

    // REQ-040/042: single-turn. Ignore inbound session id; never pass resume flags.
    if session_id.is_some() {
        tracing::debug!("agy backend is single-turn; ignoring inbound session_id");
    }

    emit_working_event(events_tx.as_ref(), lifecycle(WorkingState::TurnStarted, "turn started (agy)"));

    let print_timeout = agy_connector_timeout();
    // Outer hard-kill bound: a small grace beyond agy's own --print-timeout so a
    // clean agy exit wins the race; if agy ignores its own timeout (hang), we SIGKILL.
    let kill_after = print_timeout + Duration::from_secs(15);

    let mut last_err: Option<anyhow::Error> = None;

    // One retry on hang/empty (REQ-020/103). A non-zero exit is a real failure → no retry.
    for attempt in 0u32..2 {
        let log_path = agy_log_path();
        let run = run_agy_once(bin, extra_args, message, cwd, &log_path, print_timeout, kill_after).await;

        // REQ-100: parse the per-dispatch log for model/auth/quota, then delete it.
        let log_info = read_and_parse_log(&log_path);
        let _ = std::fs::remove_file(&log_path);

        match run {
            AgyRun::Ok(raw) => {
                let text = strip_ansi(&raw).trim().to_string();
                if text.is_empty() && attempt == 0 {
                    // REQ-020/024 canary: exit 0 + empty → retry once (transient / PTY-drop regression).
                    tracing::warn!("agy returned exit 0 with empty output (attempt {attempt}); retrying once");
                    last_err = Some(anyhow::anyhow!("agy returned empty output"));
                    continue;
                }
                if let Some(model) = &log_info.model {
                    tracing::info!(
                        agy_model = %model,
                        agy_auth = log_info.auth_method.as_deref().unwrap_or("?"),
                        "agy dispatch completed"
                    );
                }
                emit_working_event(events_tx.as_ref(), lifecycle(WorkingState::TurnCompleted, "turn completed (agy)"));
                return Ok(build_result(text, &log_info));
            }
            AgyRun::Timeout => {
                tracing::warn!("agy timed out (attempt {attempt}); SIGKILLed process group");
                last_err = Some(anyhow::anyhow!(
                    "agy connector timed out after {}s",
                    kill_after.as_secs()
                ));
                continue; // hang → retry once (REQ-103)
            }
            AgyRun::NonZero { code, stderr } => {
                // REQ-034/051/052: classify for a user-visible message. Non-zero is a
                // real failure → no retry (the degraded route lands in Slice 3).
                emit_working_event(events_tx.as_ref(), lifecycle(WorkingState::Error, "error (agy)"));
                return Err(classify_failure(code, &stderr, &log_info));
            }
            AgyRun::SpawnError(e) => {
                emit_working_event(events_tx.as_ref(), lifecycle(WorkingState::Error, "error (agy)"));
                return Err(e);
            }
        }
    }

    emit_working_event(events_tx.as_ref(), lifecycle(WorkingState::Error, "error (agy)"));
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("agy dispatch failed")))
}

// ---------------------------------------------------------------------------
// One subprocess run (pipe capture + SIGKILL-process-group timeout)
// ---------------------------------------------------------------------------

enum AgyRun {
    Ok(String),
    Timeout,
    NonZero { code: String, stderr: String },
    SpawnError(anyhow::Error),
}

#[allow(clippy::too_many_arguments)]
async fn run_agy_once(
    bin: &str,
    extra_args: &[String],
    message: &str,
    cwd: &str,
    log_path: &Path,
    print_timeout: Duration,
    kill_after: Duration,
) -> AgyRun {
    let profile_path = match write_sandbox_profile(cwd) {
        Ok(p) => p,
        Err(e) => return AgyRun::SpawnError(anyhow::anyhow!("failed to write agy sandbox profile: {e}")),
    };

    let agy_args = build_agy_args(message, log_path, print_timeout, extra_args);

    // REQ-016: spawn agy UNDER our sandbox-exec profile (containment), never with
    // --dangerously-skip-permissions on the consult path.
    let mut command = Command::new("sandbox-exec");
    command.arg("-f").arg(&profile_path).arg(bin).args(&agy_args);
    command
        .current_dir(cwd)
        .env("NO_COLOR", "1") // minimize ANSI in pipe output (defensive)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_process_group(&mut command);

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_file(&profile_path);
            return AgyRun::SpawnError(anyhow::anyhow!("failed to spawn agy under sandbox-exec: {e}"));
        }
    };

    // Take the pipes out of the child so the read future never borrows `child`
    // (leaving `child` free for wait()/SIGKILL). Buffers are owned by the future
    // and returned, so they remain readable after the timeout completes.
    let mut stdout = match child.stdout.take() {
        Some(s) => s,
        None => return finish_spawn_error(&mut child, &profile_path, "agy stdout missing"),
    };
    let mut stderr = match child.stderr.take() {
        Some(s) => s,
        None => return finish_spawn_error(&mut child, &profile_path, "agy stderr missing"),
    };

    let read = async move {
        let mut out_buf = Vec::new();
        let mut err_buf = Vec::new();
        let (a, b) = tokio::join!(
            stdout.read_to_end(&mut out_buf),
            stderr.read_to_end(&mut err_buf)
        );
        a?;
        b?;
        anyhow::Ok((out_buf, err_buf))
    };

    let outcome = match timeout(kill_after, read).await {
        Ok(Ok((out_buf, err_buf))) => match child.wait().await {
            Ok(status) if status.success() => {
                AgyRun::Ok(String::from_utf8_lossy(&out_buf).into_owned())
            }
            Ok(status) => AgyRun::NonZero {
                code: status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string()),
                stderr: String::from_utf8_lossy(&err_buf).into_owned(),
            },
            Err(e) => AgyRun::SpawnError(anyhow::anyhow!("failed to reap agy: {e}")),
        },
        Ok(Err(e)) => {
            kill_process_group(&mut child);
            let _ = child.kill().await;
            let _ = child.wait().await;
            AgyRun::SpawnError(anyhow::anyhow!("agy capture error: {e}"))
        }
        Err(_) => {
            // REQ-014: SIGKILL the process group — Go ignores soft signals.
            kill_process_group(&mut child);
            let _ = child.kill().await;
            let _ = child.wait().await;
            AgyRun::Timeout
        }
    };

    let _ = std::fs::remove_file(&profile_path);
    outcome
}

fn finish_spawn_error(child: &mut tokio::process::Child, profile_path: &Path, msg: &str) -> AgyRun {
    kill_process_group(child);
    let _ = std::fs::remove_file(profile_path);
    AgyRun::SpawnError(anyhow::anyhow!("{msg}"))
}

// ---------------------------------------------------------------------------
// Command assembly
// ---------------------------------------------------------------------------

/// Build agy's argument vector. REQ-011/012/013: prompt via `-p`, never
/// `-o/--output-format`, `-r/--resume`, `--session-id`, `-c/--continue`, or `--model`.
/// `--print-timeout` and `--log-file` are always set; operator `TRIUMVIRATE_AGY_ARGS`
/// are appended verbatim.
fn build_agy_args(message: &str, log_path: &Path, print_timeout: Duration, extra: &[String]) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        message.to_string(),
        "--print-timeout".to_string(),
        format!("{}s", print_timeout.as_secs()),
        "--log-file".to_string(),
        log_path.to_string_lossy().into_owned(),
    ];
    args.extend(extra.iter().cloned());
    args
}

/// Render the per-dispatch sandbox-exec profile and write it to a temp file (REQ-016).
/// The workspace path is canonicalized so subpath matching works under macOS symlinks
/// (`/var` → `/private/var`), which is why the verified profile uses `/private/...`.
fn write_sandbox_profile(cwd: &str) -> std::io::Result<PathBuf> {
    let workspace = std::fs::canonicalize(cwd)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| cwd.to_string());
    let home = std::env::var("HOME").unwrap_or_default();
    let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    let tmpdir = tmpdir.trim_end_matches('/').to_string();

    let profile = SANDBOX_PROFILE_TEMPLATE
        .replace("@WORKSPACE@", &workspace)
        .replace("@HOME@", &home)
        .replace("@TMPDIR@", &tmpdir)
        .replace("@EXTRA_WRITABLE@", ""); // per-workflow output dirs: Slice 3+

    let path = std::env::temp_dir().join(format!("agy-sandbox-{}.sb", Uuid::new_v4()));
    std::fs::write(&path, profile)?;
    Ok(path)
}

fn agy_log_path() -> PathBuf {
    std::env::temp_dir().join(format!("agy-log-{}.txt", Uuid::new_v4()))
}

// ---------------------------------------------------------------------------
// Result assembly + lifecycle
// ---------------------------------------------------------------------------

/// Build the single-turn agy result (REQ-025): plain text, no session id, no events,
/// no tool calls, no token usage. The serving-model label (from `--log-file`) is the
/// closest honest analogue to `cli_version`.
fn build_result(text: String, log_info: &AgyLogInfo) -> ParsedAgentResult {
    ParsedAgentResult {
        response_text: text,
        session_id: None,
        events: Vec::new(),
        tool_calls: Vec::new(),
        token_usage: None,
        cli_version: log_info.model.clone(),
        parser_mode: PARSER_MODE_PIPE.to_string(),
    }
}

/// Build an honest lifecycle event (REQ-050). The public agent name stays `gemini`
/// (C3); only honest states are emitted — no fabricated tool/token/progress events.
fn lifecycle(state: WorkingState, detail: &str) -> WorkingStateEvent {
    WorkingStateEvent {
        agent: "gemini".to_string(),
        state,
        detail: detail.to_string(),
        tool_name: None,
        tool_args_json: None,
        token_usage: None,
        ts_ms: None,
    }
}

// ---------------------------------------------------------------------------
// --log-file parsing (REQ-100) + failure classification (REQ-034/051/052)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct AgyLogInfo {
    /// Serving model label, e.g. `Gemini 3.1 Pro (High)`.
    model: Option<String>,
    /// Auth method, e.g. `consumer` (subscription) vs a Vertex/metered method.
    auth_method: Option<String>,
    /// A matched quota/429 line, if the final state shows one.
    quota_signal: Option<String>,
}

fn read_and_parse_log(path: &Path) -> AgyLogInfo {
    match std::fs::read_to_string(path) {
        Ok(log) => parse_agy_log(&log),
        Err(_) => AgyLogInfo::default(),
    }
}

/// Parse agy's glog `--log-file`. Parse SPECIFIC patterns and FINAL state — never a
/// generic error-grep: startup emits transient "not logged in" / "Failed to get OAuth
/// token" lines BEFORE auth completes even on a SUCCESSFUL call (REQ-100).
fn parse_agy_log(log: &str) -> AgyLogInfo {
    let mut info = AgyLogInfo::default();
    for line in log.lines() {
        if line.contains("Propagating selected model override to backend") {
            if let Some(label) = extract_quoted_label(line) {
                info.model = Some(label);
            }
        }
        if let Some(rest) = line.split("authMethod=").nth(1) {
            let val = rest
                .split([',', ' '])
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !val.is_empty() {
                info.auth_method = Some(val);
            }
        }
        if quota_signal_in_line(line) {
            info.quota_signal = Some(line.trim().to_string());
        }
    }
    info
}

/// Extract `label="..."` value from a model-propagation log line.
fn extract_quoted_label(line: &str) -> Option<String> {
    let start = line.find("label=\"")? + "label=\"".len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Conservative quota/429 detector (REQ-051). Strong markers always match; the weaker
/// `quota`/`capacity` markers are suppressed on the benign startup auth line, which
/// always contains `quotaProject=`/`authMethod=`. The exact lockout string is still
/// unsampled (REQ-064) — captured on the first real event.
fn quota_signal_in_line(line: &str) -> bool {
    let lower = line.to_lowercase();
    if lower.contains("resource_exhausted")
        || lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("ratelimit")
    {
        return true;
    }
    let benign_auth = lower.contains("quotaproject=") || lower.contains("authmethod=");
    !benign_auth && (lower.contains("quota") || lower.contains("capacity"))
}

/// Classify a non-zero agy exit into a user-visible error (REQ-034/051/052). Quota
/// failures name the capacity cause; auth failures name the re-auth remediation;
/// everything else surfaces the exit code + last stderr line.
fn classify_failure(code: String, stderr: &str, log_info: &AgyLogInfo) -> anyhow::Error {
    let last = stderr.trim().lines().next_back().unwrap_or("").trim();

    if log_info.quota_signal.is_some() || quota_signal_in_line(stderr) {
        let detail = log_info
            .quota_signal
            .clone()
            .unwrap_or_else(|| last.to_string());
        return anyhow::anyhow!("agy capacity/quota error (exit {code}): {detail}");
    }

    let low = stderr.to_lowercase();
    if low.contains("oauth")
        || low.contains("not logged in")
        || low.contains("unauthenticated")
        || low.contains("credential")
        || low.contains("login")
    {
        return anyhow::anyhow!(
            "agy auth error (exit {code}): {last}. Run `agy` once interactively to re-authenticate."
        );
    }

    if last.is_empty() {
        anyhow::anyhow!("agy connector failed: exited with status {code}")
    } else {
        anyhow::anyhow!("agy connector failed (exit {code}): {last}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_csi_and_keeps_text() {
        let input = "\u{1b}[1m\u{1b}[32mhello\u{1b}[0m world";
        assert_eq!(strip_ansi(input), "hello world");
    }

    #[test]
    fn strip_ansi_removes_osc_sequences() {
        let input = "\u{1b}]0;title\u{07}body";
        assert_eq!(strip_ansi(input), "body");
    }

    #[test]
    fn strip_ansi_is_utf8_safe() {
        let input = "caf\u{e9} \u{1b}[31m\u{2014}\u{1b}[0m done";
        assert_eq!(strip_ansi(input), "caf\u{e9} \u{2014} done");
    }

    #[test]
    fn parse_log_extracts_model_and_auth() {
        let log = "I0524 model_config_manager.go:157] Propagating selected model override to backend: label=\"Gemini 3.1 Pro (High)\"\n\
                   I0524 server_oauth.go:212] applyAuthResult: email=x@y.com, authMethod=consumer, quotaProject=\n";
        let info = parse_agy_log(log);
        assert_eq!(info.model.as_deref(), Some("Gemini 3.1 Pro (High)"));
        assert_eq!(info.auth_method.as_deref(), Some("consumer"));
        // The benign auth line (quotaProject=) must NOT register as a quota signal.
        assert!(info.quota_signal.is_none());
    }

    #[test]
    fn quota_detector_ignores_benign_auth_line_but_catches_real_signal() {
        assert!(!quota_signal_in_line(
            "applyAuthResult: authMethod=consumer, quotaProject="
        ));
        assert!(quota_signal_in_line("Error: RESOURCE_EXHAUSTED quota exceeded"));
        assert!(quota_signal_in_line("got HTTP 429 rate limit"));
    }

    #[test]
    fn classify_quota_vs_auth() {
        assert_eq!(
            classify_failure_message("agy capacity/quota error (exit 1): RESOURCE_EXHAUSTED"),
            AgyFailureClass::Quota
        );
        assert_eq!(
            classify_failure_message("got HTTP 429 rate limit"),
            AgyFailureClass::Quota
        );
        assert_eq!(
            classify_failure_message("agy auth error (exit 1): not logged in"),
            AgyFailureClass::AuthOrExec
        );
        // Ambiguous → NOT quota (REQ-053).
        assert_eq!(
            classify_failure_message("agy connector failed (exit 2): something odd"),
            AgyFailureClass::AuthOrExec
        );
    }

    #[test]
    fn route_plan_quota_skips_gemini_cli() {
        let hops = plan_degraded_route("gemini-cli,codex", AgyFailureClass::Quota);
        assert_eq!(hops.iter().map(|h| h.backend).collect::<Vec<_>>(), vec!["codex"]);
    }

    #[test]
    fn route_plan_auth_tries_gemini_cli_first() {
        let hops = plan_degraded_route("gemini-cli,codex", AgyFailureClass::AuthOrExec);
        assert_eq!(
            hops.iter().map(|h| h.backend).collect::<Vec<_>>(),
            vec!["gemini-cli", "codex"]
        );
    }

    #[test]
    fn route_plan_fail_disables_fallback() {
        assert!(plan_degraded_route("fail", AgyFailureClass::AuthOrExec).is_empty());
        assert!(plan_degraded_route("  ", AgyFailureClass::Quota).is_empty());
    }

    #[test]
    fn build_args_never_includes_forbidden_flags() {
        let args = build_agy_args("hi", Path::new("/tmp/x.log"), Duration::from_secs(900), &[]);
        for forbidden in ["-o", "--output-format", "-r", "--resume", "-c", "--continue", "--model"] {
            assert!(!args.iter().any(|a| a == forbidden), "must not pass {forbidden}");
        }
        assert!(args.windows(2).any(|w| w[0] == "-p" && w[1] == "hi"));
        assert!(args.contains(&"--print-timeout".to_string()));
        assert!(args.contains(&"--log-file".to_string()));
    }
}

// ---------------------------------------------------------------------------
// ANSI stripping (REQ-023)
// ---------------------------------------------------------------------------

/// Strip ANSI/CSI/OSC escape sequences from captured text (REQ-023). Char-based and
/// UTF-8 safe (the pipe path is near-clean; this matters more for the PTY fallback).
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next(); // consume '['
                // CSI: consume until a final byte in @..~ (0x40..=0x7e).
                while let Some(&n) = chars.peek() {
                    chars.next();
                    if ('\u{40}'..='\u{7e}').contains(&n) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next(); // consume ']'
                // OSC: until BEL (0x07) or ST (ESC \).
                while let Some(&n) = chars.peek() {
                    if n == '\u{07}' {
                        chars.next();
                        break;
                    }
                    if n == '\u{1b}' {
                        chars.next();
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                    chars.next();
                }
            }
            Some(_) => {
                chars.next(); // two-char ESC sequence
            }
            None => {}
        }
    }
    out
}
