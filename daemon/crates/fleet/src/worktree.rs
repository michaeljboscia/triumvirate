use std::path::{Path, PathBuf};

use shared_types::GitOps;

/// Where a fleet member's cargo builds go: `CARGO_TARGET_DIR` for its agent, on both engines.
/// A path WE chose, inside the worktree (so a sandboxed agent may write it) and under the
/// ignored `.triumvirate/`, so the member's exit can delete it without guessing which `target/`
/// directories in someone else's repo are build output. D-048: each member used to build into
/// its own `daemon/target` (1 to 5 GB) and nothing removed it; ten fleets filled the disk.
pub fn member_target_dir(worktree: &Path) -> PathBuf {
    worktree.join(".triumvirate").join("target")
}

/// Delete a finished member's build output. The worktree and its branch stay (they are the
/// deliverable). Nothing after the agent exits builds in the worktree, so nothing needs it.
/// Never fails the member: a leftover is logged, and the next exit tries again.
pub fn remove_member_target(worktree: &Path) {
    let dir = member_target_dir(worktree);
    // The agent owns the worktree's contents. A `.triumvirate` or `target` it replaced with a
    // symlink would make the delete follow the link out of the worktree (D-048 panel, Grok),
    // so the two components this fleet added must be real directories, or nothing is removed.
    for component in [worktree.join(".triumvirate"), dir.clone()] {
        match std::fs::symlink_metadata(&component) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => {
                tracing::warn!(path = %component.display(), "fleet member build dir is not a plain directory; not removed (D-048)");
                return;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::warn!(path = %component.display(), error = %e, "fleet member build dir could not be checked; not removed (D-048)");
                return;
            }
        }
    }
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(dir = %dir.display(), error = %e, "fleet member build output was not removed (D-048)"),
    }
}

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

#[cfg(test)]
mod member_target_tests {
    use super::{member_target_dir, remove_member_target};

    fn build_output_in(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("debug")).expect("mkdir");
        std::fs::write(dir.join("debug/big"), b"x").expect("write");
    }

    /// RED IF the member's own build output survives the cleanup.
    #[test]
    fn a_plain_build_dir_is_removed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ours = member_target_dir(tmp.path());
        build_output_in(&ours);
        remove_member_target(tmp.path());
        assert!(!ours.exists());
        assert!(tmp.path().join(".triumvirate").exists(), "only target/ goes, not .triumvirate/");
    }

    /// The agent swapped `.triumvirate` for a link to somewhere else that holds a `target/`.
    /// RED IF the delete follows the link and empties a directory outside the worktree.
    #[test]
    fn a_symlinked_triumvirate_dir_is_not_followed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (wt, outside) = (tmp.path().join("wt"), tmp.path().join("outside"));
        std::fs::create_dir_all(&wt).expect("wt");
        build_output_in(&outside.join("target"));
        std::os::unix::fs::symlink(&outside, wt.join(".triumvirate")).expect("symlink");
        remove_member_target(&wt);
        assert!(outside.join("target/debug/big").exists(), "a file outside the worktree was deleted");
    }

    /// Same, one level down: `target` itself is the link. std's `remove_dir_all` already unlinks
    /// a final-component link without following it (negative control: this stays green with the
    /// guard removed), so this pins that behavior rather than proving the guard.
    #[test]
    fn a_symlinked_target_dir_is_not_followed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (wt, outside) = (tmp.path().join("wt"), tmp.path().join("outside"));
        std::fs::create_dir_all(wt.join(".triumvirate")).expect("wt");
        build_output_in(&outside);
        std::os::unix::fs::symlink(&outside, member_target_dir(&wt)).expect("symlink");
        remove_member_target(&wt);
        assert!(outside.join("debug/big").exists(), "a file outside the worktree was deleted");
    }
}
