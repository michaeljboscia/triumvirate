//! Shared DTOs for MCP bridge <-> daemon communication.

mod api;
mod git_ops;
mod ledger;
mod abe;
mod streaming;

pub use abe::*;
pub use api::{
    WorkersResponse, WorkerInfo, FleetResponse, FleetBuild, FleetTask,
    StateResponse, ReplayRequest, ReplayResponse,
};
pub use git_ops::{GitOps, MergeResult};
pub use streaming::{AgentStreamEvent, WorkerLifecycleType};
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct AskAgentRequest {
    pub agent: String,
    pub message: String,
    pub cwd: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,

    // T-011 (REQ-DS-027): per-call overrides for the DeepSeek sibling. ALL
    // four fields are Optional and skip-serialize-on-None so the wire shape
    // is unchanged for Gemini/Codex callers. The runner (T-012) reads them
    // when present, otherwise falls back to DeepSeekConfig defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_thinking: Option<DeepSeekThinking>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_reasoning_effort: Option<DeepSeekEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_include_reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_max_tokens: Option<u32>,
    /// 2026-05-26 follow-up to T-011: per-call model override. When set, this
    /// value replaces `cfg.model` for this consult only — lets callers pick
    /// between `"deepseek-v4-flash"` (default; matches Pro on quality across
    /// the v1–v4 eval at ~3.4× lower cost — see PRO_VS_FLASH_EVAL_RESULTS.md)
    /// and `"deepseek-v4-pro"` (deeper reasoning available per-call) without
    /// restarting the daemon. Unrecognised models get surfaced as
    /// HardProvider(400) from DeepSeek; no client-side validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_model: Option<String>,
}

/// T-011: per-call thinking override. Lower-case wire form mirrors the
/// DeepSeek API string.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeepSeekThinking {
    Enabled,
    Disabled,
}

/// T-011: per-call reasoning-effort override. We accept all four caller
/// strings but the runner (T-012) collapses `Xhigh` → `Max` to match the
/// DeepSeek API wire surface (the API itself does the same mapping).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeepSeekEffort {
    Low,
    Medium,
    High,
    Max,
    Xhigh,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskAgentResponse {
    pub request_id: String,
    /// The agent that was REQUESTED. Stays stable (e.g. `gemini`) even when a
    /// degraded hop answers with a different agent — clients keyed on this field
    /// are unaffected by backend substitution (REQ-053).
    pub agent: String,
    pub response: String,
    pub lifecycle: Vec<LifecycleEvent>,
    /// Substitution honesty (REQ-053 R3): set only when a degraded route answered
    /// with a different agent/backend than requested. Omitted from the wire on the
    /// normal path so existing clients see an unchanged shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_by_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_by_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded_from_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degradation_reason: Option<String>,
    /// Shadow-compare mode (opt-in, TRIUMVIRATE_GEMINI_SHADOW): the OTHER Gemini
    /// backend ran alongside the primary for comparison. `.response` is still the
    /// primary's answer; these carry the shadow's answer/latency/error so the caller
    /// can compare the two backends on real traffic. Omitted from the wire when off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_response: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_latency_ms: Option<u64>,
}

impl AskAgentResponse {
    /// A normal, non-degraded response: the requested agent answered directly.
    pub fn direct(
        request_id: String,
        agent: String,
        response: String,
        lifecycle: Vec<LifecycleEvent>,
    ) -> Self {
        Self {
            request_id,
            agent,
            response,
            lifecycle,
            answered_by_agent: None,
            answered_by_backend: None,
            degraded_from_backend: None,
            degradation_reason: None,
            shadow_backend: None,
            shadow_response: None,
            shadow_error: None,
            shadow_latency_ms: None,
        }
    }

