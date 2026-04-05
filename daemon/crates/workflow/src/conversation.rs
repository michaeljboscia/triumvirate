use crate::engine::WorkflowEngine;
use crate::state::WorkflowType;

pub struct ConversationWorkflow<'a> {
    engine: &'a WorkflowEngine,
    workflow_id: String,
}

impl<'a> ConversationWorkflow<'a> {
    pub fn start(engine: &'a WorkflowEngine) -> anyhow::Result<Self> {
        let workflow_id = engine.start_workflow(WorkflowType::Conversation)?;
        Ok(Self { engine, workflow_id })
    }

    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }

    pub fn route_human_message(&self, route_target: &str, content: &str) -> anyhow::Result<()> {
        let payload = serde_json::json!({
            "route_target": route_target,
            "content": content,
        });
        self.engine
            .advance_step(&self.workflow_id, 1, &payload.to_string())
    }

    pub fn mark_completed(&self) -> anyhow::Result<()> {
        self.engine.complete(&self.workflow_id, 2)
    }
}
