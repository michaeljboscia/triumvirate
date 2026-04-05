use std::path::Path;

use anyhow::Context;
use uuid::Uuid;

use crate::persistence::WorkflowStore;
use crate::state::{WorkflowState, WorkflowType};

pub struct WorkflowEngine {
    store: WorkflowStore,
}

impl WorkflowEngine {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let store = WorkflowStore::open(path).context("open workflow store")?;
        Ok(Self { store })
    }

    pub fn start_workflow(&self, workflow_type: WorkflowType) -> anyhow::Result<String> {
        let id = Uuid::new_v4().to_string();
        self.store.create_workflow(&id, workflow_type)?;
        self.store.set_state(&id, WorkflowState::Running, 0)?;
        self.store.append_event(&id, 0, "started", "{}")?;
        Ok(id)
    }

    pub fn advance_step(&self, workflow_id: &str, step: i64, payload_json: &str) -> anyhow::Result<()> {
        self.store
            .append_event(workflow_id, step, "step_completed", payload_json)?;
        self.store
            .set_state(workflow_id, WorkflowState::Running, step + 1)?;
        Ok(())
    }

    pub fn pause(&self, workflow_id: &str, step: i64, reason: &str) -> anyhow::Result<()> {
        self.store
            .append_event(workflow_id, step, "paused", &format!(r#"{{"reason":"{}"}}"#, reason))?;
        self.store.set_state(workflow_id, WorkflowState::Paused, step)?;
        Ok(())
    }

    pub fn complete(&self, workflow_id: &str, step: i64) -> anyhow::Result<()> {
        self.store.append_event(workflow_id, step, "completed", "{}")?;
        self.store
            .set_state(workflow_id, WorkflowState::Completed, step)?;
        Ok(())
    }

    pub fn store(&self) -> &WorkflowStore {
        &self.store
    }
}
