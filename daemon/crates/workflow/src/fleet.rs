use crate::engine::WorkflowEngine;
use crate::state::WorkflowType;

pub struct FleetWorkflow<'a> {
    engine: &'a WorkflowEngine,
    workflow_id: String,
}

impl<'a> FleetWorkflow<'a> {
    pub fn start(engine: &'a WorkflowEngine, fleet_id: &str, spec: &str) -> anyhow::Result<Self> {
        let workflow_id = engine.start_workflow(WorkflowType::Fleet)?;
        let payload = serde_json::json!({
            "phase": "spawned",
            "fleet_id": fleet_id,
            "spec": spec,
        });
        engine.advance_step(&workflow_id, 0, &payload.to_string())?;
        Ok(Self { engine, workflow_id })
    }

    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }

    pub fn mark_contracts_ready(&self, contracts_summary: &str) -> anyhow::Result<()> {
        let payload = serde_json::json!({
            "phase": "contracts_ready",
            "summary": contracts_summary,
        });
        self.engine
            .advance_step(&self.workflow_id, 1, &payload.to_string())
    }

    pub fn mark_parallel_complete(&self, merged_files: usize) -> anyhow::Result<()> {
        let payload = serde_json::json!({
            "phase": "parallel_complete",
            "merged_files": merged_files,
        });
        self.engine
            .advance_step(&self.workflow_id, 2, &payload.to_string())
    }

    pub fn mark_completed(&self) -> anyhow::Result<()> {
        self.engine.complete(&self.workflow_id, 3)
    }
}

