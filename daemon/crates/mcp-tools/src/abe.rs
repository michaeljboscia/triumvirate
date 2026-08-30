use daemon_core::{current_pantheon_session, metrics::DaemonMetrics};
use tracing::instrument;
use shared_types::{
    CancelTaskRequest as AbeCancelTaskRequest, CancelTaskResponse as AbeCancelTaskResponse,
    ContractFields, DispatchCodexRequest, DispatchCodexResponse, DispatchCodexWorktreeRequest,
    DispatchCodexWorktreeResponse, GetTaskOutputRequest, GetTaskOutputResponse,
    GetTaskStatusRequest, GetTaskStatusResponse, TaskCompleteRequest, TaskStatus,
};
use std::{
    collections::HashMap,
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Command,
    sync::Arc,
    time::{Instant, SystemTime},
};
use tokio::{process::Child, sync::Mutex};
use uuid::Uuid;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

pub trait AbeTaskTracker: Clone + Send + Sync + 'static {
    /// Register a newly-dispatched worker task.
    ///
    /// `parent_session_id` / `root_session_id` carry Pantheon lineage
    /// captured from the inbound MCP request (X-Pantheon-Session-Id header
    /// or stdio `_meta.pantheon.session_id`). Callers MUST read these from
    /// `daemon_core::current_pantheon_session()` BEFORE any `tokio::spawn`
    /// — task-locals do not propagate across spawn boundaries.
    ///
    /// Both fields may be `None` for legacy callers that didn't identify
    /// themselves as a Pantheon session; in that case the WorkerLifecycle
    /// events will have `parent_session_id = None` and Pantheon's sidebar
    /// will render the worker as a top-level root.
    #[allow(clippy::too_many_arguments)]
    fn register(
        &self,
        task_id: String,
        wave: u32,
        child: Arc<Mutex<Child>>,
        worktree_path: Option<PathBuf>,
        parent_session_id: Option<String>,
        root_session_id: Option<String>,
        // Which dispatch tool + which repo, so the tracker (the arbiter, and therefore the
        // authoritative emitter) can report tv_codex_dispatch at terminal time. surface=None
        // means a non-codex task and suppresses the event. dispatch_started_at is the caller's
        // clock, taken BEFORE cwd resolution / spawn / worktree setup, so the reported duration
        // covers the whole dispatch, not just the monitored portion.
        dispatch_surface: Option<&'static str>,
        dispatch_repo: Option<String>,
        dispatch_started_at: Instant,
    ) -> BoxFuture<()>;

    // mark_* return `true` iff THIS call won the terminal transition (the record went from
    // Working -> terminal here, not "already terminal because someone else, e.g. cancel, won
    // first"). The dispatch monitor gates its $ai_generation content emit on this bool so a
    // cancel race cannot produce a second, contradictory trace. Callers that don't care ignore it.
    fn mark_completed(
        &self,
        task_id: String,
        commit_sha: String,
        modified_files: Vec<String>,
        stdout: String,
        validation_log: Option<String>,
        test_output: Option<String>,
    ) -> BoxFuture<bool>;

    fn mark_failed(
        &self,
        task_id: String,
        exit_code: Option<i32>,
        error_message: String,
    ) -> BoxFuture<bool>;

    fn mark_timeout(&self, task_id: String) -> BoxFuture<bool>;

    fn mark_stuck(&self, task_id: String, error_message: String) -> BoxFuture<bool>;

    fn register_setup_failed(&self, task_id: String, error_message: String) -> BoxFuture<()>;

    fn get_status(&self, task_id: String) -> BoxFuture<Option<GetTaskStatusResponse>>;

    fn get_output(&self, task_id: String) -> BoxFuture<Option<GetTaskOutputResponse>>;

    fn cancel(&self, task_id: String) -> BoxFuture<Option<AbeCancelTaskResponse>>;
}

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub cmd: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub envs: HashMap<String, String>,
    pub output_log_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct WorktreeSetupRequest {
    pub project_root: PathBuf,
    pub sha: String,
    pub task_id: String,
    pub briefing_content: String,
    pub contract_fields: ContractFields,
}

#[derive(Debug, Clone)]
pub struct WorktreeSetupResult {
    pub worktree_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PostExitValidation {
    pub passed: bool,
    pub violations: Vec<String>,
}

#[derive(Clone)]
pub struct AbeCallbacks {
    pub metrics: Arc<DaemonMetrics>,
    pub codex_command: Arc<dyn Fn() -> (String, Vec<String>) + Send + Sync>,
    pub setup_worktree:
        Arc<dyn Fn(WorktreeSetupRequest) -> Result<WorktreeSetupResult, String> + Send + Sync>,
    pub spawn_background:
        Arc<dyn Fn(SpawnSpec) -> BoxFuture<Result<Arc<Mutex<Child>>, String>> + Send + Sync>,
    pub enforce_timeout:
        Arc<dyn Fn(Arc<Mutex<Child>>, u64, PathBuf) -> BoxFuture<Result<bool, String>> + Send + Sync>,
    pub resolve_commit_outputs: Arc<dyn Fn(&Path, &str) -> (String, Vec<String>) + Send + Sync>,
    pub validate_commit:
        Arc<dyn Fn(&Path, &ContractFields, &str) -> PostExitValidation + Send + Sync>,
    pub rollback_worktree: Arc<dyn Fn(&Path, &Path) -> Result<(), String> + Send + Sync>,
    pub completion_env: Arc<dyn Fn() -> HashMap<String, String> + Send + Sync>,
}

fn observe_task_duration(metrics: &DaemonMetrics, wave: &str, started_at: Instant) {
    metrics
        .abe_task_duration_seconds
        .with_label_values(&[wave])
        .observe(started_at.elapsed().as_secs_f64());
}

fn git_output(worktree_path: &Path, args: &[&str]) -> Option<String> {
    Command::new("git")
        .arg("-C")
        .arg(worktree_path)
        .args(args)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
}

fn git_head(worktree_path: &Path) -> Option<String> {
    git_output(worktree_path, &["rev-parse", "HEAD"])
}

fn git_latest_commit_message(worktree_path: &Path) -> Option<String> {
    git_output(worktree_path, &["log", "-1", "--pretty=%B"])
}

fn commit_message_matches_format(commit_message: &str, commit_format: &str) -> bool {
    let subject = commit_message.lines().next().unwrap_or("");
    if subject == commit_format {
        return true;
    }
    if !commit_format.starts_with('^') && !commit_format.ends_with('$') {
        return false;
    }
    regex_lite::Regex::new(commit_format)
        .map(|re| re.is_match(commit_message))
        .unwrap_or(false)
}

fn parse_task_complete_file(path: &Path) -> Option<TaskCompleteRequest> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<TaskCompleteRequest>(&raw).ok()
}

const WORKTREE_EXCLUDE_PATTERNS: &str = r#"# triumvirate-daemon-managed — DO NOT EDIT
# Hides install artifacts from worker git status to prevent cleanup-loop deaths.
# See memory/feedback_codex_commit_step_is_the_trap.md
node_modules/
**/node_modules/
.pnpm-store/
pnpm-store/
**/pnpm-store/
.venv/
__pycache__/
target/
dist/
.turbo/
.next/
.triumvirate/logs/
"#;

fn resolve_worktree_git_path(worktree_path: &Path, git_path: &str) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_path)
        .arg("rev-parse")
        .arg("--git-path")
        .arg(git_path)
        .output()
        .map_err(|e| format!("failed to run git rev-parse --git-path {git_path}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git rev-parse --git-path {git_path} failed with status {}",
            output.status
        ));
    }
    let resolved = String::from_utf8(output.stdout)
        .map_err(|e| format!("git rev-parse --git-path {git_path} produced non-utf8 output: {e}"))?;
    let resolved = resolved.trim();
    if resolved.is_empty() {
        return Err(format!("git rev-parse --git-path {git_path} returned empty path"));
    }
    let path = Path::new(resolved);
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(worktree_path.join(path))
    }
}

