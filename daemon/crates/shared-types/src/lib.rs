//! Shared DTOs for MCP bridge <-> daemon communication.

mod git_ops;
mod ledger;
mod abe;

pub use abe::*;
pub use git_ops::{GitOps, MergeResult};
pub use ledger::{
    DrainResult, GcResult, HealthStatus, Lesson, ManualRecord, NewLesson, RawEvent, SessionDetail,
    Summary,
};

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
pub struct SpawnSessionRequest {
    pub agent: String,
    pub name: String,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionInfo {
    pub name: String,
    pub agent: String,
    pub turns: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskSessionRequest {
    pub name: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DismissSessionRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryWriteRequest {
    pub namespace: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryWriteResponse {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryReadRequest {
    pub namespace: String,
    pub key: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryEntry {
    pub id: String,
    pub namespace: String,
    pub key: String,
    pub value: String,
    pub ts_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryReadResponse {
    pub entries: Vec<MemoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScratchpadWriteRequest {
    pub project: String,
    pub topic: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScratchpadWriteResponse {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScratchpadListRequest {
    pub project: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScratchpadListResponse {
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FallbackListRequest {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FallbackListResponse {
    pub tickets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FallbackAckRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FallbackGcRequest {
    pub max_age_days: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FallbackGcResponse {
    pub removed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutboxEvent {
    pub ts_ms: u128,
    pub request_id: String,
    pub tool: String,
    pub status: String,
    pub agent: Option<String>,
    pub detail: String,
    pub cwd: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutboxRecentRequest {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutboxRecentResponse {
    pub events: Vec<OutboxEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LedgerQueryRequest {
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LedgerQueryResponse {
    pub summaries: Vec<Summary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LedgerSessionRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LessonQueryRequest {
    pub query: String,
    pub min_confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LessonAddResponse {
    pub lesson_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LessonQueryResponse {
    pub lessons: Vec<Lesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LessonValidateRequest {
    pub lesson_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LessonListRequest {
    pub tags: Option<Vec<String>>,
    pub stale_days: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LessonListResponse {
    pub lessons: Vec<Lesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FleetSpawnRequest {
    pub project_root: Option<String>,
    pub agents: Option<Vec<String>>,
    pub dry_run: Option<bool>,
    pub wait: Option<bool>,
    pub task_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FleetSpawnResponse {
    pub fleet_id: String,
    pub plan: String,
    pub head_sha: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FleetStatusRequest {
    pub fleet_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FleetStatusResponse {
    pub fleet_id: String,
    pub state: String,
    pub worktree_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FleetTaskListRequest {
    pub fleet_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FleetTaskListResponse {
    pub task_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FleetClaimTaskRequest {
    pub project_root: Option<String>,
    pub task_id: String,
    pub assigned_agent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FleetClaimTaskResponse {
    pub claimed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FleetCancelRequest {
    pub fleet_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FleetCancelResponse {
    pub canceled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviewRequestTool {
    pub project_root: Option<String>,
    pub fleet_id: Option<String>,
    pub author_agent: String,
    pub artifact: String,
    pub review_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviewRequestResponse {
    pub review_id: String,
    pub reviewer_agent: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviewSubmitRequest {
    pub project_root: Option<String>,
    pub review_id: String,
    pub verdict: String,
    pub comments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviewStatusRequest {
    pub project_root: Option<String>,
    pub review_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviewStatusResponse {
    pub review_id: String,
    pub reviewer_agent: Option<String>,
    pub verdict: Option<String>,
    pub comments: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DaemonHealthResponse {
    pub status: String,
    pub service: Option<String>,
    pub mode: Option<String>,
    pub daemon: Option<String>,
    pub auth: Option<String>,
    pub daemon_bind_addr: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DaemonStatusSnapshot {
    pub daemon_mode: Option<String>,
    pub supported_agents: Option<Vec<String>>,
    pub pending_fallbacks: Option<usize>,
    pub fallback_tickets: Option<Vec<String>>,
    pub daemon_bind_addr: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StatusResponse {
    pub daemon_mode: String,
    pub active_sessions: usize,
    pub supported_agents: Vec<String>,
    pub pending_fallbacks: usize,
    pub fallback_tickets: Vec<String>,
    pub daemon_bind_addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionState {
    pub agent: String,
    #[serde(default)]
    pub cwd: Option<String>,
    pub history: Vec<String>,
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

    #[test]
    fn status_response_roundtrips_json_with_bind_addr() {
        let status = super::StatusResponse {
            daemon_mode: "incremental-dev".to_string(),
            active_sessions: 2,
            supported_agents: vec!["gemini".to_string(), "codex".to_string()],
            pending_fallbacks: 1,
            fallback_tickets: vec!["ticket-1.md".to_string()],
            daemon_bind_addr: "127.0.0.1:8080".to_string(),
        };

        let json = serde_json::to_string(&status).expect("serialize");
        let decoded: super::StatusResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.active_sessions, 2);
        assert_eq!(decoded.daemon_bind_addr, "127.0.0.1:8080");
    }

    #[test]
    fn ledger_query_request_roundtrips_json() {
        let req = super::LedgerQueryRequest {
            query: "wal".to_string(),
            limit: Some(5),
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let decoded: super::LedgerQueryRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.query, "wal");
        assert_eq!(decoded.limit, Some(5));
    }
}
