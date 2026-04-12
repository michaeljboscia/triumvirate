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
    fn register(
        &self,
        task_id: String,
        wave: u32,
        child: Arc<Mutex<Child>>,
        worktree_path: Option<PathBuf>,
        parent_session_id: Option<String>,
        root_session_id: Option<String>,
    ) -> BoxFuture<()>;

    fn mark_completed(
        &self,
        task_id: String,
        commit_sha: String,
        modified_files: Vec<String>,
        stdout: String,
        validation_log: Option<String>,
        test_output: Option<String>,
    ) -> BoxFuture<()>;

    fn mark_failed(
        &self,
        task_id: String,
        exit_code: Option<i32>,
        error_message: String,
    ) -> BoxFuture<()>;

    fn mark_timeout(&self, task_id: String) -> BoxFuture<()>;

    fn mark_stuck(&self, task_id: String, error_message: String) -> BoxFuture<()>;

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
    regex_lite::Regex::new(commit_format)
        .map(|re| re.is_match(commit_message))
        .unwrap_or(false)
}

fn parse_task_complete_file(path: &Path) -> Option<TaskCompleteRequest> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<TaskCompleteRequest>(&raw).ok()
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

#[instrument(skip_all)]
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
    let cwd = req
        .cwd
        .clone()
        .or_else(|| std::env::current_dir().ok().map(|p| p.display().to_string()))
        .ok_or_else(|| "failed to resolve cwd".to_string())?;
    let timeout_sec = req.timeout_sec.unwrap_or(600);

    let (cmd, mut args) = (callbacks.codex_command)();
    args.push("exec".to_string());
    args.push("--full-auto".to_string());
    args.push(req.prompt.clone());
    let start_sha = std::process::Command::new("git")
        .arg("-C")
        .arg(&cwd)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();

    let child = (callbacks.spawn_background)(SpawnSpec {
        cmd,
        args,
        cwd: cwd.clone(),
        envs: HashMap::new(),
    })
    .await
    .map_err(|e| format!("dispatch_codex failed: {e}"))?;

    tracker
        .register(task_id.clone(), 0, child.clone(), None)
        .await;

    let tracker_for_monitor = tracker.clone();
    let callbacks_for_monitor = callbacks.clone();
    let task_started_at = Instant::now();
    let task_id_for_monitor = task_id.clone();
    tokio::spawn(async move {
        let wave_label = "mcp";
        let timed_out = (callbacks_for_monitor.enforce_timeout)(
            child.clone(),
            timeout_sec,
            PathBuf::from(&cwd),
        )
        .await
        .unwrap_or(false);
        if timed_out {
            observe_task_duration(&callbacks_for_monitor.metrics, wave_label, task_started_at);
            tracker_for_monitor
                .mark_timeout(task_id_for_monitor.clone())
                .await;
            return;
        }
        let exit = {
            let mut locked = child.lock().await;
            match locked.wait().await {
                Ok(status) => status,
                Err(err) => {
                    tracker_for_monitor
                        .mark_failed(task_id_for_monitor.clone(), None, err.to_string())
                        .await;
                    return;
                }
            }
        };
        if exit.success() {
            let cwd_path = PathBuf::from(&cwd);
            let (commit_sha, files) = (callbacks_for_monitor.resolve_commit_outputs)(&cwd_path, &start_sha);
            if commit_sha.is_empty() {
                observe_task_duration(&callbacks_for_monitor.metrics, wave_label, task_started_at);
                tracker_for_monitor
                    .mark_failed(
                        task_id_for_monitor.clone(),
                        exit.code(),
                        "codex process exited without creating a commit".to_string(),
                    )
                    .await;
                return;
            }
            observe_task_duration(&callbacks_for_monitor.metrics, wave_label, task_started_at);
            tracker_for_monitor
                .mark_completed(
                    task_id_for_monitor.clone(),
                    commit_sha,
                    files,
                    String::new(),
                    None,
                    None,
                )
                .await;
        } else {
            observe_task_duration(&callbacks_for_monitor.metrics, wave_label, task_started_at);
            tracker_for_monitor
                .mark_failed(
                    task_id_for_monitor.clone(),
                    exit.code(),
                    "codex process failed".to_string(),
                )
                .await;
        }
    });

    Ok(DispatchCodexResponse {
        task_id,
        status: "dispatched".to_string(),
    })
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

    let project_root = req
        .project_root
        .clone()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "failed to resolve project_root".to_string())?;
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
            tracker
                .register_setup_failed(task_id.clone(), err.to_string())
                .await;
            return Err(format!("SETUP_FAILED: {err}"));
        }
    };

    let prompt =
        "Read .triumvirate/BRIEFING.md and implement the task contract. Commit when complete.".to_string();
    let (cmd, mut args) = (callbacks.codex_command)();
    args.push("exec".to_string());
    args.push("--full-auto".to_string());

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

    let child = (callbacks.spawn_background)(SpawnSpec {
        cmd,
        args,
        cwd: setup.worktree_path.display().to_string(),
        envs: worker_env,
    })
    .await
    .map_err(|e| format!("dispatch_codex_worktree failed: {e}"))?;

    tracker
        .register(
            task_id.clone(),
            req.contract_fields.wave,
            child.clone(),
            Some(setup.worktree_path.clone()),
        )
        .await;

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
        tokio::spawn(async move {
            let timed_out = (timeout_callbacks.enforce_timeout)(
                timeout_child,
                timeout_sec,
                timeout_worktree,
            )
            .await
            .unwrap_or(false);
            if timed_out {
                observe_task_duration(&timeout_metrics, &timeout_wave_label, task_started_at);
                timeout_tracker.mark_timeout(timeout_task_id).await;
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
                sentinel_tracker
                    .mark_completed(
                        sentinel_task_id.clone(),
                        commit_sha,
                        files,
                        String::new(),
                        None,
                        None,
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
                head_tracker
                    .mark_completed(head_task_id.clone(), commit_sha, files, String::new(), None, None)
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
                stuck_tracker
                    .mark_stuck(
                        stuck_task_id.clone(),
                        "worker marked STUCK after 180s without filesystem activity".to_string(),
                    )
                    .await;
                observe_task_duration(&stuck_metrics, &stuck_wave_label, task_started_at);
                terminate_worker(stuck_child).await;
                break;
            }
        });

        let exit = {
            let mut locked = child.lock().await;
            match locked.wait().await {
                Ok(status) => status,
                Err(err) => {
                    tracker_for_monitor
                        .mark_failed(task_id_for_monitor.clone(), None, err.to_string())
                        .await;
                    return;
                }
            }
        };

        if exit.success() {
            let (commit_sha, files) =
                (callbacks_for_monitor.resolve_commit_outputs)(&worktree_path, &start_sha);
            if commit_sha.is_empty() {
                observe_task_duration(&callbacks_for_monitor.metrics, &wave_label, task_started_at);
                tracker_for_monitor
                    .mark_failed(
                        task_id_for_monitor.clone(),
                        exit.code(),
                        "codex process exited without creating a commit".to_string(),
                    )
                    .await;
                return;
            }

            let validation =
                (callbacks_for_monitor.validate_commit)(&worktree_path, &contract_for_validation, &start_sha);
            if !validation.passed {
                let violation_summary = validation.violations.join("; ");
                observe_task_duration(&callbacks_for_monitor.metrics, &wave_label, task_started_at);
                tracker_for_monitor
                    .mark_failed(
                        task_id_for_monitor.clone(),
                        None,
                        format!("DAEMON_VALIDATION_FAILED: {violation_summary}"),
                    )
                    .await;
                return;
            }

            let validation_log =
                fs::read_to_string(worktree_path.join(".triumvirate").join("VALIDATION_LOG.md")).ok();
            tracker_for_monitor
                .mark_completed(
                    task_id_for_monitor.clone(),
                    commit_sha,
                    files,
                    String::new(),
                    validation_log,
                    None,
                )
                .await;
            observe_task_duration(&callbacks_for_monitor.metrics, &wave_label, task_started_at);
        } else {
            observe_task_duration(&callbacks_for_monitor.metrics, &wave_label, task_started_at);
            tracker_for_monitor
                .mark_failed(
                    task_id_for_monitor.clone(),
                    exit.code(),
                    "codex process failed".to_string(),
                )
                .await;
            if !keep_failed {
                let _ = (callbacks_for_monitor.rollback_worktree)(
                    &project_root_for_cleanup,
                    &worktree_path,
                );
            }
        }
    });

    Ok(DispatchCodexWorktreeResponse {
        task_id,
        worktree_path: setup.worktree_path.display().to_string(),
        status: "dispatched".to_string(),
    })
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
