use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use shared_types::{
    CancelTaskResponse, GetTaskOutputResponse, GetTaskStatusResponse, TaskStatus,
};
use tokio::{process::Child, sync::Mutex};

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
    status: TaskStatus,
    started_at: Instant,
    commit_sha: Option<String>,
    exit_code: Option<i32>,
    error_message: Option<String>,
    output: Option<TaskOutput>,
    child: Option<Arc<Mutex<Child>>>,
    worktree_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct TaskTracker {
    inner: Arc<Mutex<HashMap<String, TaskRecord>>>,
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
    pub async fn register(
        &self,
        task_id: String,
        child: Arc<Mutex<Child>>,
        worktree_path: Option<PathBuf>,
    ) {
        let mut guard = self.inner.lock().await;
        guard.insert(
            task_id,
            TaskRecord {
                status: TaskStatus::Working,
                started_at: Instant::now(),
                commit_sha: None,
                exit_code: None,
                error_message: None,
                output: None,
                child: Some(child),
                worktree_path,
            },
        );
    }

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
        task.output = Some(TaskOutput {
            commit_sha,
            modified_files,
            stdout,
            validation_log,
            test_output,
        });
        task.child = None;
        TransitionOutcome::Transitioned
    }

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
        task.exit_code = exit_code;
        task.error_message = Some(error_message);
        task.child = None;
        TransitionOutcome::Transitioned
    }

    pub async fn mark_timeout(&self, task_id: &str) -> TransitionOutcome {
        let mut guard = self.inner.lock().await;
        let Some(task) = guard.get_mut(task_id) else {
            return TransitionOutcome::NotFound;
        };
        if is_terminal(&task.status) {
            return TransitionOutcome::AlreadyTerminal;
        }
        task.status = TaskStatus::Timeout;
        task.error_message = Some("task timed out".to_string());
        task.child = None;
        TransitionOutcome::Transitioned
    }

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
        task.child = None;

        Some(CancelTaskResponse {
            task_id: task_id.to_string(),
            status: "cancelled".to_string(),
            worktree_path: task
                .worktree_path
                .as_ref()
                .map(|p| p.display().to_string()),
        })
    }

    pub async fn exists(&self, task_id: &str) -> bool {
        let guard = self.inner.lock().await;
        guard.contains_key(task_id)
    }

    pub async fn worktree_path_for(&self, task_id: &str) -> Option<PathBuf> {
        let guard = self.inner.lock().await;
        guard.get(task_id).and_then(|task| task.worktree_path.clone())
    }

    pub async fn register_setup_failed(&self, task_id: String, error_message: String) {
        let mut guard = self.inner.lock().await;
        guard.insert(
            task_id,
            TaskRecord {
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
    }

    pub async fn elapsed_for(&self, task_id: &str) -> Option<Duration> {
        let guard = self.inner.lock().await;
        guard.get(task_id).map(|r| r.started_at.elapsed())
    }

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
    use super::TaskTracker;
    use tokio::{process::Command, sync::Mutex};

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
                std::sync::Arc::new(Mutex::new(child)),
                Some(wt.clone()),
            )
            .await;

        let _ = tracker.cancel("T-LOCK").await;
        assert!(!lock.exists(), "expected index.lock to be removed on cancel");
    }
}
