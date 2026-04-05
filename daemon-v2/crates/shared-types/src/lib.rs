//! Shared DTOs for MCP bridge <-> daemon communication.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LifecycleEvent {
    pub state: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskAgentRequest {
    pub agent: String,
    pub message: String,
    pub cwd: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskAgentResponse {
    pub request_id: String,
    pub agent: String,
    pub response: String,
    pub lifecycle: Vec<LifecycleEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskTwinsRequest {
    pub message: String,
    pub cwd: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentResult {
    pub agent: String,
    pub response: String,
    pub prompt_sent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskTwinsResponse {
    pub request_id: String,
    pub results: Vec<AgentResult>,
    pub failures: Vec<LifecycleEvent>,
    pub lifecycle: Vec<LifecycleEvent>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn lifecycle_event_holds_values() {
        let s = super::LifecycleEvent {
            state: "DONE".to_string(),
            detail: "ok".to_string(),
        };
        assert_eq!(s.state, "DONE");
        assert_eq!(s.detail, "ok");
    }

    #[test]
    fn ask_agent_request_roundtrips_json() {
        let req = super::AskAgentRequest {
            agent: "gemini".to_string(),
            message: "hello".to_string(),
            cwd: Some("/tmp".to_string()),
            repo: None,
            branch: None,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let decoded: super::AskAgentRequest =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.agent, "gemini");
        assert_eq!(decoded.message, "hello");
    }
}
