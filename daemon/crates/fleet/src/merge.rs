use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use ledger::LedgerStore;
use shared_types::ManualRecord;
use shared_types::{GitOps, MergeResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedWork {
    pub task_id: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeConflictRecord {
    pub task_id: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MergeCoordinator<G: GitOps> {
    git_ops: G,
    project_root: Option<PathBuf>,
    completion_queue: VecDeque<CompletedWork>,
    merged_order: Vec<String>,
    paused: bool,
    conflicts: Vec<MergeConflictRecord>,
    review_status: HashMap<String, ReviewGateState>,
    review_comments: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewGateState {
    Approved,
    Pending,
    RequestChanges,
}

impl<G: GitOps> MergeCoordinator<G> {
    pub fn new(git_ops: G) -> Self {
        Self {
            git_ops,
            project_root: None,
            completion_queue: VecDeque::new(),
            merged_order: Vec::new(),
            paused: false,
            conflicts: Vec::new(),
            review_status: HashMap::new(),
            review_comments: HashMap::new(),
        }
    }

    pub fn with_project_root(mut self, project_root: PathBuf) -> Self {
        self.project_root = Some(project_root);
        self
    }

    pub fn enqueue_completed(&mut self, task_id: impl Into<String>, branch: impl Into<String>) {
        self.completion_queue.push_back(CompletedWork {
            task_id: task_id.into(),
            branch: branch.into(),
        });
    }

    pub async fn merge_next(&mut self) -> anyhow::Result<Option<String>> {
        if self.paused {
            anyhow::bail!("merge queue paused due to previous conflict");
        }
        let Some(next) = self.completion_queue.pop_front() else {
            return Ok(None);
        };

        if !skip_review_enabled() {
            match self
                .review_status
                .get(&next.task_id)
                .copied()
                .unwrap_or(ReviewGateState::Pending)
            {
                ReviewGateState::Approved => {}
                ReviewGateState::Pending => {
                    self.completion_queue.push_front(next);
                    anyhow::bail!("pending review for task; merge blocked");
                }
                ReviewGateState::RequestChanges => {
                    self.paused = true;
                    let comment = self
                        .review_comments
                        .get(&next.task_id)
                        .cloned()
                        .unwrap_or_else(|| "request_changes".to_string());
                    self.completion_queue.push_front(next);
                    anyhow::bail!("merge paused due to request_changes: {comment}");
                }
            }
        } else if let Some(project_root) = self.project_root.clone() {
            let store = LedgerStore::open(project_root)?;
            let _ = store.record(ManualRecord {
                session_id: None,
                title: format!("review skipped for {}", next.task_id),
                narrative: "TRIUMVIRATE_FLEET_SKIP_REVIEW=1".to_string(),
                facts_json: None,
                concepts_json: None,
                affected_files_json: None,
                summary_type: "review_skipped".to_string(),
            });
        }

        match self.git_ops.merge(&next.branch).await? {
            MergeResult::Success => {
                self.merged_order.push(next.task_id.clone());
                Ok(Some(next.task_id))
            }
            MergeResult::Conflict { files } => {
                self.paused = true;
                self.conflicts.push(MergeConflictRecord {
                    task_id: next.task_id.clone(),
                    files: files.clone(),
                });
                anyhow::bail!(
                    "merge conflict while merging task {}: {}; queue paused",
                    next.task_id,
                    files.join(", ")
                );
            }
        }
    }

    pub fn merged_order(&self) -> &[String] {
        &self.merged_order
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn conflicts(&self) -> &[MergeConflictRecord] {
        &self.conflicts
    }

    pub fn set_review_status(
        &mut self,
        task_id: impl Into<String>,
        state: ReviewGateState,
        comments: Option<String>,
    ) {
        let task_id = task_id.into();
        self.review_status.insert(task_id.clone(), state);
        if let Some(comments) = comments {
            self.review_comments.insert(task_id, comments);
        }
    }
}

fn skip_review_enabled() -> bool {
    std::env::var("TRIUMVIRATE_FLEET_SKIP_REVIEW")
        .ok()
        .map(|v| !matches!(v.to_lowercase().as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::{path::Path, path::PathBuf, sync::Arc};
    use std::fs;

    use async_trait::async_trait;
    use ledger::LedgerStore;
    use tokio::sync::Mutex;

    use super::{GitOps, MergeCoordinator, MergeResult, ReviewGateState};

    #[derive(Debug, Clone)]
    struct MockGitOps {
        merged_branches: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl GitOps for MockGitOps {
        async fn worktree_add(&self, _path: &Path, _branch: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn worktree_remove(&self, _path: &Path) -> anyhow::Result<()> {
            Ok(())
        }

        async fn is_clean(&self) -> anyhow::Result<bool> {
            Ok(true)
        }

        async fn current_head(&self) -> anyhow::Result<String> {
            Ok("head".to_string())
        }

        async fn merge(&self, branch: &str) -> anyhow::Result<MergeResult> {
            self.merged_branches.lock().await.push(branch.to_string());
            if branch.contains("conflict") {
                Ok(MergeResult::Conflict {
                    files: vec!["src/lib.rs".to_string()],
                })
            } else {
                Ok(MergeResult::Success)
            }
        }

        async fn diff(&self, _branch: &str) -> anyhow::Result<String> {
            Ok(String::new())
        }

        async fn rev_parse_toplevel(&self, _cwd: &Path) -> anyhow::Result<PathBuf> {
            Ok(PathBuf::from("/tmp/mock"))
        }
    }

    #[tokio::test]
    async fn merges_follow_completion_order() {
        let merged = Arc::new(Mutex::new(Vec::new()));
        let git_ops = MockGitOps {
            merged_branches: Arc::clone(&merged),
        };
        let mut coordinator = MergeCoordinator::new(git_ops);

        // Agent 2 completes first, then agent 1.
        coordinator.enqueue_completed("task-2", "fleet/task-2");
        coordinator.enqueue_completed("task-1", "fleet/task-1");
        coordinator.set_review_status("task-2", ReviewGateState::Approved, None);
        coordinator.set_review_status("task-1", ReviewGateState::Approved, None);

        let first = coordinator.merge_next().await.expect("merge 1");
        let second = coordinator.merge_next().await.expect("merge 2");
        let none = coordinator.merge_next().await.expect("merge 3");

        assert_eq!(first.as_deref(), Some("task-2"));
        assert_eq!(second.as_deref(), Some("task-1"));
        assert!(none.is_none());
        assert_eq!(coordinator.merged_order(), &["task-2".to_string(), "task-1".to_string()]);

        let merged_branches = merged.lock().await.clone();
        assert_eq!(
            merged_branches,
            vec!["fleet/task-2".to_string(), "fleet/task-1".to_string()]
        );
    }

    #[tokio::test]
    async fn conflict_pauses_queue_and_blocks_subsequent_merges() {
        let merged = Arc::new(Mutex::new(Vec::new()));
        let git_ops = MockGitOps {
            merged_branches: Arc::clone(&merged),
        };
        let mut coordinator = MergeCoordinator::new(git_ops);

        coordinator.enqueue_completed("task-ok", "fleet/task-ok");
        coordinator.enqueue_completed("task-conflict", "fleet/task-conflict");
        coordinator.enqueue_completed("task-after", "fleet/task-after");
        coordinator.set_review_status("task-ok", ReviewGateState::Approved, None);
        coordinator.set_review_status("task-conflict", ReviewGateState::Approved, None);
        coordinator.set_review_status("task-after", ReviewGateState::Approved, None);

        let first = coordinator.merge_next().await.expect("first merge");
        assert_eq!(first.as_deref(), Some("task-ok"));

        let conflict_err = coordinator
            .merge_next()
            .await
            .expect_err("second merge should conflict");
        let conflict_text = conflict_err.to_string();
        assert!(conflict_text.contains("src/lib.rs"));
        assert!(coordinator.is_paused());
        assert_eq!(coordinator.conflicts().len(), 1);
        assert_eq!(coordinator.conflicts()[0].task_id, "task-conflict");

        let paused_err = coordinator
            .merge_next()
            .await
            .expect_err("queue should remain paused");
        assert!(paused_err.to_string().contains("paused"));
    }

    #[tokio::test]
    async fn review_gate_blocks_until_approved_and_pauses_on_request_changes() {
        let merged = Arc::new(Mutex::new(Vec::new()));
        let git_ops = MockGitOps {
            merged_branches: Arc::clone(&merged),
        };
        let mut coordinator = MergeCoordinator::new(git_ops);

        coordinator.enqueue_completed("task-1", "fleet/task-1");
        coordinator.enqueue_completed("task-2", "fleet/task-2");
        coordinator.set_review_status("task-1", ReviewGateState::Pending, None);
        coordinator.set_review_status(
            "task-2",
            ReviewGateState::RequestChanges,
            Some("needs refactor".to_string()),
        );

        let pending_err = coordinator
            .merge_next()
            .await
            .expect_err("pending review should block merge");
        assert!(pending_err.to_string().contains("pending review"));

        coordinator.set_review_status("task-1", ReviewGateState::Approved, None);
        let first = coordinator.merge_next().await.expect("approved merge");
        assert_eq!(first.as_deref(), Some("task-1"));

        let changes_err = coordinator
            .merge_next()
            .await
            .expect_err("request_changes should pause merge");
        assert!(changes_err.to_string().contains("request_changes"));
        assert!(coordinator.is_paused());
    }

    #[tokio::test]
    async fn skip_review_env_bypasses_gate_and_logs_skip_record() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool")).expect("spool");
        let store = LedgerStore::open(project_root.clone()).expect("open ledger");

        let merged = Arc::new(Mutex::new(Vec::new()));
        let git_ops = MockGitOps {
            merged_branches: Arc::clone(&merged),
        };
        let mut coordinator = MergeCoordinator::new(git_ops).with_project_root(project_root.clone());
        coordinator.enqueue_completed("task-1", "fleet/task-1");
        coordinator.set_review_status("task-1", ReviewGateState::Pending, None);

        // SAFETY: test controls env var lifecycle.
        unsafe {
            std::env::set_var("TRIUMVIRATE_FLEET_SKIP_REVIEW", "1");
        }
        let merged_task = coordinator.merge_next().await.expect("skip review merge");
        assert_eq!(merged_task.as_deref(), Some("task-1"));
        // SAFETY: test controls env var lifecycle.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_FLEET_SKIP_REVIEW");
        }

        let skipped = store.query("review skipped", 10).expect("query skipped");
        assert!(skipped
            .iter()
            .any(|summary| summary.summary_type == "review_skipped"));
    }
}
