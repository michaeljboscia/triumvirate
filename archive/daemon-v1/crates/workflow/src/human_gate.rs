use uuid::Uuid;

/// Represents a paused workflow awaiting explicit human approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanGateTicket {
    pub workflow_id: String,
    pub step: i64,
    pub prompt: String,
    pub ticket_id: String,
}

impl HumanGateTicket {
    pub fn new(workflow_id: impl Into<String>, step: i64, prompt: impl Into<String>) -> Self {
        Self {
            workflow_id: workflow_id.into(),
            step,
            prompt: prompt.into(),
            ticket_id: Uuid::new_v4().to_string(),
        }
    }
}
