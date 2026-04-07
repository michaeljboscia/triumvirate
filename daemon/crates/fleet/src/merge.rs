use std::collections::VecDeque;

use shared_types::{GitOps, MergeResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedWork {
    pub task_id: String,
    pub branch: String,
}

#[derive(Debug, Clone)]
pub struct MergeCoordinator<G: GitOps> {
    git_ops: G,
    completion_queue: VecDeque<CompletedWork>,
    merged_order: Vec<String>,
}

impl<G: GitOps> MergeCoordinator<G> {
    pub fn new(git_ops: G) -> Self {
        Self {
            git_ops,
            completion_queue: VecDeque::new(),
            merged_order: Vec::new(),
        }
    }

    pub fn enqueue_completed(&mut self, task_id: impl Into<String>, branch: impl Into<String>) {
        self.completion_queue.push_back(CompletedWork {
            task_id: task_id.into(),
            branch: branch.into(),
        });
    }

    pub async fn merge_next(&mut self) -> anyhow::Result<Option<String>> {
        let Some(next) = self.completion_queue.pop_front() else {
            return Ok(None);
        };

        match self.git_ops.merge(&next.branch).await? {
            MergeResult::Success => {
                self.merged_order.push(next.task_id.clone());
                Ok(Some(next.task_id))
            }
            MergeResult::Conflict { files } => {
                anyhow::bail!(
                    "merge conflict while merging task {} ({} files)",
                    next.task_id,
                    files.len()
                );
            }
        }
    }

    pub fn merged_order(&self) -> &[String] {
        &self.merged_order
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, path::PathBuf, sync::Arc};

    use async_trait::async_trait;
    use tokio::sync::Mutex;

    use super::{GitOps, MergeCoordinator, MergeResult};

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
            Ok(MergeResult::Success)
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
}