fn write_worktree_exclude_file(worktree_path: &Path) -> Result<(), String> {
    let exclude_path = resolve_worktree_git_path(worktree_path, "info/exclude")?;
    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create parent dir for {}: {e}", exclude_path.display()))?;
    }
    fs::write(&exclude_path, WORKTREE_EXCLUDE_PATTERNS)
        .map_err(|e| format!("failed to write {}: {e}", exclude_path.display()))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn write_worktree_commit_helper(worktree_path: &Path, allowed_files: &[String]) -> Result<(), String> {
    let files_block = allowed_files
        .iter()
        .map(|f| format!("  {}", shell_single_quote(f)))
        .collect::<Vec<_>>()
        .join("\n");
    let script = format!(
        r#"#!/usr/bin/env bash
# Auto-generated by triumvirate daemon. DO NOT EDIT.
# Stages ONLY the contract's allowed_files, then commits.
# See memory/feedback_codex_commit_step_is_the_trap.md
set -euo pipefail

if [ "$#" -lt 1 ] || [ -z "$1" ]; then
  echo "usage: bash .triumvirate/commit.sh '<commit-message>'" >&2
  exit 2
fi

MSG="$1"

FILES=(
{files_block}
)

STAGED=0
for f in "${{FILES[@]}}"; do
  if [ -e "$f" ] || git cat-file -e "HEAD:$f" 2>/dev/null; then
    git add -- "$f"
    STAGED=$((STAGED+1))
  fi
done

if [ "$STAGED" = "0" ]; then
  echo "ERROR: zero files from allowed_files present on disk or at HEAD." >&2
  echo "Allowed files (none matched):" >&2
  printf '  %s\n' "${{FILES[@]}}" >&2
  exit 3
fi

git commit -m "$MSG"
echo "Committed ${{STAGED}} file(s) via triumvirate commit helper."
"#,
        files_block = files_block
    );
    let script_path = worktree_path.join(".triumvirate").join("commit.sh");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create parent dir for {}: {e}", script_path.display()))?;
    }
    fs::write(&script_path, script)
        .map_err(|e| format!("failed to write {}: {e}", script_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path)
            .map_err(|e| format!("failed to stat {}: {e}", script_path.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms)
            .map_err(|e| format!("failed to set executable mode on {}: {e}", script_path.display()))?;
    }
    Ok(())
}

fn latest_worktree_touch(path: &Path) -> Option<SystemTime> {
    fn walk(path: &Path, latest: &mut Option<SystemTime>) {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name == ".git")
            {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if let Ok(modified) = meta.modified() {
                match latest {
                    Some(existing) if modified <= *existing => {}
                    _ => *latest = Some(modified),
                }
            }
            if meta.is_dir() {
                walk(&entry_path, latest);
            }
        }
    }

    let mut latest = None;
    walk(path, &mut latest);
    latest
}

async fn terminate_worker(child: Arc<Mutex<Child>>) {
    let mut locked = child.lock().await;
    if locked.try_wait().ok().flatten().is_some() {
        return;
    }
    let _ = locked.start_kill();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    if locked.try_wait().ok().flatten().is_none() {
        let _ = locked.kill().await;
    }
}

fn cleanup_failed_worktree(
    callbacks: &AbeCallbacks,
    keep_failed: bool,
    project_root: &Path,
    worktree_path: &Path,
) {
    if !keep_failed {
        let _ = (callbacks.rollback_worktree)(project_root, worktree_path);
    }
}

/// Translate user-facing sandbox permission names into codex-exec `-c key=value`
/// config overrides that extend the `--full-auto` preset without clobbering it.
///
/// The daemon's contract API exposes a stable string vocabulary
/// (`"network-full-access"`, etc.) and owns the mapping to codex's actual
/// nested TOML config keys. This insulates callers from codex config churn —
/// codex renamed/restructured these keys between v0.11x revisions (the stale
/// top-level `sandbox_permissions=[...]` example in codex-exec's help text
/// does NOT work against v0.118.0; the live config is under
/// `[sandbox_workspace_write]`).
///
/// Verified working 2026-04-14 on codex v0.118.0 via direct exec smoke test
/// (TEST-SANDBOX-NET, `/tmp/sandbox-perms-test`):
///   `codex exec --full-auto -c 'sandbox_workspace_write.network_access=true' ...`
/// preamble reports "network access enabled" and curl to registry.npmjs.org
/// returns HTTP/2 200.
///
/// Mapping table (extend as new permissions prove out against live codex):
///   "network-full-access" → `-c sandbox_workspace_write.network_access=true`
///
/// Unknown names are skipped with a warning rather than passed through raw —
/// an unrecognized `-c key=value` at the wrong codex version can silently
/// change behavior. Prefer loud-fail: the worker's sandbox stays at
/// --full-auto defaults and the operator sees the warning in daemon logs.
pub(crate) fn build_sandbox_permission_args(perms: Option<&[String]>) -> Vec<String> {
    let list = match perms {
        Some(l) if !l.is_empty() => l,
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for perm in list {
        match perm.as_str() {
            "network-full-access" => {
                out.push("-c".to_string());
                out.push("sandbox_workspace_write.network_access=true".to_string());
            }
            other => {
                tracing::warn!(
                    permission = %other,
                    "unknown sandbox_permissions value; ignored — add a mapping in mcp-tools/src/abe.rs::build_sandbox_permission_args after verifying the codex config key live"
                );
            }
        }
    }
    out
}

/// Outcome of a `git status --porcelain` check used to diagnose why a worker
/// exited cleanly without producing a commit. The dispatcher emits distinct
/// error messages so brief authors can see whether the worker forgot the
/// commit step or never produced any work in the first place.
enum NoCommitDiagnosis {
    /// Worker wrote files matching the contract's allowed_files (or wrote
    /// any files at all when no contract is in scope) but never committed.
    DirtyAllowedFiles(Vec<String>),
    /// Worker wrote files outside the allowed_files contract. Worktree-only;
    /// indicates a contract violation, not just a missing commit step.
    DirtyOtherFiles(Vec<String>),
    /// Worker exited without writing or staging anything. Genuinely did
    /// nothing — distinct from "did work but skipped commit".
    NoChanges,
    /// `git status` itself failed — fall back to the original generic message.
    GitStatusFailed,
}

fn parse_porcelain_paths(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| line.get(3..).map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn diagnose_no_commit(path: &Path, allowed_files: Option<&[String]>) -> NoCommitDiagnosis {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["status", "--porcelain"])
        .output();
    let dirty = match output {
        Ok(out) if out.status.success() => parse_porcelain_paths(&String::from_utf8_lossy(&out.stdout)),
        _ => return NoCommitDiagnosis::GitStatusFailed,
    };
    if dirty.is_empty() {
        return NoCommitDiagnosis::NoChanges;
    }
    match allowed_files {
        None => NoCommitDiagnosis::DirtyAllowedFiles(dirty),
        Some(allowed) => {
            let allowed_set: std::collections::HashSet<&str> =
                allowed.iter().map(|s| s.as_str()).collect();
            let in_allowed: Vec<String> = dirty
                .iter()
                .filter(|f| allowed_set.contains(f.as_str()))
                .cloned()
                .collect();
            if !in_allowed.is_empty() {
                NoCommitDiagnosis::DirtyAllowedFiles(in_allowed)
            } else {
                NoCommitDiagnosis::DirtyOtherFiles(dirty)
            }
        }
    }
}

fn no_commit_error_message(diag: NoCommitDiagnosis) -> String {
    match diag {
        NoCommitDiagnosis::DirtyAllowedFiles(files) => format!(
            "codex wrote files but did not commit — uncommitted: {}; run `bash .triumvirate/commit.sh '<msg>'` before exit",
            files.join(", ")
        ),
        NoCommitDiagnosis::DirtyOtherFiles(files) => format!(
            "codex modified files outside allowed_files and did not commit — dirty paths: {}",
            files.join(", ")
        ),
        NoCommitDiagnosis::NoChanges => {
            "codex exited cleanly without committing or writing any files — agent produced no work".to_string()
        }
        NoCommitDiagnosis::GitStatusFailed => {
            "codex process exited without creating a commit (git status check failed)".to_string()
        }
    }
}

pub(crate) fn append_codex_exec_mcp_compat_args(args: &mut Vec<String>) {
    // codex exec v0.117+ can auto-cancel MCP tool calls in non-interactive
    // workers when MCP elicitation is enabled. Dispatch workers cannot answer
    // prompts, so disable that feature and let Codex's config/server policy
    // decide which MCP tools are callable.
    args.push("--disable".to_string());
    args.push("tool_call_mcp_elicitation".to_string());
}


/// Per-component byte cap for a dispatch's "produced" text. Each component (diff, stdout) is capped
/// INDEPENDENTLY at this size BEFORE they are joined, so a huge stdout cannot starve the diff (or
/// vice versa) under posthog.rs's 60KB whole-field cap (Antigravity). Two ~28KB components + labels
/// stay under 60KB.
const DISPATCH_COMPONENT_CAP: usize = 28 * 1024;

/// Cap `s` to `max` bytes on a char boundary, marking truncation.
fn cap_component(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push_str("\n...<truncated>");
    out
}

/// Read a worker's captured stdout (written by `spawn_background` when `output_log_dir` is set),
/// trimmed and capped. Reads at most CAP+1 bytes off disk (spawn_background writes stdout.log with
/// an unbounded copy, so a noisy worker could leave a huge file); bounding the read keeps the
/// terminal emission from pulling megabytes into memory (Codex review), matching git_show_bounded.
fn read_stdout_log(log_dir: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(log_dir.join("stdout.log")).ok()?;
    let mut buf = Vec::with_capacity(DISPATCH_COMPONENT_CAP + 1);
    f.by_ref()
        .take(DISPATCH_COMPONENT_CAP as u64 + 1)
        .read_to_end(&mut buf)
        .ok()?;
    let s = String::from_utf8_lossy(&buf);
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(cap_component(t, DISPATCH_COMPONENT_CAP))
    }
}

