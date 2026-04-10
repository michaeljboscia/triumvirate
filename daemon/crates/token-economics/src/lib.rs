mod attribution;
mod direct;
mod queries;
mod scanner;
mod storage;

use std::sync::Mutex;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub use attribution::{OutboxAttributionEvent, attribute_records};
pub use direct::record_daemon_tokens;
pub use queries::{
    AgentTokenSummary, BuildCostBreakdown, BuildTaskCost, SessionTokenBreakdown, SummaryQueryFilters,
    TimeRange, by_build_query, by_session_query, summary_query,
};
pub use storage::{insert_record, open, query_summary};
pub use scanner::{
    scan_claude_file, scan_codex_file, scan_gemini_chat_file, scan_gemini_telemetry_file,
};

#[derive(Debug)]
pub struct TokenDb {
    pub(crate) conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenRecord {
    pub agent: String,
    pub session_id: String,
    pub timestamp: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub thinking_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: Option<f64>,
    pub latency_ms: Option<i64>,
    pub tool_calls: Option<i64>,
    pub lines_added: Option<i64>,
    pub lines_removed: Option<i64>,
    pub rate_limit_pct: Option<f64>,
    pub context_window: Option<i64>,
    pub build_id: Option<String>,
    pub task_id: Option<String>,
    pub wave: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenSummaryRow {
    pub id: i64,
    pub agent: String,
    pub session_id: String,
    pub timestamp: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub thinking_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: Option<f64>,
    pub latency_ms: Option<i64>,
    pub tool_calls: Option<i64>,
    pub lines_added: Option<i64>,
    pub lines_removed: Option<i64>,
    pub rate_limit_pct: Option<f64>,
    pub context_window: Option<i64>,
    pub build_id: Option<String>,
    pub task_id: Option<String>,
    pub wave: Option<i64>,
}
