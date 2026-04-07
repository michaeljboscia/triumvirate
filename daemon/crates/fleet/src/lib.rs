//! Fleet orchestration crate.
//!
//! This crate owns multi-agent task planning and execution primitives.

use shared_types::GitOps;

#[derive(Debug, Clone)]
pub struct FleetEngine<G: GitOps> {
    git_ops: G,
}

impl<G: GitOps> FleetEngine<G> {
    pub fn new(git_ops: G) -> Self {
        Self { git_ops }
    }

    pub fn git_ops(&self) -> &G {
        &self.git_ops
    }
}

#[cfg(test)]
mod tests {
    use super::FleetEngine;

    #[derive(Debug, Clone)]
    struct NoopGitOps;

    impl shared_types::GitOps for NoopGitOps {
        fn worktree_add(
            &self,
            _repo_path: &std::path::Path,
            _worktree_path: &std::path::Path,
            _branch: &str,
            _start_point: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn worktree_remove(
            &self,
            _repo_path: &std::path::Path,
            _worktree_path: &std::path::Path,
            _force: bool,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn is_clean(&self, _repo_path: &std::path::Path) -> anyhow::Result<bool> {
            Ok(true)
        }

        fn merge(
            &self,
            _repo_path: &std::path::Path,
            _source_ref: &str,
            _target_ref: &str,
            _no_ff: bool,
        ) -> anyhow::Result<shared_types::MergeResult> {
            Ok(shared_types::MergeResult {
                ok: true,
                fast_forward: false,
                commit_sha: Some("deadbeef".to_string()),
                conflicts: Vec::new(),
                message: None,
            })
        }
    }

    #[test]
    fn engine_holds_git_ops() {
        let engine = FleetEngine::new(NoopGitOps);
        assert!(engine.git_ops().is_clean(std::path::Path::new(".")).is_ok());
    }
}