/// `git show` the committed diff, bounded at the PROCESS level: we read at most CAP+1 bytes from
/// git's stdout pipe then kill it, so a worker that commits a 10MB generated blob can never spike
/// the daemon's memory (Antigravity). `--no-ext-diff --no-color` for stable, scrub-friendly text.
fn git_show_bounded(cwd: &std::path::Path, sha: &str) -> Option<String> {
    use std::io::Read;
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["show", "--stat", "-p", "--no-ext-diff", "--no-color", sha])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut buf = Vec::with_capacity(DISPATCH_COMPONENT_CAP + 1);
    if let Some(mut out) = child.stdout.take() {
        let _ = out
            .by_ref()
            .take(DISPATCH_COMPONENT_CAP as u64 + 1)
            .read_to_end(&mut buf);
    }
    let _ = child.kill();
    let _ = child.wait();
    let s = String::from_utf8_lossy(&buf);
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(cap_component(t, DISPATCH_COMPONENT_CAP))
    }
}

/// Assemble a dispatched worker's "produced" text from whichever signals the outcome yields:
/// commit + files + diff (success), diagnosis (no_commit), exit/error (failure/timeout), plus the
/// worker's stdout when captured. Labeled sections so a reader can tell effect (diff) from reasoning
/// (stdout). Components are already capped; this only joins them.
#[allow(clippy::too_many_arguments)]
fn build_dispatch_produced(
    outcome: &str,
    commit_sha: Option<&str>,
    files: &[String],
    diff: Option<&str>,
    diagnosis: Option<&str>,
    exit_code: Option<i32>,
    error: Option<&str>,
    stdout: Option<&str>,
) -> String {
    let mut parts: Vec<String> = vec![format!("[outcome] {outcome}")];
    if let Some(sha) = commit_sha.filter(|s| !s.is_empty()) {
        parts.push(format!("[commit] {sha}"));
    }
    if !files.is_empty() {
        let mut f = files.to_vec();
        f.truncate(50);
        parts.push(format!("[files] {}", f.join(", ")));
    }
    if let Some(code) = exit_code {
        parts.push(format!("[exit_code] {code}"));
    }
    if let Some(e) = error.filter(|s| !s.is_empty()) {
        parts.push(format!("[error]\n{e}"));
    }
    if let Some(d) = diagnosis.filter(|s| !s.is_empty()) {
        parts.push(format!("[diagnosis]\n{d}"));
    }
    if let Some(d) = diff {
        parts.push(format!("[diff]\n{d}"));
    }
    if let Some(o) = stdout {
        parts.push(format!("[stdout]\n{o}"));
    }
    parts.join("\n\n")
}

/// Build + emit a worktree dispatch's told/produced content trace, gated on `won`. Shared by every
/// terminal site (the 4 racing watchers + the main path) so each is a one-liner. The `won` gate is
/// what makes the race safe: only the watcher that WON the terminal transition emits, so the losers
/// (whose mark_* return AlreadyTerminal -> false) stay silent and there is exactly one content
/// trace. Reads stdout from the worktree's own log dir and the committed diff when a commit exists.
#[allow(clippy::too_many_arguments)]
async fn emit_worktree_content(
    won: bool,
    outcome: &str,
    worktree_path: &std::path::Path,
    told: &str,
    trace_id: &str,
    repo: &str,
    commit_sha: Option<&str>,
    files: &[String],
    error_or_diagnosis: Option<&str>,
    duration_ms: u64,
) {
    if !won {
        return;
    }
    let stdout = read_stdout_log(&worktree_path.join(".triumvirate").join("logs"));
    let diff = commit_sha
        .filter(|s| !s.is_empty())
        .and_then(|s| git_show_bounded(worktree_path, s));
    // no_commit carries a diagnosis; the other failures carry an error string. Same field, routed
    // to the right labeled section.
    let (diagnosis, error) = if outcome == "no_commit" {
        (error_or_diagnosis, None)
    } else {
        (None, error_or_diagnosis)
    };
    let produced = build_dispatch_produced(
        outcome,
        commit_sha,
        files,
        diff.as_deref(),
        diagnosis,
        None,
        error,
        stdout.as_deref(),
    );
    mcp_bridge::posthog::record_dispatch_generation(
        trace_id,
        "dispatch_codex_worktree",
        Some(repo),
        told,
        &produced,
        outcome != "completed",
        duration_ms,
    );
}