    /// Attach shadow-compare results to a response (Slice 6 / shadow mode).
    pub fn with_shadow(
        mut self,
        backend: Option<String>,
        response: Option<String>,
        error: Option<String>,
        latency_ms: Option<u64>,
    ) -> Self {
        self.shadow_backend = backend;
        self.shadow_response = response;
        self.shadow_error = error;
        self.shadow_latency_ms = latency_ms;
        self
    }
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
    pub version: Option<String>,
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
    /// The immediate session that triggered this session's dispatch.
    /// For Pantheon-spawned Claude Code sessions, this is the panel_id.
    /// For Codex/Gemini workers dispatched via MCP, this is the caller's session_id.
    /// FEAT-011 (REQ-010)
    #[serde(default)]
    pub parent_session_id: Option<String>,
    /// The top-level user session in a dispatch chain. For Pantheon terminal
    /// panels, root == parent == own session_id. For workers spawned by
    /// sub-agents, this chains back to the original Pantheon panel.
    /// FEAT-011 (REQ-010)
    #[serde(default)]
    pub root_session_id: Option<String>,
    /// The PANTHEON_SESSION_ID env var value captured from the MCP handshake
    /// (via _meta.pantheon.session_id for stdio or X-Pantheon-Session-Id
    /// header for HTTP/SSE). NULL for non-Pantheon sessions.
    /// FEAT-011 (REQ-033)
    #[serde(default)]
    pub pantheon_session_id: Option<String>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_state_persists_pantheon_lineage_fields() {
        // Reality test: verify lineage fields round-trip through JSON
        // (SessionState is persisted as JSON at ~/.triumvirate/sessions.json,
        // not SQLite — the JSON file IS the database).
        // A stub struct missing these fields would fail deserialization
        // because we assert exact values after round-trip.
        use super::SessionState;

        let state = SessionState {
            agent: "claude".to_string(),
            cwd: Some("/Users/you/projects/triumvirate".to_string()),
            history: vec!["initialize".to_string()],
            parent_session_id: Some("sess-pantheon-panel-1".to_string()),
            root_session_id: Some("sess-pantheon-panel-1".to_string()),
            pantheon_session_id: Some("pantheon-uuid-abc-123".to_string()),
        };

        let json = serde_json::to_string(&state).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["parent_session_id"], "sess-pantheon-panel-1");
        assert_eq!(value["root_session_id"], "sess-pantheon-panel-1");
        assert_eq!(value["pantheon_session_id"], "pantheon-uuid-abc-123");

