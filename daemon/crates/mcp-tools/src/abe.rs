use shared_types::{
    CancelTaskRequest as AbeCancelTaskRequest, CancelTaskResponse as AbeCancelTaskResponse,
    ContractFields, DispatchCodexRequest, DispatchCodexResponse, DispatchCodexWorktreeRequest,
    DispatchCodexWorktreeResponse, GetTaskOutputRequest, GetTaskOutputResponse,
    GetTaskStatusRequest, GetTaskStatusResponse,
};
use std::{
    collections::HashMap,
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};
use tokio::{process::Child, sync::Mutex};
use uuid::Uuid;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

pub trait AbeTaskTracker: Clone + Send + Sync + 'static {
    fn register(
        &self,
        task_id: String,
        child: Arc<Mutex<Child>>,
        worktree_path: Option<PathBuf>,
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
}

pub async fn dispatch_codex<T: AbeTaskTracker>(
    tracker: T,
    req: DispatchCodexRequest,
    callbacks: AbeCallbacks,
) -> Result<DispatchCodexResponse, String> {
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
        .register(task_id.clone(), child.clone(), None)
        .await;

    let tracker_for_monitor = tracker.clone();
    let callbacks_for_monitor = callbacks.clone();
    let task_id_for_monitor = task_id.clone();
    tokio::spawn(async move {
        let timed_out = (callbacks_for_monitor.enforce_timeout)(
            child.clone(),
            timeout_sec,
            PathBuf::from(&cwd),
        )
        .await
        .unwrap_or(false);
        if timed_out {
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
                tracker_for_monitor
                    .mark_failed(
                        task_id_for_monitor.clone(),
                        exit.code(),
                        "codex process exited without creating a commit".to_string(),
                    )
                    .await;
                return;
            }
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

pub async fn dispatch_codex_worktree<T: AbeTaskTracker>(
    tracker: T,
    req: DispatchCodexWorktreeRequest,
    callbacks: AbeCallbacks,
) -> Result<DispatchCodexWorktreeResponse, String> {
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

    let child = (callbacks.spawn_background)(SpawnSpec {
        cmd,
        args,
        cwd: setup.worktree_path.display().to_string(),
        envs: HashMap::from([(
            "CARGO_TARGET_DIR".to_string(),
            setup.worktree_path
                .join(".triumvirate")
                .join("target")
                .join(task_id.clone())
                .display()
                .to_string(),
        )]),
    })
    .await
    .map_err(|e| format!("dispatch_codex_worktree failed: {e}"))?;

    tracker
        .register(task_id.clone(), child.clone(), Some(setup.worktree_path.clone()))
        .await;

    let tracker_for_monitor = tracker.clone();
    let callbacks_for_monitor = callbacks.clone();
    let worktree_path = setup.worktree_path.clone();
    let timeout_sec = req.contract_fields.task_timeout_sec;
    let keep_failed = req.keep_failed_worktree.unwrap_or(false);
    let project_root_for_cleanup = project_root.clone();
    let task_id_for_monitor = task_id.clone();
    let start_sha = req.sha.clone();
    let contract_for_validation = req.contract_fields.clone();
    tokio::spawn(async move {
        let timed_out = (callbacks_for_monitor.enforce_timeout)(
            child.clone(),
            timeout_sec,
            worktree_path.clone(),
        )
        .await
        .unwrap_or(false);
        if timed_out {
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
            let (commit_sha, files) =
                (callbacks_for_monitor.resolve_commit_outputs)(&worktree_path, &start_sha);
            if commit_sha.is_empty() {
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
        } else {
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

pub async fn get_task_status<T: AbeTaskTracker>(
    tracker: T,
    req: GetTaskStatusRequest,
) -> Result<GetTaskStatusResponse, String> {
    tracker
        .get_status(req.task_id.clone())
        .await
        .ok_or_else(|| format!("unknown task_id: {}", req.task_id))
}

pub async fn get_task_output<T: AbeTaskTracker>(
    tracker: T,
    req: GetTaskOutputRequest,
) -> Result<GetTaskOutputResponse, String> {
    tracker
        .get_output(req.task_id.clone())
        .await
        .ok_or_else(|| format!("task output unavailable for task_id: {}", req.task_id))
}

pub async fn cancel_task<T: AbeTaskTracker>(
    tracker: T,
    req: AbeCancelTaskRequest,
) -> Result<AbeCancelTaskResponse, String> {
    tracker
        .cancel(req.task_id.clone())
        .await
        .ok_or_else(|| format!("unknown task_id: {}", req.task_id))
}
