//! Fleet orchestration crate.

pub mod worktree;

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
