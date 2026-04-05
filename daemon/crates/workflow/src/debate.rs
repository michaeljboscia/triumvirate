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

    pub fn attach(engine: &'a WorkflowEngine, workflow_id: impl Into<String>) -> Self {
        Self {
            engine,
            workflow_id: workflow_id.into(),
        }
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use uuid::Uuid;

    use super::DebateWorkflow;
    use crate::WorkflowEngine;

    fn temp_db() -> PathBuf {
        std::env::temp_dir().join(format!("workflow-debate-{}.db", Uuid::new_v4()))
    }

    #[test]
    fn debate_workflow_records_full_lifecycle() {
        let db = temp_db();
        let engine = WorkflowEngine::open(&db).expect("open");
        let debate = DebateWorkflow::start(&engine, "Redis vs Postgres", &["claude", "gemini"])
            .expect("start");

        debate
            .record_challenge("gemini", "postgreSQL is simpler operationally")
            .expect("challenge");
        debate.record_vote("claude", "postgres").expect("vote");
        debate.complete("postgres").expect("complete");

        let events = engine.store().events(debate.workflow_id()).expect("events");
        assert!(events.len() >= 5);
    }
}
