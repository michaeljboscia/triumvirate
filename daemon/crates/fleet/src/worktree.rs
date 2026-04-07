use std::path::Path;

use shared_types::GitOps;

#[derive(Debug, Clone)]
pub struct WorktreeManager<G: GitOps> {
    git_ops: G,
}

impl<G: GitOps> WorktreeManager<G> {
    pub fn new(git_ops: G) -> Self {
        Self { git_ops }
    }

    pub async fn create_worktree(&self, path: &Path, branch: &str) -> anyhow::Result<()> {
        if !self.git_ops.is_clean().await? {
            anyhow::bail!(
                "cannot create worktree with uncommitted or dirty changes; commit or stash first"
            );
        }
        self.git_ops.worktree_add(path, branch).await
    }

    pub async fn remove_worktree(&self, path: &Path) -> anyhow::Result<()> {
        self.git_ops.worktree_remove(path).await
    }
}

#[cfg(test)]
mod tests {
    use std::{path::{Path, PathBuf}, sync::{Arc, Mutex}};

    use async_trait::async_trait;
    use shared_types::{GitOps, MergeResult};

    use super::WorktreeManager;

    #[derive(Debug, Clone)]
    struct MockGitOps {
        clean: bool,
        created: Arc<Mutex<Vec<PathBuf>>>,
        removed: Arc<Mutex<Vec<PathBuf>>>,
    }

    #[async_trait]
    impl GitOps for MockGitOps {
        async fn worktree_add(&self, path: &Path, _branch: &str) -> anyhow::Result<()> {
            self.created
                .lock()
                .expect("created lock")
                .push(path.to_path_buf());
            Ok(())
        }

        async fn worktree_remove(&self, path: &Path) -> anyhow::Result<()> {
            self.removed
                .lock()
                .expect("removed lock")
                .push(path.to_path_buf());
            Ok(())
        }

        async fn is_clean(&self) -> anyhow::Result<bool> {
            Ok(self.clean)
        }

        async fn current_head(&self) -> anyhow::Result<String> {
            Ok("head".to_string())
        }

        async fn merge(&self, _branch: &str) -> anyhow::Result<MergeResult> {
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
    async fn clean_repo_creates_worktree() {
        let created = Arc::new(Mutex::new(Vec::new()));
        let removed = Arc::new(Mutex::new(Vec::new()));
        let manager = WorktreeManager::new(MockGitOps {
            clean: true,
            created: Arc::clone(&created),
            removed,
        });

        let path = PathBuf::from("/tmp/worktree-clean");
        manager
            .create_worktree(&path, "feature/clean")
            .await
            .expect("create worktree");

        let created_paths = created.lock().expect("created lock");
        assert_eq!(created_paths.len(), 1);
        assert_eq!(created_paths[0], path);
    }

    #[tokio::test]
    async fn dirty_repo_rejects_create_with_actionable_error() {
        let manager = WorktreeManager::new(MockGitOps {
            clean: false,
            created: Arc::new(Mutex::new(Vec::new())),
            removed: Arc::new(Mutex::new(Vec::new())),
        });

        let err = manager
            .create_worktree(Path::new("/tmp/worktree-dirty"), "feature/dirty")
            .await
            .expect_err("dirty repository must be rejected");
        let message = err.to_string().to_lowercase();
        assert!(message.contains("uncommitted") || message.contains("dirty"));
    }

    #[tokio::test]
    async fn remove_worktree_calls_gitops() {
        let removed = Arc::new(Mutex::new(Vec::new()));
        let manager = WorktreeManager::new(MockGitOps {
            clean: true,
            created: Arc::new(Mutex::new(Vec::new())),
            removed: Arc::clone(&removed),
        });

        let path = PathBuf::from("/tmp/worktree-remove");
        manager
            .remove_worktree(&path)
            .await
            .expect("remove worktree");

        let removed_paths = removed.lock().expect("removed lock");
        assert_eq!(removed_paths.len(), 1);
        assert_eq!(removed_paths[0], path);
    }
}
