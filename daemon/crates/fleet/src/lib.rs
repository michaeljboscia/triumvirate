//! Fleet orchestration crate.

pub mod worktree;
pub mod tasks;
pub mod orchestrator;
pub mod merge;
pub mod recovery;

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
