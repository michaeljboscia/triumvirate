use std::path::{Path, PathBuf};

use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeResult {
    Success,
    Conflict { files: Vec<String> },
}

#[async_trait]
pub trait GitOps: Send + Sync {
    async fn worktree_add(&self, path: &Path, branch: &str) -> anyhow::Result<()>;
    async fn worktree_remove(&self, path: &Path) -> anyhow::Result<()>;
    async fn is_clean(&self) -> anyhow::Result<bool>;
    async fn current_head(&self) -> anyhow::Result<String>;
    async fn merge(&self, branch: &str) -> anyhow::Result<MergeResult>;
    async fn diff(&self, branch: &str) -> anyhow::Result<String>;
    async fn rev_parse_toplevel(&self, cwd: &Path) -> anyhow::Result<PathBuf>;
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use async_trait::async_trait;

    use super::{GitOps, MergeResult};

    struct MockGitOps;

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
            Ok("abc123".to_string())
        }

        async fn merge(&self, branch: &str) -> anyhow::Result<MergeResult> {
            if branch == "main" {
                Ok(MergeResult::Success)
            } else {
                Ok(MergeResult::Conflict {
                    files: vec!["src/lib.rs".to_string()],
                })
            }
        }

        async fn diff(&self, branch: &str) -> anyhow::Result<String> {
            Ok(format!("diff -- {branch}"))
        }

        async fn rev_parse_toplevel(&self, _cwd: &Path) -> anyhow::Result<PathBuf> {
            Ok(PathBuf::from("/tmp/mock-repo"))
        }
    }

    #[tokio::test]
    async fn gitops_trait_mock_compiles_and_methods_execute() {
        let mock = MockGitOps;
        mock.worktree_add(Path::new("/tmp/repo"), "main")
            .await
            .expect("worktree_add");
        mock.worktree_remove(Path::new("/tmp/repo"))
            .await
            .expect("worktree_remove");
        assert!(mock.is_clean().await.expect("is_clean"));
        assert_eq!(mock.current_head().await.expect("current_head"), "abc123");
        assert_eq!(
            mock.merge("main").await.expect("merge"),
            MergeResult::Success
        );
        assert!(matches!(
            mock.merge("feature").await.expect("merge"),
            MergeResult::Conflict { .. }
        ));
        assert_eq!(mock.diff("main").await.expect("diff"), "diff -- main");
        assert_eq!(
            mock.rev_parse_toplevel(Path::new("/tmp/repo"))
                .await
                .expect("rev_parse_toplevel"),
            PathBuf::from("/tmp/mock-repo")
        );
    }
}
