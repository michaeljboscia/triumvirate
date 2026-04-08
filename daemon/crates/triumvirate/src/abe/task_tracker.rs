use std::{
    collections::HashMap,
    path::PathBuf,
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
    ) {
        let mut guard = self.inner.lock().await;
        if let Some(task) = guard.get_mut(task_id) {
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
        }
    }

    pub async fn mark_failed(&self, task_id: &str, exit_code: Option<i32>, error_message: String) {
        let mut guard = self.inner.lock().await;
        if let Some(task) = guard.get_mut(task_id) {
            task.status = TaskStatus::Failed;
            task.exit_code = exit_code;
            task.error_message = Some(error_message);
            task.child = None;
        }
    }

    pub async fn mark_timeout(&self, task_id: &str) {
        let mut guard = self.inner.lock().await;
        if let Some(task) = guard.get_mut(task_id) {
            task.status = TaskStatus::Timeout;
            task.error_message = Some("task timed out".to_string());
            task.child = None;
        }
    }

    pub async fn mark_setup_failed(&self, task_id: &str, error_message: String) {
        let mut guard = self.inner.lock().await;
        if let Some(task) = guard.get_mut(task_id) {
            task.status = TaskStatus::SetupFailed;
            task.error_message = Some(error_message);
            task.child = None;
        }
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
        let child = {
            let guard = self.inner.lock().await;
            guard
                .get(task_id)
                .and_then(|task| task.child.as_ref().cloned())
        };
        if let Some(child) = child {
            let mut child = child.lock().await;
            let _ = child.start_kill();
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
}