pub async fn dispatch_codex<T: AbeTaskTracker>(
    tracker: T,
    req: DispatchCodexRequest,
    callbacks: AbeCallbacks,
) -> Result<DispatchCodexResponse, String> {
    callbacks
        .metrics
        .abe_task_dispatch_total
        .with_label_values(&["dispatched"])
        .inc();
    let task_id = format!("abe-{}", Uuid::new_v4().simple());
    // Clock starts HERE, before cwd resolution and before the spawn. Started after the
    // spawn (as it was), the reported duration silently omits spawn negotiation, which is
    // slowest exactly when the machine is loaded and you most want to see it.
    let task_started_at = Instant::now();
    let cwd = req
        .cwd
        .clone()
        .or_else(|| std::env::current_dir().ok().map(|p| p.display().to_string()))
        .ok_or_else(|| {
            // Early return that bypassed telemetry: a dispatch that never happened is still
            // a dispatch someone asked for, and silence here looks identical to no traffic.
            mcp_bridge::posthog::record_codex_dispatch(
                "dispatch_codex",
                "cwd_unresolved",
                0,
                None,
                None,
                None,
            );
            "failed to resolve cwd".to_string()
        })?;
    let timeout_sec = req.timeout_sec.unwrap_or(600);

    // Agent-aware. Defaults to codex, so an ABE dispatch that worked before is unchanged.
    let worker_agent = abe_worker_agent();
    let (cmd, args) = build_worker_argv(
        &worker_agent,
        callbacks.codex_command.as_ref(),
        &req.prompt,
        &cwd,
        false,
    )?;
    let start_sha = std::process::Command::new("git")
        .arg("-C")
        .arg(&cwd)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();

    // Capture the worker's stdout/stderr so the dispatch $ai_generation can show what codex
    // actually SAID (its reasoning), not only the diff it committed. A temp dir keyed by task_id
    // keeps these logs out of the user's repo (unlike worktree dispatch, which logs inside its
    // isolated worktree).
    let log_dir = std::env::temp_dir().join(format!("tv-dispatch-{task_id}"));

    // A spawn failure returns before the monitor task exists, so nothing downstream can
    // report it: we were blind to every "codex would not even start". Emit here, the only
    // place that knows.
    let child = match (callbacks.spawn_background)(SpawnSpec {
        cmd,
        args,
        cwd: cwd.clone(),
        envs: HashMap::new(),
        output_log_dir: Some(log_dir.clone()),
    })
    .await
    {
        Ok(child) => child,
        Err(e) => {
            mcp_bridge::posthog::record_codex_dispatch(
                "dispatch_codex",
                "spawn_failed",
                task_started_at.elapsed().as_millis() as u64,
                Some(&cwd),
                None,
                None,
            );
            return Err(format!("dispatch_codex failed: {e}"));
        }
    };

    // FEAT-014 (REQ-010) T-004: Capture Pantheon lineage from the inbound
    // MCP request's task-local BEFORE we spawn the monitor task. task-locals
    // do not cross tokio::spawn, so reading it inside the spawned closure
    // below would silently return None. Owned Option<String>s are moved
    // into the spawn and attached to register() immediately.
    let pantheon_ctx = current_pantheon_session();
    let parent_session_id = pantheon_ctx.as_ref().map(|c| c.parent_session_id.clone());
    let root_session_id = pantheon_ctx.as_ref().map(|c| c.root_session_id.clone());

    // Content-trace correlation: nest the dispatch $ai_generation under the Pantheon parent (root,
    // else parent) so it renders INSIDE the parent agent's LLM trace rather than as an orphan; fall
    // back to the task id. Computed before register() moves the lineage Options.
    let dispatch_trace_id = root_session_id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| parent_session_id.clone().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| task_id.clone());
    let told = req.prompt.clone();

    tracker
        .register(
            task_id.clone(),
            0,
            child.clone(),
            None,
            parent_session_id,
            root_session_id,
            Some("dispatch_codex"),
            Some(cwd.clone()),
            task_started_at,
        )
        .await;

    let tracker_for_monitor = tracker.clone();
    let callbacks_for_monitor = callbacks.clone();
    let task_id_for_monitor = task_id.clone();
    // Captured into the monitor so it can emit the dispatch content $ai_generation (told + produced)
    // on the terminal transition it WINS. See the emit calls in each branch below.
    let told_for_monitor = told;
    let trace_id_for_monitor = dispatch_trace_id;
    let log_dir_for_monitor = log_dir;
    let repo_for_monitor = cwd.clone();
    // Supervisor clones: reach the tracker if the monitor dies before any transition.
    let tracker_for_sup = tracker.clone();
    let task_id_for_sup = task_id.clone();
    // The monitor calls mark_* on the tracker, and the TRACKER emits tv_codex_dispatch on the
    // terminal transition (it is the arbiter). No report/canary here: those would double-emit.
    let monitor = tokio::spawn(async move {
        let wave_label = "mcp";
        let cwd_path = PathBuf::from(&cwd);
        let timeout_duration = std::time::Duration::from_secs(timeout_sec);

        // Race the worker against its deadline. The previous structure ran
        // an unconditional sleep(timeout_sec) before wait(), which both
        // delayed completion reporting for fast tasks and produced false
        // "task timed out" results when the worker finished mid-grace.
        // First-to-finish wins eliminates both bugs.
        let exit_outcome: Option<std::io::Result<std::process::ExitStatus>> = {
            let child_for_wait = child.clone();
            tokio::select! {
                wait_result = async move {
                    let mut locked = child_for_wait.lock().await;
                    locked.wait().await
                } => Some(wait_result),
                _ = tokio::time::sleep(timeout_duration) => None,
            }
        };

        // Emit the dispatch content trace ($ai_input=told, $ai_output_choices=produced) ONLY when
        // this monitor WON the terminal transition (mark_* returned true). If cancel won first the
        // mark_* returns false and we stay silent, so there is exactly one content trace per task.
        let dur_ms = || task_started_at.elapsed().as_millis() as u64;
        let exit = match exit_outcome {
            Some(Ok(status)) => status,
            Some(Err(err)) => {
                let won = tracker_for_monitor
                    .mark_failed(task_id_for_monitor.clone(), None, err.to_string())
                    .await;
                if won {
                    let stdout = read_stdout_log(&log_dir_for_monitor);
                    let produced = build_dispatch_produced(
                        "failed", None, &[], None, None, None, Some(&err.to_string()),
                        stdout.as_deref(),
                    );
                    mcp_bridge::posthog::record_dispatch_generation(
                        &trace_id_for_monitor, "dispatch_codex", Some(&repo_for_monitor),
                        &told_for_monitor, &produced, true, dur_ms(),
                    );
                }
                return;
            }
            None => {
                terminate_worker(child.clone()).await;
                observe_task_duration(&callbacks_for_monitor.metrics, wave_label, task_started_at);
                let won = tracker_for_monitor
                    .mark_timeout(task_id_for_monitor.clone())
                    .await;
                if won {
                    let stdout = read_stdout_log(&log_dir_for_monitor);
                    let produced = build_dispatch_produced(
                        "timeout", None, &[], None, None, None,
                        Some("worker exceeded its timeout and was terminated"), stdout.as_deref(),
                    );
                    mcp_bridge::posthog::record_dispatch_generation(
                        &trace_id_for_monitor, "dispatch_codex", Some(&repo_for_monitor),
                        &told_for_monitor, &produced, true, dur_ms(),
                    );
                }
                return;
            }
        };

        if exit.success() {
            let (commit_sha, files) =
                (callbacks_for_monitor.resolve_commit_outputs)(&cwd_path, &start_sha);
            if commit_sha.is_empty() {
                observe_task_duration(&callbacks_for_monitor.metrics, wave_label, task_started_at);
                // Exit 0, no commit: codex reported success and changed nothing. The most
                // valuable event here, because it is the one a human only discovers by going
                // to look for a commit that was never made.
                let diag = diagnose_no_commit(&cwd_path, None);
                let msg = no_commit_error_message(diag);
                let won = tracker_for_monitor
                    .mark_failed(task_id_for_monitor.clone(), exit.code(), msg.clone())
                    .await;
                if won {
                    let stdout = read_stdout_log(&log_dir_for_monitor);
                    let produced = build_dispatch_produced(
                        "no_commit", None, &[], None, Some(&msg), exit.code(), None,
                        stdout.as_deref(),
                    );
                    mcp_bridge::posthog::record_dispatch_generation(
                        &trace_id_for_monitor, "dispatch_codex", Some(&repo_for_monitor),
                        &told_for_monitor, &produced, true, dur_ms(),
                    );
                }
                return;
            }
            observe_task_duration(&callbacks_for_monitor.metrics, wave_label, task_started_at);
            // Build produced BEFORE mark_completed moves commit_sha/files.
            let diff = git_show_bounded(&cwd_path, &commit_sha);
            let stdout = read_stdout_log(&log_dir_for_monitor);
            let produced = build_dispatch_produced(
                "completed", Some(&commit_sha), &files, diff.as_deref(), None, None, None,
                stdout.as_deref(),
            );
            let won = tracker_for_monitor
                .mark_completed(
                    task_id_for_monitor.clone(),
                    commit_sha,
                    files,
                    String::new(),
                    None,
                    None,
                )
                .await;
            if won {
                mcp_bridge::posthog::record_dispatch_generation(
                    &trace_id_for_monitor, "dispatch_codex", Some(&repo_for_monitor),
                    &told_for_monitor, &produced, false, dur_ms(),
                );
            }
        } else {
            observe_task_duration(&callbacks_for_monitor.metrics, wave_label, task_started_at);
            let won = tracker_for_monitor
                .mark_failed(
                    task_id_for_monitor.clone(),
                    exit.code(),
                    "codex process failed".to_string(),
                )
                .await;
            if won {
                let stdout = read_stdout_log(&log_dir_for_monitor);
                let produced = build_dispatch_produced(
                    "failed", None, &[], None, None, exit.code(), Some("codex process failed"),
                    stdout.as_deref(),
                );
                mcp_bridge::posthog::record_dispatch_generation(
                    &trace_id_for_monitor, "dispatch_codex", Some(&repo_for_monitor),
                    &told_for_monitor, &produced, true, dur_ms(),
                );
            }
        }
    });

    // Supervisor (Antigravity's JoinHandle pattern, replacing the old Drop canary): if the
    // monitor PANICS or is aborted before it reaches any mark_*, the tracker never transitions
    // and nothing is emitted — the dispatch would vanish. Route that through mark_failed, which
    // emits via the tracker exactly once (AlreadyTerminal if the body already transitioned, so
    // no double-emit). Unified path, immediate detection, no cross-crate canary state.
    tokio::spawn(async move {
        match monitor.await {
            Ok(()) => {}
            Err(e) if e.is_panic() => {
                tracker_for_sup
                    .mark_failed(task_id_for_sup, None, "dispatch monitor panicked".to_string())
                    .await;
            }
            Err(e) if e.is_cancelled() => {
                tracker_for_sup
                    .mark_failed(task_id_for_sup, None, "dispatch monitor cancelled".to_string())
                    .await;
            }
            Err(_) => {}
        }
    });

    Ok(DispatchCodexResponse {
        task_id,
        status: "dispatched".to_string(),
    })
}

/// Report a worktree dispatch that died during pre-register setup. These paths call
/// register_setup_failed (which stores no dispatch_surface, so the tracker can't emit) and
/// return, so this is the only place that knows a worktree dispatch failed at setup.
fn emit_worktree_setup_failed(project_root: &Path, started_at: Instant) {
    mcp_bridge::posthog::record_codex_dispatch(
        "dispatch_codex_worktree",
        "setup_failed",
        started_at.elapsed().as_millis() as u64,
        Some(&project_root.display().to_string()),
        None,
        None,
    );
}

