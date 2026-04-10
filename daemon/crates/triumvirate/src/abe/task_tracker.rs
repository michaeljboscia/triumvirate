use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use daemon_core::{encode_ws_event, metrics::DaemonMetrics};
use shared_types::{
    CancelTaskResponse, GetTaskOutputResponse, GetTaskStatusResponse, TaskStatus,
};
use tokio::{process::Child, sync::{Mutex, broadcast}};
use tracing::instrument;

#[derive(Debug)]
struct TaskOutput {
    commit_sha: String,
    modified_files: Vec<String>,
    stdout: String,
    validation_log: Option<String>,
    test_output: Option<String>,
}

#[derive(Debug)]
struct TaskRecord {
    wave: u32,
    status: TaskStatus,
    started_at: Instant,
    commit_sha: Option<String>,
    exit_code: Option<i32>,
    error_message: Option<String>,
    output: Option<TaskOutput>,
    child: Option<Arc<Mutex<Child>>>,
    worktree_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct TaskTracker {
    inner: Arc<Mutex<HashMap<String, TaskRecord>>>,
    metrics: Option<Arc<DaemonMetrics>>,
    ws_events: Option<broadcast::Sender<String>>,
}

impl Default for TaskTracker {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            metrics: None,
            ws_events: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionOutcome {
    Transitioned,
    AlreadyTerminal,
    NotFound,
}

fn is_terminal(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Completed
            | TaskStatus::Stuck
            | TaskStatus::Failed
            | TaskStatus::Timeout
            | TaskStatus::SetupFailed
            | TaskStatus::Cancelled
    )
}

impl TaskTracker {
    pub fn with_metrics(metrics: Arc<DaemonMetrics>) -> Self {
        Self::with_observability(metrics, None)
    }

    pub fn with_observability(
        metrics: Arc<DaemonMetrics>,
        ws_events: Option<broadcast::Sender<String>>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            metrics: Some(metrics),
            ws_events,
        }
    }

    fn inc_dispatch_status(&self, status: &'static str) {
        if let Some(metrics) = &self.metrics {
            metrics
                .abe_task_dispatch_total
                .with_label_values(&[status])
                .inc();
        }
    }

    fn emit_task_state(
        &self,
        task_id: &str,
        wave: u32,
        status: &'static str,
        duration_ms: u128,
        commit_sha: Option<&str>,
    ) {
        let Some(ws_events) = &self.ws_events else {
            return;
        };
        let payload = serde_json::json!({
            "task_id": task_id,
            "wave": wave,
            "status": status,
            "duration_ms": duration_ms,
            "commit_sha": commit_sha,
        });
        let _ = ws_events.send(encode_ws_event("abe_task_state", payload));
    }

