pub mod schema;
pub mod sink;

pub use schema::TraceEvent;
pub use sink::JsonlSink;

pub const TOOL_CALL_STARTED: &str = "tool_call.started";
pub const TOOL_CALL_COMPLETED: &str = "tool_call.completed";
pub const WORKER_SPAWNED: &str = "worker.spawned";
pub const WORKER_STATE_CHANGED: &str = "worker.state_changed";
pub const WORKER_COMPLETED: &str = "worker.completed";
pub const PEER_REVIEW_REQUESTED: &str = "peer_review.requested";
pub const PEER_REVIEW_DECIDED: &str = "peer_review.decided";
pub const COST_TOKEN_USAGE: &str = "cost.token_usage";
pub const COST_API_CALL: &str = "cost.api_call";
pub const LESSON_CANDIDATE: &str = "lesson.candidate";