#[instrument(skip_all, fields(task_id = %req.contract_fields.task_id))]
pub async fn dispatch_codex_worktree<T: AbeTaskTracker>(
    tracker: T,
    req: DispatchCodexWorktreeRequest,
    callbacks: AbeCallbacks,
) -> Result<DispatchCodexWorktreeResponse, String> {
    callbacks
        .metrics
        .abe_task_dispatch_total
        .with_label_values(&["dispatched"])
        .inc();
    let task_id = req.contract_fields.task_id.clone();
    let validation =
        shared_types::validate_contract(&req.contract_fields).map_err(|e| format!("invalid contract_fields: {e}"));
    if let Err(err) = validation {
        tracker
            .register_setup_failed(task_id.clone(), err.clone())
            .await;
        return Err(err);
    }

    let project_root = match req
        .project_root
        .clone()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
    {
        Some(root) => root,
        None => {
            // The `?` used to skip telemetry here (Antigravity), leaving worktree root-
            // resolution failures dark. Emit directly; no repo is known.
            mcp_bridge::posthog::record_codex_dispatch(
                "dispatch_codex_worktree",
                "project_root_unresolved",
                0,
                None,
                None,
                None,
            );
            return Err("failed to resolve project_root".to_string());
        }
    };
    // Dispatch clock starts here, before setup + spawn, so the terminal duration covers the
    // whole dispatch (Codex: worktree setup/spawn happen before register).
    let dispatch_started_at = Instant::now();
    let setup = (callbacks.setup_worktree)(WorktreeSetupRequest {
        project_root: project_root.clone(),
        sha: req.sha.clone(),
        task_id: task_id.clone(),
        briefing_content: req.briefing_content.clone(),
        contract_fields: req.contract_fields.clone(),
    });
    let setup = match setup {
        Ok(s) => s,
        Err(err) => {
            // Pre-register path: register_setup_failed inserts a record directly without a
            // dispatch_surface, so it can't emit via the tracker. Report it here (all three
            // worktree setup-failure sites do), the only place that knows this was a worktree
            // dispatch.
            emit_worktree_setup_failed(&project_root, dispatch_started_at);
            tracker
                .register_setup_failed(task_id.clone(), err.to_string())
                .await;
            return Err(format!("SETUP_FAILED: {err}"));
        }
    };
    if let Err(err) = write_worktree_exclude_file(&setup.worktree_path) {
        emit_worktree_setup_failed(&project_root, dispatch_started_at);
        tracker
            .register_setup_failed(task_id.clone(), err.clone())
            .await;
        return Err(format!("SETUP_FAILED: {err}"));
    }
    if let Err(err) = write_worktree_commit_helper(&setup.worktree_path, &req.contract_fields.allowed_files) {
        emit_worktree_setup_failed(&project_root, dispatch_started_at);
        tracker
            .register_setup_failed(task_id.clone(), err.clone())
            .await;
        return Err(format!("SETUP_FAILED: {err}"));
    }

    let prompt =
        "Read .triumvirate/BRIEFING.md and implement the task contract. Commit when complete.".to_string();
    let (cmd, mut args) = (callbacks.codex_command)();
    args.push("exec".to_string());
    args.push("--full-auto".to_string());
    append_codex_exec_mcp_compat_args(&mut args);
    // Translate sandbox_permissions contract field into `-c key=value` overrides
    // that codex-exec merges ON TOP of --full-auto. See build_sandbox_permission_args.
    args.extend(build_sandbox_permission_args(
        req.contract_fields.sandbox_permissions.as_deref(),
    ));

    let main_git_dir = project_root.join(".git");
    args.push("--add-dir".to_string());
    args.push(main_git_dir.display().to_string());

    let dot_git = setup.worktree_path.join(".git");
    if dot_git.is_file() {
        if let Ok(content) = std::fs::read_to_string(&dot_git) {
            if let Some(gitdir) = content.strip_prefix("gitdir: ").map(|s| s.trim()) {
                let resolved = if std::path::Path::new(gitdir).is_absolute() {
                    gitdir.to_string()
                } else {
                    setup.worktree_path.join(gitdir).display().to_string()
                };
                args.push("--add-dir".to_string());
                args.push(resolved);
            }
        }
    }

    args.push(prompt);

    let mut worker_env = (callbacks.completion_env)();
    worker_env.insert(
        "CARGO_TARGET_DIR".to_string(),
        setup.worktree_path
            .join(".triumvirate")
            .join("target")
            .join(task_id.clone())
            .display()
            .to_string(),
    );

    let child = match (callbacks.spawn_background)(SpawnSpec {
        cmd,
        args,
        cwd: setup.worktree_path.display().to_string(),
        envs: worker_env,
        output_log_dir: Some(setup.worktree_path.join(".triumvirate").join("logs")),
    })
    .await
    {
        Ok(child) => child,
        Err(e) => {
            // Pre-register (like dispatch_codex): no monitor exists to report it. Emit directly.
            mcp_bridge::posthog::record_codex_dispatch(
                "dispatch_codex_worktree",
                "spawn_failed",
                dispatch_started_at.elapsed().as_millis() as u64,
                Some(&project_root.display().to_string()),
                None,
                None,
            );
            return Err(format!("dispatch_codex_worktree failed: {e}"));
        }
    };

    // FEAT-014 (REQ-010) T-004: capture Pantheon lineage once in the request
    // task before the monitor task is spawned; see dispatch_codex for rationale.
    let pantheon_ctx = current_pantheon_session();
    let parent_session_id = pantheon_ctx.as_ref().map(|c| c.parent_session_id.clone());
    let root_session_id = pantheon_ctx.as_ref().map(|c| c.root_session_id.clone());

    // Content-trace inputs for the dispatch $ai_generation (told/produced), computed before
    // register() moves the lineage Options. trace_id nests under the Pantheon parent when present,
    // else the task id. `told` is the briefing the worktree worker was given.
    let wt_told = req.briefing_content.clone();
    let wt_trace_id = root_session_id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| parent_session_id.clone().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| task_id.clone());
    let wt_repo = project_root.display().to_string();

    tracker
        .register(
            task_id.clone(),
            req.contract_fields.wave,
            child.clone(),
            Some(setup.worktree_path.clone()),
            parent_session_id,
            root_session_id,
            Some("dispatch_codex_worktree"),
            Some(project_root.display().to_string()),
            dispatch_started_at,
        )
        .await;

    // KNOWN LIMITATION (Antigravity): unlike dispatch_codex, these worktree watchers are not
    // wrapped in a JoinHandle supervisor. A watcher that PANICS before any mark_* is not
    // attributed as a panic — the timeout watcher eventually marks the task `timeout`, which
    // masks a daemon crash as an agent-speed issue. The tracker still emits (as timeout), so
    // nothing vanishes; only the ATTRIBUTION is wrong. A per-watcher supervisor is the fix and
    // is deferred: worktree dispatch is cold (last real run 2026-05-12) and the 4-watcher
    // structure makes supervision a larger change than dispatch_codex's single monitor.
    let tracker_for_monitor = tracker.clone();
    let callbacks_for_monitor = callbacks.clone();
    let worktree_path = setup.worktree_path.clone();
    let timeout_sec = req.contract_fields.task_timeout_sec;
    let wave_label = req.contract_fields.wave.to_string();
    let keep_failed = req.keep_failed_worktree.unwrap_or(false);
    let project_root_for_cleanup = project_root.clone();
    let task_id_for_monitor = task_id.clone();
    let start_sha = req.sha.clone();
    let contract_for_validation = req.contract_fields.clone();
    let task_started_at = Instant::now();
    tokio::spawn(async move {
        let timeout_tracker = tracker_for_monitor.clone();
        let timeout_callbacks = callbacks_for_monitor.clone();
        let timeout_task_id = task_id_for_monitor.clone();
        let timeout_worktree = worktree_path.clone();
        let timeout_child = child.clone();
        let timeout_wave_label = wave_label.clone();
        let timeout_metrics = callbacks_for_monitor.metrics.clone();
        let timeout_project_root = project_root_for_cleanup.clone();
        let timeout_cleanup_worktree = worktree_path.clone();
        let timeout_keep_failed = keep_failed;
        let timeout_told = wt_told.clone();
        let timeout_trace_id = wt_trace_id.clone();
        let timeout_repo = wt_repo.clone();
        tokio::spawn(async move {
            let enforce_timeout = timeout_callbacks.enforce_timeout.clone();
            let timed_out = (enforce_timeout)(timeout_child, timeout_sec, timeout_worktree)
                .await
                .unwrap_or(false);
            if timed_out {
                observe_task_duration(&timeout_metrics, &timeout_wave_label, task_started_at);
                let won = timeout_tracker.mark_timeout(timeout_task_id).await;
                // Emit BEFORE cleanup deletes the worktree (logs + any commit).
                emit_worktree_content(
                    won, "timeout", &timeout_cleanup_worktree, &timeout_told, &timeout_trace_id,
                    &timeout_repo, None, &[], Some("worker exceeded its timeout and was terminated"),
                    task_started_at.elapsed().as_millis() as u64,
                )
                .await;
                cleanup_failed_worktree(
                    &timeout_callbacks,
                    timeout_keep_failed,
                    &timeout_project_root,
                    &timeout_cleanup_worktree,
                );
            }
        });

        let sentinel_tracker = tracker_for_monitor.clone();
        let sentinel_task_id = task_id_for_monitor.clone();
        let sentinel_worktree = worktree_path.clone();
        let sentinel_start_sha = start_sha.clone();
        let sentinel_child = child.clone();
        let resolve_commit_outputs = callbacks_for_monitor.resolve_commit_outputs.clone();
        let sentinel_wave_label = wave_label.clone();
        let sentinel_metrics = callbacks_for_monitor.metrics.clone();
        let sentinel_told = wt_told.clone();
        let sentinel_trace_id = wt_trace_id.clone();
        let sentinel_repo = wt_repo.clone();
        tokio::spawn(async move {
            let sentinel_path = sentinel_worktree.join(".triumvirate").join("TASK_COMPLETE.json");
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                let status = sentinel_tracker
                    .get_status(sentinel_task_id.clone())
                    .await
                    .map(|s| s.status);
                if !matches!(status, Some(TaskStatus::Working)) {
                    break;
                }
                if !sentinel_path.exists() {
                    continue;
                }
                let Some(payload) = parse_task_complete_file(&sentinel_path) else {
                    continue;
                };
                if payload.task_id != sentinel_task_id {
                    continue;
                }
                let Some(head_sha) = git_head(&sentinel_worktree) else {
                    continue;
                };
                if head_sha != payload.commit_sha {
                    continue;
                }
                let (commit_sha, files) = (resolve_commit_outputs)(&sentinel_worktree, &sentinel_start_sha);
                if commit_sha.is_empty() {
                    continue;
                }
                let won = sentinel_tracker
                    .mark_completed(
                        sentinel_task_id.clone(),
                        commit_sha.clone(),
                        files.clone(),
                        String::new(),
                        None,
                        None,
                    )
                    .await;
                emit_worktree_content(
                    won, "completed", &sentinel_worktree, &sentinel_told, &sentinel_trace_id,
                    &sentinel_repo, Some(&commit_sha), &files, None,
                    task_started_at.elapsed().as_millis() as u64,
                )
                .await;
                observe_task_duration(&sentinel_metrics, &sentinel_wave_label, task_started_at);
                terminate_worker(sentinel_child).await;
                break;
            }
        });

        let head_tracker = tracker_for_monitor.clone();
        let head_task_id = task_id_for_monitor.clone();
        let head_worktree = worktree_path.clone();
        let head_start_sha = start_sha.clone();
        let head_commit_format = contract_for_validation.commit_format.clone();
        let head_child = child.clone();
        let resolve_commit_outputs = callbacks_for_monitor.resolve_commit_outputs.clone();
        let head_wave_label = wave_label.clone();
        let head_metrics = callbacks_for_monitor.metrics.clone();
        let head_told = wt_told.clone();
        let head_trace_id = wt_trace_id.clone();
        let head_repo = wt_repo.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let status = head_tracker
                    .get_status(head_task_id.clone())
                    .await
                    .map(|s| s.status);
                if !matches!(status, Some(TaskStatus::Working)) {
                    break;
                }
                let Some(head_sha) = git_head(&head_worktree) else {
                    continue;
                };
                if head_sha == head_start_sha {
                    continue;
                }
                let Some(message) = git_latest_commit_message(&head_worktree) else {
                    continue;
                };
                if !commit_message_matches_format(&message, &head_commit_format) {
                    continue;
                }
                let (commit_sha, files) = (resolve_commit_outputs)(&head_worktree, &head_start_sha);
                if commit_sha.is_empty() {
                    continue;
                }
                let won = head_tracker
                    .mark_completed(head_task_id.clone(), commit_sha.clone(), files.clone(), String::new(), None, None)
                    .await;
                emit_worktree_content(
                    won, "completed", &head_worktree, &head_told, &head_trace_id, &head_repo,
                    Some(&commit_sha), &files, None, task_started_at.elapsed().as_millis() as u64,
                )
                .await;
                observe_task_duration(&head_metrics, &head_wave_label, task_started_at);
                terminate_worker(head_child).await;
                break;
            }
        });

        let stuck_tracker = tracker_for_monitor.clone();
        let stuck_task_id = task_id_for_monitor.clone();
        let stuck_worktree = worktree_path.clone();
        let stuck_child = child.clone();
        let stuck_wave_label = wave_label.clone();
        let stuck_metrics = callbacks_for_monitor.metrics.clone();
        let stuck_callbacks = callbacks_for_monitor.clone();
        let stuck_project_root = project_root_for_cleanup.clone();
        let stuck_cleanup_worktree = worktree_path.clone();
        let stuck_keep_failed = keep_failed;
        let stuck_told = wt_told.clone();
        let stuck_trace_id = wt_trace_id.clone();
        let stuck_repo = wt_repo.clone();
        tokio::spawn(async move {
            let mut last_touch = latest_worktree_touch(&stuck_worktree).or_else(|| Some(SystemTime::now()));
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let status = stuck_tracker
                    .get_status(stuck_task_id.clone())
                    .await
                    .map(|s| s.status);
                if !matches!(status, Some(TaskStatus::Working)) {
                    break;
                }
                if let Some(current_touch) = latest_worktree_touch(&stuck_worktree) {
                    if match last_touch {
                        None => true,
                        Some(prev) => current_touch > prev,
                    } {
                        last_touch = Some(current_touch);
                    }
                }
                let idle_for = last_touch
                    .and_then(|touch| SystemTime::now().duration_since(touch).ok())
                    .unwrap_or_default();
                if idle_for.as_secs() < 180 {
                    continue;
                }
                let won = stuck_tracker
                    .mark_stuck(
                        stuck_task_id.clone(),
                        "worker marked STUCK after 180s without filesystem activity".to_string(),
                    )
                    .await;
                emit_worktree_content(
                    won, "stuck", &stuck_cleanup_worktree, &stuck_told, &stuck_trace_id, &stuck_repo,
                    None, &[], Some("worker marked STUCK after 180s without filesystem activity"),
                    task_started_at.elapsed().as_millis() as u64,
                )
                .await;
                observe_task_duration(&stuck_metrics, &stuck_wave_label, task_started_at);
                terminate_worker(stuck_child).await;
                cleanup_failed_worktree(
                    &stuck_callbacks,
                    stuck_keep_failed,
                    &stuck_project_root,
                    &stuck_cleanup_worktree,
                );
                break;
            }
        });

        let exit = loop {
            {
                let mut locked = child.lock().await;
                match locked.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None) => {}
                    Err(err) => {
                        let won = tracker_for_monitor
                            .mark_failed(task_id_for_monitor.clone(), None, err.to_string())
                            .await;
                        emit_worktree_content(
                            won, "failed", &worktree_path, &wt_told, &wt_trace_id, &wt_repo,
                            None, &[], Some(&err.to_string()),
                            task_started_at.elapsed().as_millis() as u64,
                        )
                        .await;
                        cleanup_failed_worktree(
                            &callbacks_for_monitor,
                            keep_failed,
                            &project_root_for_cleanup,
                            &worktree_path,
                        );
                        return;
                    }
                }
            }
            let status = tracker_for_monitor
                .get_status(task_id_for_monitor.clone())
                .await
                .map(|s| s.status);
            if !matches!(status, Some(TaskStatus::Working)) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        };

        if exit.success() {
            let (commit_sha, files) =
                (callbacks_for_monitor.resolve_commit_outputs)(&worktree_path, &start_sha);
            if commit_sha.is_empty() {
                observe_task_duration(&callbacks_for_monitor.metrics, &wave_label, task_started_at);
                let diag = diagnose_no_commit(
                    &worktree_path,
                    Some(&contract_for_validation.allowed_files),
                );
                let msg = no_commit_error_message(diag);
                let won = tracker_for_monitor
                    .mark_failed(task_id_for_monitor.clone(), exit.code(), msg.clone())
                    .await;
                emit_worktree_content(
                    won, "no_commit", &worktree_path, &wt_told, &wt_trace_id, &wt_repo,
                    None, &[], Some(&msg), task_started_at.elapsed().as_millis() as u64,
                )
                .await;
                cleanup_failed_worktree(
                    &callbacks_for_monitor,
                    keep_failed,
                    &project_root_for_cleanup,
                    &worktree_path,
                );
                return;
            }

            let validation =
                (callbacks_for_monitor.validate_commit)(&worktree_path, &contract_for_validation, &start_sha);
            if !validation.passed {
                let violation_summary = validation.violations.join("; ");
                observe_task_duration(&callbacks_for_monitor.metrics, &wave_label, task_started_at);
                let msg = format!("DAEMON_VALIDATION_FAILED: {violation_summary}");
                let won = tracker_for_monitor
                    .mark_failed(task_id_for_monitor.clone(), None, msg.clone())
                    .await;
                // Pass the committed sha + files: validation runs AFTER a real commit, so the diff
                // that FAILED validation is exactly what you want to see (Codex review).
                emit_worktree_content(
                    won, "validation_failed", &worktree_path, &wt_told, &wt_trace_id, &wt_repo,
                    Some(&commit_sha), &files, Some(&msg),
                    task_started_at.elapsed().as_millis() as u64,
                )
                .await;
                cleanup_failed_worktree(
                    &callbacks_for_monitor,
                    keep_failed,
                    &project_root_for_cleanup,
                    &worktree_path,
                );
                return;
            }

            let validation_log =
                fs::read_to_string(worktree_path.join(".triumvirate").join("VALIDATION_LOG.md")).ok();
            let won = tracker_for_monitor
                .mark_completed(
                    task_id_for_monitor.clone(),
                    commit_sha.clone(),
                    files.clone(),
                    String::new(),
                    validation_log,
                    None,
                )
                .await;
            emit_worktree_content(
                won, "completed", &worktree_path, &wt_told, &wt_trace_id, &wt_repo,
                Some(&commit_sha), &files, None, task_started_at.elapsed().as_millis() as u64,
            )
            .await;
            observe_task_duration(&callbacks_for_monitor.metrics, &wave_label, task_started_at);
        } else {
            observe_task_duration(&callbacks_for_monitor.metrics, &wave_label, task_started_at);
            let won = tracker_for_monitor
                .mark_failed(
                    task_id_for_monitor.clone(),
                    exit.code(),
                    "codex process failed".to_string(),
                )
                .await;
            emit_worktree_content(
                won, "failed", &worktree_path, &wt_told, &wt_trace_id, &wt_repo,
                None, &[], Some("codex process failed"),
                task_started_at.elapsed().as_millis() as u64,
            )
            .await;
            cleanup_failed_worktree(
                &callbacks_for_monitor,
                keep_failed,
                &project_root_for_cleanup,
                &worktree_path,
            );
        }
    });

    Ok(DispatchCodexWorktreeResponse {
        task_id,
        worktree_path: setup.worktree_path.display().to_string(),
        status: "dispatched".to_string(),
    })
}