    pub async fn register(
        &self,
        task_id: String,
        wave: u32,
        child: Arc<Mutex<Child>>,
        worktree_path: Option<PathBuf>,
    ) {
        let mut guard = self.inner.lock().await;
        let started_at = Instant::now();
        guard.insert(
            task_id.clone(),
            TaskRecord {
                wave,
                status: TaskStatus::Working,
                started_at,
                commit_sha: None,
                exit_code: None,
                error_message: None,
                output: None,
                child: Some(child),
                worktree_path,
            },
        );
        self.emit_task_state(&task_id, wave, "dispatched", 0, None);
        self.emit_task_state(&task_id, wave, "running", 0, None);
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "completed"))]
    pub async fn mark_completed(
        &self,
        task_id: &str,
        commit_sha: String,
        modified_files: Vec<String>,
        stdout: String,
        validation_log: Option<String>,
        test_output: Option<String>,
    ) -> TransitionOutcome {
        let mut guard = self.inner.lock().await;
        let Some(task) = guard.get_mut(task_id) else {
            return TransitionOutcome::NotFound;
        };
        if is_terminal(&task.status) {
            return TransitionOutcome::AlreadyTerminal;
        }
        task.status = TaskStatus::Completed;
        task.commit_sha = Some(commit_sha.clone());
        let duration_ms = task.started_at.elapsed().as_millis();
        task.output = Some(TaskOutput {
            commit_sha,
            modified_files,
            stdout,
            validation_log,
            test_output,
        });
        task.child = None;
        self.inc_dispatch_status("completed");
        tracing::info!(
            task_id = %task_id,
            commit_sha = task.commit_sha.as_deref().unwrap_or_default(),
            duration_ms = task.started_at.elapsed().as_millis() as u64,
            "abe_task_completed"
        );
        self.emit_task_state(
            task_id,
            task.wave,
            "completed",
            duration_ms,
            task.commit_sha.as_deref(),
        );
        TransitionOutcome::Transitioned
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "failed"))]
    pub async fn mark_failed(
        &self,
        task_id: &str,
        exit_code: Option<i32>,
        error_message: String,
    ) -> TransitionOutcome {
        let mut guard = self.inner.lock().await;
        let Some(task) = guard.get_mut(task_id) else {
            return TransitionOutcome::NotFound;
        };
        if is_terminal(&task.status) {
            return TransitionOutcome::AlreadyTerminal;
        }
        task.status = TaskStatus::Failed;
        let duration_ms = task.started_at.elapsed().as_millis();
        task.exit_code = exit_code;
        task.error_message = Some(error_message);
        task.child = None;
        self.inc_dispatch_status("failed");
        self.emit_task_state(task_id, task.wave, "failed", duration_ms, task.commit_sha.as_deref());
        TransitionOutcome::Transitioned
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "timeout"))]
    pub async fn mark_timeout(&self, task_id: &str) -> TransitionOutcome {
        let mut guard = self.inner.lock().await;
        let Some(task) = guard.get_mut(task_id) else {
            return TransitionOutcome::NotFound;
        };
        if is_terminal(&task.status) {
            return TransitionOutcome::AlreadyTerminal;
        }
        task.status = TaskStatus::Timeout;
        let duration_ms = task.started_at.elapsed().as_millis();
        task.error_message = Some("task timed out".to_string());
        task.child = None;
        self.inc_dispatch_status("timeout");
        self.emit_task_state(task_id, task.wave, "timeout", duration_ms, task.commit_sha.as_deref());
        TransitionOutcome::Transitioned
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "stuck"))]
    pub async fn mark_stuck(&self, task_id: &str, error_message: String) -> TransitionOutcome {
        let mut guard = self.inner.lock().await;
        let Some(task) = guard.get_mut(task_id) else {
            return TransitionOutcome::NotFound;
        };
        if is_terminal(&task.status) {
            return TransitionOutcome::AlreadyTerminal;
        }
        task.status = TaskStatus::Stuck;
        task.error_message = Some(error_message);
        task.child = None;
        TransitionOutcome::Transitioned
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "setup_failed"))]
    pub async fn mark_setup_failed(&self, task_id: &str, error_message: String) -> TransitionOutcome {
        let mut guard = self.inner.lock().await;
        let Some(task) = guard.get_mut(task_id) else {
            return TransitionOutcome::NotFound;
        };
        if is_terminal(&task.status) {
            return TransitionOutcome::AlreadyTerminal;
        }
        task.status = TaskStatus::SetupFailed;
        task.error_message = Some(error_message);
        task.child = None;
        TransitionOutcome::Transitioned
    }

    #[instrument(skip_all, fields(task_id = %task_id))]
    pub async fn get_status(&self, task_id: &str) -> Option<GetTaskStatusResponse> {
        let guard = self.inner.lock().await;
        let task = guard.get(task_id)?;
        Some(GetTaskStatusResponse {
            task_id: task_id.to_string(),
            status: task.status.clone(),
            elapsed_sec: Some(task.started_at.elapsed().as_secs()),
            commit_sha: task.commit_sha.clone(),
            exit_code: task.exit_code,
            error_message: task.error_message.clone(),
        })
    }

    #[instrument(skip_all, fields(task_id = %task_id))]
    pub async fn get_output(&self, task_id: &str) -> Option<GetTaskOutputResponse> {
        let guard = self.inner.lock().await;
        let task = guard.get(task_id)?;
        let output = task.output.as_ref()?;
        Some(GetTaskOutputResponse {
            task_id: task_id.to_string(),
            commit_sha: output.commit_sha.clone(),
            modified_files: output.modified_files.clone(),
            stdout: output.stdout.clone(),
            validation_log: output.validation_log.clone(),
            test_output: output.test_output.clone(),
        })
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "cancelled"))]
    pub async fn cancel(&self, task_id: &str) -> Option<CancelTaskResponse> {
        let (child, worktree_path, already_terminal) = {
            let guard = self.inner.lock().await;
            let task = guard.get(task_id)?;
            (
                task.child.as_ref().cloned(),
                task.worktree_path.clone(),
                is_terminal(&task.status),
            )
        };
        if already_terminal {
            let guard = self.inner.lock().await;
            let task = guard.get(task_id)?;
            return Some(CancelTaskResponse {
                task_id: task_id.to_string(),
                status: "already-terminal".to_string(),
                worktree_path: task.worktree_path.as_ref().map(|p| p.display().to_string()),
            });
        }
        if let Some(child) = child {
            let mut child = child.lock().await;
            if child.try_wait().ok().flatten().is_none() {
                if let Some(pid) = child.id() {
                    let _ = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
                }
                tokio::time::sleep(Duration::from_secs(10)).await;
                if child.try_wait().ok().flatten().is_none() {
                    let _ = child.kill().await;
                }
            }
        }
        if let Some(worktree_path) = worktree_path.as_ref() {
            cleanup_git_locks(worktree_path);
        }

        let mut guard = self.inner.lock().await;
        let task = guard.get_mut(task_id)?;
        task.status = TaskStatus::Cancelled;
        let duration_ms = task.started_at.elapsed().as_millis();
        task.child = None;
        self.inc_dispatch_status("cancelled");
        self.emit_task_state(task_id, task.wave, "cancelled", duration_ms, task.commit_sha.as_deref());

        Some(CancelTaskResponse {
            task_id: task_id.to_string(),
            status: "cancelled".to_string(),
            worktree_path: task
                .worktree_path
                .as_ref()
                .map(|p| p.display().to_string()),
        })
    }

    #[instrument(skip_all, fields(task_id = %task_id))]
    pub async fn exists(&self, task_id: &str) -> bool {
        let guard = self.inner.lock().await;
        guard.contains_key(task_id)
    }

    #[instrument(skip_all, fields(task_id = %task_id))]
    pub async fn worktree_path_for(&self, task_id: &str) -> Option<PathBuf> {
        let guard = self.inner.lock().await;
        guard.get(task_id).and_then(|task| task.worktree_path.clone())
    }

    #[instrument(skip_all, fields(task_id = %task_id, status = "setup_failed"))]
    pub async fn register_setup_failed(&self, task_id: String, error_message: String) {
        let mut guard = self.inner.lock().await;
        guard.insert(
            task_id.clone(),
            TaskRecord {
                wave: 0,
                status: TaskStatus::SetupFailed,
                started_at: Instant::now(),
                commit_sha: None,
                exit_code: None,
                error_message: Some(error_message),
                output: None,
                child: None,
                worktree_path: None,
            },
        );
        self.emit_task_state(&task_id, 0, "failed", 0, None);
    }

    #[instrument(skip_all, fields(task_id = %task_id))]
    pub async fn elapsed_for(&self, task_id: &str) -> Option<Duration> {
        let guard = self.inner.lock().await;
        guard.get(task_id).map(|r| r.started_at.elapsed())
    }

    #[instrument(skip_all, fields(task_id = %task_id))]
    pub async fn status_for(&self, task_id: &str) -> Option<TaskStatus> {
        let guard = self.inner.lock().await;
        guard.get(task_id).map(|r| r.status.clone())
    }
}

