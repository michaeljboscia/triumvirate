use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Which agent produced or should receive this message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentId {
    Claude,
    Gemini,
    Codex,
    Human,
    System,
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Claude => write!(f, "claude"),
            Self::Gemini => write!(f, "gemini"),
            Self::Codex => write!(f, "codex"),
            Self::Human => write!(f, "human"),
            Self::System => write!(f, "system"),
        }
    }
}

/// Health states for an agent connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Starting,
    Ready,
    Busy,
    Unresponsive,
    Restarting,
    Dead,
}

/// A message flowing through the fabric.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricMessage {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub source: AgentId,
    pub topic: Topic,
    pub payload: Payload,
}

impl FabricMessage {
    pub fn new(source: AgentId, topic: Topic, payload: Payload) -> Self {
        Self {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            source,
            topic,
            payload,
        }
    }
}

/// Topics mirror the NATS topic structure from the spec.
/// When we add real NATS, these map 1:1 to NATS subjects.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    /// Directed human input for a specific agent: agents.<name>.input
    AgentInput(AgentId),
    /// Agent streaming output: agents.<name>.output
    AgentOutput(AgentId),
    /// Human input: agents.human.input
    HumanInput,
    /// Broadcast to all agents
    Broadcast,
    /// Debate proposals
    DebateProposal,
    /// Debate challenges
    DebateChallenge,
    /// Debate votes
    DebateVote,
    /// Task lifecycle
    TaskCreated,
    TaskProgress,
    TaskCompleted,
    /// Memory operations
    MemoryWrite,
    MemoryRead,
    /// System events (health, errors)
    SystemHealth,
    SystemError,
}

/// Message payloads — the actual data flowing through the fabric.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Payload {
    /// Streaming text from an agent
    TextChunk {
        content: String,
        #[serde(default)]
        is_final: bool,
    },
    /// Complete agent response (after streaming finishes)
    AgentResponse {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        tokens_used: Option<u64>,
    },
    /// Human typed a message
    HumanMessage { content: String },
    /// Agent health changed
    HealthChange {
        agent: AgentId,
        status: HealthStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// Error from any subsystem
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_agent: Option<AgentId>,
    },
    /// Routing metadata emitted by the router for traceability.
    RoutingDecision {
        target_agent: AgentId,
        reason: String,
        content: String,
    },
    /// Memory write request (syntax-gated via # DECISION: keyword)
    MemoryEntry {
        key: String,
        value: String,
        memory_type: String,
    },
}

/// Session metadata for the stenographer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub agents_involved: Vec<AgentId>,
    pub working_directory: String,
}