        let parsed: SessionState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.parent_session_id, Some("sess-pantheon-panel-1".to_string()));
        assert_eq!(parsed.root_session_id, Some("sess-pantheon-panel-1".to_string()));
        assert_eq!(parsed.pantheon_session_id, Some("pantheon-uuid-abc-123".to_string()));
    }

    #[test]
    fn session_state_backwards_compatible_missing_lineage() {
        // Verify existing sessions.json files (without lineage fields) still
        // deserialize — #[serde(default)] means missing fields become None.
        use super::SessionState;

        let legacy_json = r#"{"agent":"gemini","cwd":"/tmp","history":["start"]}"#;
        let parsed: SessionState = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(parsed.agent, "gemini");
        assert_eq!(parsed.parent_session_id, None);
        assert_eq!(parsed.root_session_id, None);
        assert_eq!(parsed.pantheon_session_id, None);
    }

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
            ..Default::default()
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

    #[test]
    fn token_usage_has_thinking_tokens_field() {
        let usage = super::TokenUsage {
            input: None,
            output: None,
            cached: None,
            thinking_tokens: Some(42),
            latency_ms: None,
            tool_calls: None,
            total: None,
        };
        assert_eq!(usage.thinking_tokens, Some(42));
    }

    #[test]
    fn token_usage_has_latency_ms_field() {
        let usage = super::TokenUsage {
            input: None,
            output: None,
            cached: None,
            thinking_tokens: None,
            latency_ms: Some(1200),
            tool_calls: None,
            total: None,
        };
        assert_eq!(usage.latency_ms, Some(1200));
    }

    #[test]
    fn token_usage_has_tool_calls_field() {
        let usage = super::TokenUsage {
            input: None,
            output: None,
            cached: None,
            thinking_tokens: None,
            latency_ms: None,
            tool_calls: Some(3),
            total: None,
        };
        assert_eq!(usage.tool_calls, Some(3));
    }

    #[test]
    fn token_usage_roundtrips_with_optional_fields() {
        let usage = super::TokenUsage {
            input: Some(100),
            output: Some(40),
            cached: Some(5),
            thinking_tokens: Some(8),
            latency_ms: Some(900),
            tool_calls: Some(2),
            total: Some(145),
        };
        let json = serde_json::to_string(&usage).expect("serialize");
        let decoded: super::TokenUsage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.input, Some(100));
        assert_eq!(decoded.output, Some(40));
        assert_eq!(decoded.cached, Some(5));
        assert_eq!(decoded.thinking_tokens, Some(8));
        assert_eq!(decoded.latency_ms, Some(900));
        assert_eq!(decoded.tool_calls, Some(2));
        assert_eq!(decoded.total, Some(145));
    }

    // T-011 (REQ-DS-027) reality test: the 4 optional DeepSeek per-call
    // overrides are backward-compatible. A Gemini/Codex request without them
    // deserialises with all 4 fields == None. A DeepSeek request with them
    // round-trips losslessly.
    //
    // Stub guard: a serializer that drops `Disabled` on round-trip (e.g. by
    // defaulting to Enabled at decode time) would fail the round-trip
    // assertion.
    #[test]
    fn ask_agent_request_optional_deepseek_backward_compatible() {
        // (1) Gemini-shape: no deepseek_* keys → all None.
        let gemini: super::AskAgentRequest =
            serde_json::from_str(r#"{"agent":"gemini","message":"x"}"#)
                .expect("gemini parse");
        assert_eq!(gemini.agent, "gemini");
        assert_eq!(gemini.deepseek_thinking, None);
        assert_eq!(gemini.deepseek_reasoning_effort, None);
        assert_eq!(gemini.deepseek_include_reasoning, None);
        assert_eq!(gemini.deepseek_max_tokens, None);
    }

    #[test]
    fn ask_agent_request_optional_deepseek_populates_and_round_trips() {
        let raw = r#"{
            "agent":"deepseek",
            "message":"x",
            "deepseek_thinking":"disabled",
            "deepseek_reasoning_effort":"xhigh",
            "deepseek_include_reasoning":true,
            "deepseek_max_tokens":512
        }"#;
        let req: super::AskAgentRequest = serde_json::from_str(raw).expect("parse");
        assert_eq!(req.deepseek_thinking, Some(super::DeepSeekThinking::Disabled));
        assert_eq!(
            req.deepseek_reasoning_effort,
            Some(super::DeepSeekEffort::Xhigh)
        );
        assert_eq!(req.deepseek_include_reasoning, Some(true));
        assert_eq!(req.deepseek_max_tokens, Some(512));

        // Round-trip must preserve all four values (stub guard).
        let json = serde_json::to_string(&req).expect("serialize");
        let again: super::AskAgentRequest = serde_json::from_str(&json).expect("re-parse");
        assert_eq!(again.deepseek_thinking, Some(super::DeepSeekThinking::Disabled));
        assert_eq!(again.deepseek_reasoning_effort, Some(super::DeepSeekEffort::Xhigh));
        assert_eq!(again.deepseek_include_reasoning, Some(true));
        assert_eq!(again.deepseek_max_tokens, Some(512));

        // Wire-shape stability: when None, the fields MUST be omitted (the
        // serialisation matches what Gemini/Codex clients have always sent).
        let bare = super::AskAgentRequest {
            agent: "gemini".to_string(),
            message: "x".to_string(),
            ..Default::default()
        };
        let bare_json = serde_json::to_string(&bare).expect("serialize");
        assert!(
            !bare_json.contains("deepseek_"),
            "None fields must be omitted from the wire; got: {bare_json}"
        );
    }

    #[test]
    fn deepseek_effort_accepts_all_five_levels() {
        for (s, expected) in &[
            ("low", super::DeepSeekEffort::Low),
            ("medium", super::DeepSeekEffort::Medium),
            ("high", super::DeepSeekEffort::High),
            ("max", super::DeepSeekEffort::Max),
            ("xhigh", super::DeepSeekEffort::Xhigh),
        ] {
            let raw = format!(r#""{}""#, s);
            let parsed: super::DeepSeekEffort =
                serde_json::from_str(&raw).expect("parse");
            assert_eq!(&parsed, expected);
        }
    }
}