fn cleanup_git_locks(worktree_path: &Path) {
    let git_dir = resolve_git_dir(worktree_path);
    let _ = fs::remove_file(git_dir.join("index.lock"));
    let _ = fs::remove_file(worktree_path.join(".git").join("index.lock"));
    let _ = fs::remove_file(worktree_path.join(".git/index.lock"));
}

fn resolve_git_dir(worktree_path: &Path) -> PathBuf {
    let dot_git = worktree_path.join(".git");
    if dot_git.is_file() {
        let content = fs::read_to_string(&dot_git).unwrap_or_default();
        if let Some(gitdir) = content.lines().find_map(|line| line.strip_prefix("gitdir:")) {
            let raw = gitdir.trim();
            let parsed = PathBuf::from(raw);
            if parsed.is_absolute() {
                return parsed;
            }
            return worktree_path.join(parsed);
        }
    }
    dot_git
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use shared_types::TaskStatus;

    use super::{TaskTracker, TransitionOutcome};
    use tokio::{process::Command, sync::Mutex};

    async fn register_working_task(tracker: &TaskTracker, task_id: &str) {
        let child = Command::new("sh")
            .arg("-c")
            .arg("true")
            .spawn()
            .expect("spawn child");
        tracker
            .register(
                task_id.to_string(),
                1,
                Arc::new(Mutex::new(child)),
                None,
            )
            .await;
    }

    #[tokio::test]
    async fn register_creates_working_task() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-01").await;

        let status = tracker.get_status("U-TT-01").await.expect("task status");
        assert_eq!(status.status, TaskStatus::Working);
    }

    #[tokio::test]
    async fn mark_completed_transitions_to_completed() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-02").await;

        let outcome = tracker
            .mark_completed(
                "U-TT-02",
                "abc123".to_string(),
                vec!["src/lib.rs".to_string()],
                "done".to_string(),
                Some("validation ok".to_string()),
                Some("tests ok".to_string()),
            )
            .await;
        assert_eq!(outcome, TransitionOutcome::Transitioned);

        let status = tracker.get_status("U-TT-02").await.expect("task status");
        assert_eq!(status.status, TaskStatus::Completed);
        assert_eq!(status.commit_sha.as_deref(), Some("abc123"));
    }

    #[tokio::test]
    async fn mark_failed_transitions_to_failed() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-03").await;

        let outcome = tracker
            .mark_failed("U-TT-03", Some(1), "boom".to_string())
            .await;
        assert_eq!(outcome, TransitionOutcome::Transitioned);

        let status = tracker.get_status("U-TT-03").await.expect("task status");
        assert_eq!(status.status, TaskStatus::Failed);
        assert_eq!(status.exit_code, Some(1));
        assert_eq!(status.error_message.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn mark_timeout_transitions_to_timeout() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-04").await;

        let outcome = tracker.mark_timeout("U-TT-04").await;
        assert_eq!(outcome, TransitionOutcome::Transitioned);

        let status = tracker.get_status("U-TT-04").await.expect("task status");
        assert_eq!(status.status, TaskStatus::Timeout);
    }

    #[tokio::test]
    async fn mark_stuck_transitions_to_stuck() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-05").await;

        let outcome = tracker
            .mark_stuck("U-TT-05", "stuck waiting for signal".to_string())
            .await;
        assert_eq!(outcome, TransitionOutcome::Transitioned);

        let status = tracker.get_status("U-TT-05").await.expect("task status");
        assert_eq!(status.status, TaskStatus::Stuck);
        assert_eq!(
            status.error_message.as_deref(),
            Some("stuck waiting for signal")
        );
    }

    #[tokio::test]
    async fn double_transition_returns_already_terminal() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-06").await;

        let first = tracker
            .mark_completed(
                "U-TT-06",
                "def456".to_string(),
                Vec::new(),
                String::new(),
                None,
                None,
            )
            .await;
        assert_eq!(first, TransitionOutcome::Transitioned);

        let second = tracker
            .mark_failed("U-TT-06", Some(2), "should fail transition".to_string())
            .await;
        assert_eq!(second, TransitionOutcome::AlreadyTerminal);
    }

    #[tokio::test]
    async fn unknown_task_returns_not_found() {
        let tracker = TaskTracker::default();

        let outcome = tracker
            .mark_completed(
                "U-TT-07-missing",
                "nope".to_string(),
                Vec::new(),
                String::new(),
                None,
                None,
            )
            .await;
        assert_eq!(outcome, TransitionOutcome::NotFound);
    }

    #[tokio::test]
    async fn cancel_transitions_task_to_cancelled() {
        let tracker = TaskTracker::default();
        register_working_task(&tracker, "U-TT-08").await;

        let response = tracker.cancel("U-TT-08").await.expect("cancel response");
        assert_eq!(response.status, "cancelled");

        let status = tracker.get_status("U-TT-08").await.expect("task status");
        assert_eq!(status.status, TaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn cancel_cleans_git_index_lock() {
        let tracker = TaskTracker::default();
        let tmp = tempfile::tempdir().expect("tmp");
        let wt = tmp.path().join("worktree");
        std::fs::create_dir_all(wt.join(".git")).expect("mkdir");
        let lock = wt.join(".git").join("index.lock");
        std::fs::write(&lock, "lock").expect("write lock");

        let child = Command::new("sh")
            .arg("-c")
            .arg("sleep 5")
            .spawn()
            .expect("spawn");
        tracker
            .register(
                "T-LOCK".to_string(),
                0,
                Arc::new(Mutex::new(child)),
                Some(wt.clone()),
            )
            .await;

        let _ = tracker.cancel("T-LOCK").await;
        assert!(!lock.exists(), "expected index.lock to be removed on cancel");
    }
}
