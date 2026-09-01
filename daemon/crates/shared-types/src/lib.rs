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

    /// Resume the cached worker session for (agent, cwd) instead of starting a
    /// fresh one. Opt-in, and `None`/`false` for every one-shot caller.
    ///
    /// Resuming replays the ENTIRE prior transcript as input on every turn (for
    /// codex, `codex exec resume <id>`), so the cost of a call tracks the age of
    /// the session, not the size of the question — a one-word ask on a long-lived
    /// session was measured at 189,930 input tokens against 26,215 fresh. Only
    /// named sessions (`ask_session`/`ask_daemon`) want that; they set this true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reuse_session: Option<bool>,
    /// Which NAMED session this call belongs to, when it belongs to one.
    ///
    /// The worker registry was keyed on `(agent, cwd)` alone, so two named sessions for the same
    /// agent in the same directory shared one worker record and therefore one grok/codex session
    /// id. Demonstrated live: a secret told only to session A came back verbatim from session B,
    /// with tools explicitly forbidden so no shared store was involved.
    ///
    /// `None` keeps the old `(agent, cwd)` behavior for one-shot `ask_agent`, which is correct
    /// there: an unnamed call has no session of its own to keep separate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_key: Option<String>,
    /// The CLI session id this NAMED session already owns, from its own `SessionState`.
    ///
    /// When set it is authoritative and the worker registry is not consulted. The registry stays
    /// as a fallback so sessions created before ownership moved keep resuming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_cli_session_id: Option<String>,

    /// This call is a REVIEW, so an answer produced without looking at anything is not an
    /// answer. When true, a turn that completes with zero tool calls is rejected instead of
    /// returned.
    ///
    /// 2026-09-01: three peers were dispatched with filesystem access to review one brief.
    /// Codex made 35 tool calls and cited file and line. Grok opened four primary sources and
    /// found the only error that had actually reached a client. The third, holding the widest
    /// permissions of the three, made ZERO calls, then graded nine research citations from
    /// memory and opened with "the claims below were subjected to rigorous sourcing". It was
    /// caught by a human noticing the output had no links in it.
    ///
    /// This is ISO/IEC 27042's validation step pointed at the reviewer rather than the
    /// evidence: before accepting a finding, establish that the method could have seen the
    /// thing. A peer that opened nothing cannot have verified anything, whatever its prose says.
    ///
    /// Deliberately a hard failure and not a warning. The same session established that a
    /// generated objection which does not stop the caller gets quoted approvingly and the
    /// wrong conclusion ships anyway.
    ///
    /// `None`/`false` leaves every existing one-shot caller unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_sight: Option<bool>,

    /// The primary sources this review must actually open, by path.
    ///
    /// `require_sight` alone only proves the agent made SOME tool call, which passes on one
    /// `todo_write`, one `list_dir .`, or one `pwd`. That is citation, not method: it demands a
    /// look be cited once a look has happened, without forcing the look at the thing in
    /// question. Naming the sources here turns the gate from "did it do anything" into "did it
    /// request the evidence", which is the validation step the forensic standard actually asks
    /// for.
    ///
    /// Matched against the recorded arguments of SUCCESSFUL read-shaped tool calls. A failed
    /// read saw nothing and does not count.
    ///
    /// This proves the method requested the named thing. It does NOT prove the contents were
    /// used: an agent can open every source and still answer from memory. That layer is not
    /// built, and this field should not be described as if it were.
    ///
    /// Empty or absent falls back to the weaker any-tool-call check.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_sources: Vec<String>,

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

/// A REVIEW dispatch. Sight is not optional here, which is the entire point of the type.
///
/// `ask_agent` carries `require_sight` as a flag, and a flag defaults to off and gets forgotten.
/// Grok, twice: "a skip-catcher that is off unless remembered is not a skip-catcher." A caller
/// who wants a review reaches for a tool called review, and the tool decides the gate.
///
/// `sources` is REQUIRED and must be non-empty. A review with nothing to read is either a
/// review of an inline artifact, which belongs on `ask_agent`, or it is a review of nothing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviewRequestParams {
    /// Which peer reviews. Same names and aliases as `ask_agent`.
    pub agent: String,
    /// What to review, and what to look for.
    pub message: String,
    /// Absolute paths the reviewer MUST successfully read. Non-empty.
    ///
    /// Absolute, because agy runs its tools from its own scratch directory rather than the
    /// dispatch cwd, so a relative path is not resolvable there.
    pub sources: Vec<String>,
    pub cwd: Option<String>,
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
    /// The agent CLI's session id for this turn, when there is one.
    ///
    /// Returned so a NAMED session can persist it in its own `SessionState` instead of the worker
    /// registry inferring it from `(agent, cwd)`. That inference is what let two named sessions in
    /// one directory resume each other. Omitted from the wire when absent so existing clients see
    /// an unchanged shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_session_id: Option<String>,
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
    /// How many tool calls the agent made producing this answer. The receipt.
    ///
    /// Always populated, not just when `require_sight` is set, because the count is the
    /// cheapest way for a caller to tell a review apart from a recollection. Zero on a
    /// question that needed no tools is perfectly normal; zero on a review is the finding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls_made: Option<u32>,
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
            cli_session_id: None,
            answered_by_agent: None,
            answered_by_backend: None,
            degraded_from_backend: None,
            degradation_reason: None,
            shadow_backend: None,
            shadow_response: None,
            shadow_error: None,
            shadow_latency_ms: None,
            tool_calls_made: None,
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

    /// The primary sources this turn must actually open, by path.
    ///
    /// A multi-turn review could not be gated at all before this: `ask_session`, `ask_daemon`
    /// and HTTP `/session/ask` all built `AskAgentRequest { ..Default::default() }`, so
    /// neither sight field could ever be set. Codex found that in its route survey.
    ///
    /// Naming sources implies `require_sight`, so this one field is enough to gate a session
    /// turn. Empty leaves every existing caller behaving exactly as before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_sources: Vec<String>,

    /// Gate this turn on the reviewer having used tools at all, without naming sources.
    /// Weaker than `required_sources` and useful when the evidence is not a fixed file set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_sight: Option<bool>,
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
    /// The AGENT CLI's session id for this named session.
    ///
    /// This used to live in the worker registry, keyed on `(agent, cwd)`, which is what let two
    /// named sessions in one directory resume each other. All three peers independently landed on
    /// the same fix: the logical session (this history) and the physical session (the CLI id) must
    /// have ONE owner, and it is this struct.
    ///
    /// With the id here, an anonymous one-shot ask physically cannot leak or inherit one, because
    /// it has no `SessionState` to read it from.
    ///
    /// `#[serde(default)]` so a sessions file written before this field loads cleanly; the
    /// migration in `hydrate_session_ids_from_workers` fills it from the old registry once.
    #[serde(default)]
    pub cli_session_id: Option<String>,
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
            cli_session_id: None,
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