#[cfg(test)]
mod dispatch_produced_tests {
    use super::*;

    #[test]
    fn cap_component_truncates_on_char_boundary_and_marks() {
        let short = "hello";
        assert_eq!(cap_component(short, 100), "hello");
        // Multibyte content near the cap must not split a UTF-8 sequence.
        let multi = "é".repeat(100); // 2 bytes each = 200 bytes
        let capped = cap_component(&multi, 51);
        assert!(capped.ends_with("...<truncated>"), "truncation marked: {capped}");
        assert!(capped.len() <= 51 + "\n...<truncated>".len() + 1);
        // The kept prefix is valid UTF-8 (no panic constructing it) and contains only 'é'.
        assert!(capped.trim_end_matches("\n...<truncated>").chars().all(|c| c == 'é'));
    }

    #[test]
    fn build_produced_labels_and_orders_available_signals() {
        // Success: commit + files + diff, no error/diagnosis.
        let files = vec!["a.rs".to_string(), "b.rs".to_string()];
        let p = build_dispatch_produced(
            "completed", Some("abc123"), &files, Some("--- diff body ---"), None, None, None,
            Some("codex said hi"),
        );
        assert!(p.contains("[outcome] completed"));
        assert!(p.contains("[commit] abc123"));
        assert!(p.contains("[files] a.rs, b.rs"));
        assert!(p.contains("[diff]\n--- diff body ---"));
        assert!(p.contains("[stdout]\ncodex said hi"));
        assert!(!p.contains("[error]") && !p.contains("[diagnosis]"), "no empty sections: {p}");

        // Failure with no commit: exit + error + stdout, no diff/commit section.
        let none: Vec<String> = vec![];
        let f = build_dispatch_produced(
            "failed", None, &none, None, None, Some(1), Some("boom"), Some("trace..."),
        );
        assert!(f.contains("[outcome] failed") && f.contains("[exit_code] 1"));
        assert!(f.contains("[error]\nboom") && f.contains("[stdout]\ntrace..."));
        assert!(!f.contains("[commit]") && !f.contains("[diff]"), "no commit/diff on failure: {f}");

        // Empty commit sha is treated as absent (no [commit] line).
        let e = build_dispatch_produced("no_commit", Some(""), &none, None, Some("nothing changed"), Some(0), None, None);
        assert!(!e.contains("[commit]"));
        assert!(e.contains("[diagnosis]\nnothing changed"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_helper_single_quotes_allowed_files() {
        let temp_dir = std::env::temp_dir().join(format!("abe-commit-helper-{}", Uuid::new_v4()));
        fs::create_dir_all(temp_dir.join(".triumvirate")).expect("create temp dir");
        let files = vec![
            "apps/web/src/it's.ts".to_string(),
            "scripts/.verify-commit-step-fixes.marker".to_string(),
        ];
        write_worktree_commit_helper(&temp_dir, &files).expect("write commit helper");
        let script_path = temp_dir.join(".triumvirate").join("commit.sh");
        let script = fs::read_to_string(&script_path).expect("read generated script");
        assert!(script.contains("'apps/web/src/it'\\''s.ts'"));
        assert!(script.contains("'scripts/.verify-commit-step-fixes.marker'"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&script_path).expect("stat script").permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "script should be executable");
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn exclude_file_written_via_git_path_resolution() {
        let temp_dir = std::env::temp_dir().join(format!("abe-exclude-helper-{}", Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let init = Command::new("git")
            .arg("-C")
            .arg(&temp_dir)
            .arg("init")
            .output()
            .expect("git init should run");
        assert!(init.status.success(), "git init failed: {}", String::from_utf8_lossy(&init.stderr));
        write_worktree_exclude_file(&temp_dir).expect("write exclude file");
        let exclude_path = resolve_worktree_git_path(&temp_dir, "info/exclude").expect("resolve exclude path");
        let content = fs::read_to_string(exclude_path).expect("read exclude file");
        assert!(content.contains("triumvirate-daemon-managed"));
        assert!(content.contains("node_modules/"));
        assert!(content.contains("pnpm-store/"));
        assert!(content.contains(".triumvirate/logs/"));
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn commit_format_matches_literal_subject_before_regex() {
        let subject = "feat(T-305b) [model=codex] [worker=W3-T-305b-scoring-reconcile]: rewrite health + opportunity scoring to match Python algorithms per R46/T-304";
        assert!(commit_message_matches_format(&format!("{subject}\n"), subject));
    }

    #[test]
    fn commit_format_preserves_legacy_regex_anchors() {
        assert!(commit_message_matches_format(
            "T-003: implement worker change",
            "^T-003:"
        ));
        assert!(!commit_message_matches_format(
            "featT m",
            "feat(T-305b) [model=codex]"
        ));
    }
}

#[instrument(skip_all, fields(task_id = %req.task_id))]
pub async fn get_task_status<T: AbeTaskTracker>(
    tracker: T,
    req: GetTaskStatusRequest,
) -> Result<GetTaskStatusResponse, String> {
    tracker
        .get_status(req.task_id.clone())
        .await
        .ok_or_else(|| format!("unknown task_id: {}", req.task_id))
}

#[instrument(skip_all, fields(task_id = %req.task_id))]
pub async fn get_task_output<T: AbeTaskTracker>(
    tracker: T,
    req: GetTaskOutputRequest,
) -> Result<GetTaskOutputResponse, String> {
    tracker
        .get_output(req.task_id.clone())
        .await
        .ok_or_else(|| format!("task output unavailable for task_id: {}", req.task_id))
}

#[instrument(skip_all, fields(task_id = %req.task_id))]
pub async fn cancel_task<T: AbeTaskTracker>(
    tracker: T,
    req: AbeCancelTaskRequest,
) -> Result<AbeCancelTaskResponse, String> {
    tracker
        .cancel(req.task_id.clone())
        .await
        .ok_or_else(|| format!("unknown task_id: {}", req.task_id))
}

#[cfg(test)]
mod sandbox_permissions_tests {
    use super::{append_codex_exec_mcp_compat_args, build_sandbox_permission_args};

    #[test]
    fn none_emits_no_args() {
        assert!(build_sandbox_permission_args(None).is_empty());
    }

    #[test]
    fn empty_list_emits_no_args() {
        let empty: Vec<String> = Vec::new();
        assert!(build_sandbox_permission_args(Some(&empty)).is_empty());
    }

    #[test]
    fn network_full_access_maps_to_codex_config_key() {
        let perms = vec!["network-full-access".to_string()];
        let args = build_sandbox_permission_args(Some(&perms));
        assert_eq!(
            args,
            vec![
                "-c".to_string(),
                "sandbox_workspace_write.network_access=true".to_string(),
            ]
        );
    }

    #[test]
    fn unknown_permission_is_skipped_not_passed_through() {
        let perms = vec!["completely-made-up-permission".to_string()];
        let args = build_sandbox_permission_args(Some(&perms));
        assert!(
            args.is_empty(),
            "unknown perms must be dropped (with warn log), not emitted raw"
        );
    }

    #[test]
    fn known_and_unknown_mix_emits_only_known() {
        let perms = vec![
            "network-full-access".to_string(),
            "disk-full-read-access".to_string(), // not yet mapped → skip
        ];
        let args = build_sandbox_permission_args(Some(&perms));
        assert_eq!(
            args,
            vec![
                "-c".to_string(),
                "sandbox_workspace_write.network_access=true".to_string(),
            ]
        );
    }

    #[test]
    fn codex_exec_mcp_compat_args_disable_mcp_elicitation() {
        let mut args = vec!["exec".to_string(), "--full-auto".to_string()];
        append_codex_exec_mcp_compat_args(&mut args);
        assert_eq!(
            args,
            vec![
                "exec".to_string(),
                "--full-auto".to_string(),
                "--disable".to_string(),
                "tool_call_mcp_elicitation".to_string(),
            ]
        );
    }
}

/// Build the argv for an ABE worker, per agent.
///
/// ABE was Codex-only: both dispatch sites called `codex_command()` and then appended codex
/// `exec` flags inline. That was never a grok-specific gap, it excluded gemini and claude too,
/// and `task_tracker` still reports `agent: "codex"` on lifecycle events for the same reason.
///
/// The codex branch is byte-for-byte what those call sites built before, so an ABE dispatch that
/// worked yesterday produces the same process today. New agents are additive.
///
/// grok reuses `build_grok_invocation`, the SAME builder the consult and fleet paths use, so an
/// ABE worker inherits the forbidden-flag guard and the session-flag rules instead of assembling
/// a third divergent argv. It gets `--sandbox workspace` rather than the consult default of
/// `read-only`, because an ABE worker exists to WRITE code in its own worktree.
pub fn build_worker_argv(
    agent: &str,
    codex_command: &(dyn Fn() -> (String, Vec<String>) + Send + Sync),
    prompt: &str,
    cwd: &str,
    full_auto: bool,
) -> Result<(String, Vec<String>), String> {
    match mcp_bridge::normalize_agent_name(agent).as_str() {
        "grok" => {
            let (bin, extra) = mcp_bridge::grok_command();
            // An ABE worker is single-turn in a fresh worktree: no session id, so it can never
            // attach to another task's conversation. `workspace` rather than the consult default
            // of `read-only`, because this worker exists to WRITE code. Passed explicitly rather
            // than through env, which would race in a threaded daemon.
            let inv = mcp_bridge::grok::build_grok_invocation_with_sandbox(
                &bin, &extra, prompt, cwd, None, false, Some("workspace"),
            )
            .map_err(|e| format!("failed to assemble grok ABE worker: {e}"))?;
            Ok((inv.program, inv.args))
        }
        // Codex, and the default. Identical to what the call sites built inline.
        _ => {
            let (cmd, mut args) = codex_command();
            args.push("exec".to_string());
            if full_auto {
                args.push("--full-auto".to_string());
            } else {
                // 0.145 deprecated `--full-auto`; this is the explicit equivalent it resolves to.
                args.push("--sandbox".to_string());
                args.push("workspace-write".to_string());
                args.push("--ask-for-approval".to_string());
                args.push("never".to_string());
            }
            append_codex_exec_mcp_compat_args(&mut args);
            if !full_auto {
                args.push("--skip-git-repo-check".to_string());
                args.push(prompt.to_string());
            }
            Ok((cmd, args))
        }
    }
}

/// Which agent ABE dispatches workers as. Defaults to codex, which is what it always did.
///
/// ABE hardcoded codex at both dispatch sites, so gemini, claude and grok were all equally
/// excluded. This makes the choice explicit and overridable without changing the default.
pub fn abe_worker_agent() -> String {
    let raw = std::env::var("TRIUMVIRATE_ABE_AGENT").unwrap_or_else(|_| "codex".to_string());
    let canonical = mcp_bridge::normalize_agent_name(&raw);
    if mcp_bridge::is_supported_agent_name(&canonical) {
        canonical
    } else {
        tracing::warn!(requested = %raw, "TRIUMVIRATE_ABE_AGENT is not a dispatchable agent; using codex");
        "codex".to_string()
    }
}

#[cfg(test)]
mod abe_agent_tests {
    use super::{abe_worker_agent, build_worker_argv};

    fn codex_cmd() -> (String, Vec<String>) {
        ("codex".to_string(), Vec::new())
    }

    /// ABE defaulted to codex and must KEEP defaulting to codex. A dispatch that worked before
    /// this change must produce the same process.
    #[test]
    fn u_abe_01_codex_argv_is_unchanged() {
        let (cmd, args) = build_worker_argv("codex", &codex_cmd, "do the task", "/tmp", false)
            .expect("codex must build");
        assert_eq!(cmd, "codex");
        assert_eq!(args[0], "exec");
        assert!(args.contains(&"--sandbox".to_string()));
        assert!(args.contains(&"workspace-write".to_string()));
        assert!(args.contains(&"--ask-for-approval".to_string()));
        assert!(args.contains(&"never".to_string()));
        assert!(args.contains(&"--skip-git-repo-check".to_string()));
        assert_eq!(args.last().unwrap(), "do the task");
    }

    /// grok reuses the SAME builder the consult and fleet paths use, so an ABE worker inherits
    /// the forbidden-flag guard rather than assembling a third divergent argv.
    #[test]
    fn u_abe_02_grok_worker_uses_the_shared_builder() {
        let (cmd, args) = build_worker_argv("grok", &codex_cmd, "do the task", "/tmp", false)
            .expect("grok must build");
        assert!(cmd.ends_with("grok"), "must spawn grok, not codex: {cmd}");
        assert!(args.contains(&"--output-format".to_string()));
        assert!(args.contains(&"streaming-json".to_string()));
        let n = args.len();
        assert_eq!(args[n - 2], "-p");
        assert_eq!(args[n - 1], "do the task");
        // An ABE worker is single-turn in a fresh worktree: no session flags, so it can never
        // attach to another task's conversation.
        assert!(!args.contains(&"--resume".to_string()));
        assert!(!args.contains(&"--session-id".to_string()));
    }

    /// An ABE worker exists to WRITE code, unlike a consult, so its containment differs.
    #[test]
    fn u_abe_03_grok_worker_gets_a_writable_sandbox_not_read_only() {
        let (_, args) = build_worker_argv("grok", &codex_cmd, "task", "/tmp", false).unwrap();
        let i = args.iter().position(|a| a == "--sandbox").expect("must be contained");
        assert_eq!(args[i + 1], "workspace", "an ABE worker must be able to write its worktree");
    }

    /// Aliases resolve, and an unknown agent falls back to codex rather than failing a dispatch.
    #[test]
    fn u_abe_04_agent_selection_normalizes_and_defaults_safely() {
        // SAFETY: single-threaded test, restored below.
        unsafe { std::env::remove_var("TRIUMVIRATE_ABE_AGENT") };
        assert_eq!(abe_worker_agent(), "codex", "the default must not change");

        // SAFETY: as above.
        unsafe { std::env::set_var("TRIUMVIRATE_ABE_AGENT", "supergrok") };
        assert_eq!(abe_worker_agent(), "grok", "aliases must resolve");

        // SAFETY: as above.
        unsafe { std::env::set_var("TRIUMVIRATE_ABE_AGENT", "not-an-agent") };
        assert_eq!(abe_worker_agent(), "codex", "a bad value must not break dispatch");

        // SAFETY: as above.
        unsafe { std::env::remove_var("TRIUMVIRATE_ABE_AGENT") };
    }

    /// The grok branch temporarily sets TRIUMVIRATE_GROK_SANDBOX; it must restore the operator's
    /// value so a consult after an ABE dispatch is not silently re-contained.
    #[test]
    fn u_abe_05_building_a_worker_does_not_leak_env_state() {
        // SAFETY: single-threaded test.
        unsafe { std::env::set_var("TRIUMVIRATE_GROK_SANDBOX", "strict") };
        let _ = build_worker_argv("grok", &codex_cmd, "task", "/tmp", false).unwrap();
        assert_eq!(
            std::env::var("TRIUMVIRATE_GROK_SANDBOX").ok().as_deref(),
            Some("strict"),
            "the operator's sandbox setting must survive an ABE build"
        );
        // SAFETY: as above.
        unsafe { std::env::remove_var("TRIUMVIRATE_GROK_SANDBOX") };
        let _ = build_worker_argv("grok", &codex_cmd, "task", "/tmp", false).unwrap();
        assert!(std::env::var("TRIUMVIRATE_GROK_SANDBOX").is_err(), "must not leave a value behind");
    }
}
