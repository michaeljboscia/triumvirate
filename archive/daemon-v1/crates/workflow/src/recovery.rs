use std::path::Path;

use crate::{WorkflowStore, WorkflowSummary};

#[derive(Debug, Clone)]
pub struct RecoveryReport {
    pub resumable: Vec<WorkflowSummary>,
}

impl RecoveryReport {
    pub fn resumable_count(&self) -> usize {
        self.resumable.len()
    }
}

/// Inspect workflow state for any incomplete executions that can be resumed.
pub fn inspect_recovery(path: &Path) -> anyhow::Result<RecoveryReport> {
    let store = WorkflowStore::open(path)?;
    let resumable = store.resumable_workflows()?;
    Ok(RecoveryReport { resumable })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use uuid::Uuid;

    use super::inspect_recovery;
    use crate::{WorkflowState, WorkflowStore, WorkflowType};

    fn temp_db() -> PathBuf {
        std::env::temp_dir().join(format!("workflow-recovery-{}.db", Uuid::new_v4()))
    }

    #[test]
    fn recovery_finds_resumable_workflows() {
        let db = temp_db();
        let store = WorkflowStore::open(&db).expect("open");
        let id = Uuid::new_v4().to_string();
        store
            .create_workflow(&id, WorkflowType::Conversation)
            .expect("create");
        store
            .set_state(&id, WorkflowState::Running, 1)
            .expect("running");

        let report = inspect_recovery(&db).expect("report");
        assert_eq!(report.resumable_count(), 1);
    }
}
