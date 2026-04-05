use crate::engine::WorkflowEngine;
use crate::state::WorkflowType;

pub struct DebateWorkflow<'a> {
    engine: &'a WorkflowEngine,
    workflow_id: String,
}

impl<'a> DebateWorkflow<'a> {
    pub fn start(
        engine: &'a WorkflowEngine,
        topic: &str,
        participants: &[&str],
    ) -> anyhow::Result<Self> {
        let workflow_id = engine.start_workflow(WorkflowType::Debate)?;
        let payload = serde_json::json!({
            "phase": "proposal",
            "topic": topic,
            "participants": participants,
        });
        engine.advance_step(&workflow_id, 0, &payload.to_string())?;
        Ok(Self { engine, workflow_id })
    }

    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }

    pub fn record_challenge(&self, challenger: &str, argument: &str) -> anyhow::Result<()> {
        let payload = serde_json::json!({
            "phase": "challenge",
            "challenger": challenger,
            "argument": argument,
        });
        self.engine
            .advance_step(&self.workflow_id, 1, &payload.to_string())
    }

    pub fn record_vote(&self, voter: &str, vote: &str) -> anyhow::Result<()> {
        let payload = serde_json::json!({
            "phase": "vote",
            "voter": voter,
            "vote": vote,
        });
        self.engine
            .advance_step(&self.workflow_id, 2, &payload.to_string())
    }

    pub fn complete(&self, decision: &str) -> anyhow::Result<()> {
        let payload = serde_json::json!({
            "phase": "decision",
            "decision": decision,
        });
        self.engine
            .advance_step(&self.workflow_id, 3, &payload.to_string())?;
        self.engine.complete(&self.workflow_id, 4)
    }
}

