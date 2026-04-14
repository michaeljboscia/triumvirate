//! REST API response types for Pantheon v4.0 daemon endpoints.
//!
//! These types are the contract between the daemon (v3.9.0) and Pantheon (v4.0.0).
//! They are consumed by Pantheon's Tauri backend via reqwest, and emitted by
//! the daemon's new REST endpoints (/api/workers, /api/fleet, /api/state).
//!
//! FEAT-012 (REQ-017, REQ-020)

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Response for GET /api/workers — all active sessions and workers with lineage.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct WorkersResponse {
    pub workers: Vec<WorkerInfo>,
}

/// A single worker record with lineage fields for hierarchical sidebar display.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct WorkerInfo {
    pub session_id: String,
    pub agent: String,
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pantheon_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub started_at: String,
    pub elapsed_ms: u64,
}

/// Response for GET /api/fleet — all active ABE fleet builds.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct FleetResponse {
    pub builds: Vec<FleetBuild>,
}

/// A fleet build with per-task status.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct FleetBuild {
    pub build_id: String,
    pub task_count: u32,
    pub completed: u32,
    pub failed: u32,
    pub in_progress: u32,
    pub queued: u32,
    pub tasks: Vec<FleetTask>,
}

/// A single task within a fleet build.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct FleetTask {
    pub task_id: String,
    pub status: String,
    pub files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_session_id: Option<String>,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
}

/// Response for GET /api/state — full state snapshot for reconnect.
/// Clients use this when the WebSocket reconnects after a gap larger than
/// the event replay buffer can cover.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct StateResponse {
    pub version: String,
    pub uptime_ms: u64,
    pub workers: Vec<WorkerInfo>,
    pub fleet: Vec<FleetBuild>,
    pub last_event_seq: u64,
}

/// WebSocket replay request — client sends this on reconnect to request
/// events since its last seen sequence number.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ReplayRequest {
    pub action: String, // "subscribe"
    pub last_seq: u64,
}

/// WebSocket replay response — daemon returns this when the requested
/// seq is older than the ring buffer. Client must then fetch /api/state.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ReplayResponse {
    pub replay: String, // "ok" | "out_of_range"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_seq: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workers_response_roundtrips_with_full_lineage() {
        let response = WorkersResponse {
            workers: vec![WorkerInfo {
                session_id: "sess-abc123".into(),
                agent: "codex".into(),
                name: "codex-worker-1".into(),
                status: "working".into(),
                task_id: Some("T-001".into()),
                parent_session_id: Some("sess-main-1".into()),
                root_session_id: Some("sess-main-1".into()),
                pantheon_session_id: Some("uuid-from-pantheon".into()),
                cwd: Some("/Users/mikeboscia/projects/triumvirate".into()),
                started_at: "2026-04-11T22:00:00Z".into(),
                elapsed_ms: 45000,
            }],
        };

        let json = serde_json::to_string(&response).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Verify field names match the BACKEND_STRUCTURE.md contract exactly
        assert_eq!(value["workers"][0]["session_id"], "sess-abc123");
        assert_eq!(value["workers"][0]["agent"], "codex");
        assert_eq!(value["workers"][0]["parent_session_id"], "sess-main-1");
        assert_eq!(value["workers"][0]["pantheon_session_id"], "uuid-from-pantheon");

        let parsed: WorkersResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(response, parsed);
    }

    #[test]
    fn workers_response_omits_null_optional_fields() {
        let response = WorkersResponse {
            workers: vec![WorkerInfo {
                session_id: "sess-orphan".into(),
                agent: "gemini".into(),
                name: "gemini-1".into(),
                status: "idle".into(),
                task_id: None,
                parent_session_id: None,
                root_session_id: None,
                pantheon_session_id: None,
                cwd: None,
                started_at: "2026-04-11T22:00:00Z".into(),
                elapsed_ms: 0,
            }],
        };

        let json = serde_json::to_string(&response).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        // skip_serializing_if should omit None fields from JSON
        assert!(value["workers"][0].get("task_id").is_none());
        assert!(value["workers"][0].get("parent_session_id").is_none());
        assert!(value["workers"][0].get("pantheon_session_id").is_none());
        // Non-optional fields present
        assert_eq!(value["workers"][0]["session_id"], "sess-orphan");
    }

    #[test]
    fn fleet_response_roundtrips_with_nested_tasks() {
        let response = FleetResponse {
            builds: vec![FleetBuild {
                build_id: "build-001".into(),
                task_count: 6,
                completed: 3,
                failed: 0,
                in_progress: 2,
                queued: 1,
                tasks: vec![FleetTask {
                    task_id: "T-001".into(),
                    status: "committed".into(),
                    files: vec!["src/auth.rs".into()],
                    worker_session_id: Some("sess-abc123".into()),
                    elapsed_ms: 42000,
                    commit_sha: Some("a1b2c3d".into()),
                }],
            }],
        };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: FleetResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(response, parsed);

        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["builds"][0]["build_id"], "build-001");
        assert_eq!(value["builds"][0]["task_count"], 6);
        assert_eq!(value["builds"][0]["tasks"][0]["task_id"], "T-001");
        assert_eq!(value["builds"][0]["tasks"][0]["commit_sha"], "a1b2c3d");
    }

    #[test]
    fn state_response_has_all_pantheon_fields() {
        let response = StateResponse {
            version: "3.9.0".into(),
            uptime_ms: 3600000,
            workers: vec![],
            fleet: vec![],
            last_event_seq: 1542,
        };

        let json = serde_json::to_string(&response).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["version"], "3.9.0");
        assert_eq!(value["uptime_ms"], 3600000);
        assert_eq!(value["last_event_seq"], 1542);
        assert!(value["workers"].is_array());
        assert!(value["fleet"].is_array());
    }

    #[test]
    fn replay_request_and_response_contract() {
        let req = ReplayRequest {
            action: "subscribe".into(),
            last_seq: 1234,
        };
        let json = serde_json::to_string(&req).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["action"], "subscribe");
        assert_eq!(value["last_seq"], 1234);

        let ok_resp = ReplayResponse {
            replay: "ok".into(),
            oldest_seq: None,
        };
        let json = serde_json::to_string(&ok_resp).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["replay"], "ok");
        assert!(value.get("oldest_seq").is_none());

        let out_of_range = ReplayResponse {
            replay: "out_of_range".into(),
            oldest_seq: Some(2000),
        };
        let json = serde_json::to_string(&out_of_range).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["replay"], "out_of_range");
        assert_eq!(value["oldest_seq"], 2000);
    }
}
