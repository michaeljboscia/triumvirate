use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct RawEvent {
    pub session_id: String,
    pub event_type: String,
    pub sequence: i64,
    pub timestamp: String,
    pub payload_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Summary {
    pub id: Option<i64>,
    pub event_id: Option<i64>,
    pub title: String,
    pub narrative: String,
    pub facts_json: Option<String>,
    pub concepts_json: Option<String>,
    pub affected_files_json: Option<String>,
    pub summary_type: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct HealthStatus {
    pub last_event_timestamp: Option<String>,
    pub events_last_5min: i64,
    pub queue_depth: i64,
    pub spool_size_bytes: i64,
    pub db_size_bytes: i64,
    pub stale_jobs: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct DrainResult {
    pub ingested_count: usize,
    pub skipped_count: usize,
    pub failed_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct NewLesson {
    pub title: String,
    pub body: String,
    pub source_session_id: Option<String>,
    pub initial_confidence: f64,
    pub tags_json: Option<String>,
    pub req_ids_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Lesson {
    pub lesson_id: i64,
    pub title: String,
    pub body: String,
    pub source_session_id: Option<String>,
    pub created_at: String,
    pub last_validated_at: String,
    pub initial_confidence: f64,
    pub tags_json: Option<String>,
    pub req_ids_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct GcResult {
    pub removed_events: usize,
    pub removed_summaries: usize,
    pub removed_lessons: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ManualRecord {
    pub session_id: Option<String>,
    pub title: String,
    pub narrative: String,
    pub facts_json: Option<String>,
    pub concepts_json: Option<String>,
    pub affected_files_json: Option<String>,
    pub summary_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct SessionDetail {
    pub session_id: String,
    pub project: String,
    pub branch: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub event_count: i64,
    pub summary_count: i64,
    pub events: Vec<RawEvent>,
    pub summaries: Vec<Summary>,
}

#[cfg(test)]
mod tests {
    use super::{
        DrainResult, GcResult, HealthStatus, Lesson, ManualRecord, NewLesson, RawEvent,
        SessionDetail, Summary,
    };

    fn assert_roundtrip<T>(value: &T)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let encoded = serde_json::to_string(value).expect("serialize");
        let decoded: T = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(&decoded, value);
    }

    #[test]
    fn ledger_dtos_roundtrip_json() {
        assert_roundtrip(&RawEvent {
            session_id: "sess-1".to_string(),
            event_type: "PostToolUse".to_string(),
            sequence: 42,
            timestamp: "2026-04-07T00:00:00Z".to_string(),
            payload_json: "{\"ok\":true}".to_string(),
        });
        assert_roundtrip(&Summary {
            id: Some(1),
            event_id: Some(42),
            title: "auth middleware bug".to_string(),
            narrative: "Fixed race in request auth path".to_string(),
            facts_json: Some("[\"race\",\"auth\"]".to_string()),
            concepts_json: Some("[\"middleware\"]".to_string()),
            affected_files_json: Some("[\"src/auth.rs\"]".to_string()),
            summary_type: "bug_fix".to_string(),
            created_at: Some("2026-04-07T00:00:01Z".to_string()),
        });
        assert_roundtrip(&HealthStatus {
            last_event_timestamp: Some("2026-04-07T00:00:01Z".to_string()),
            events_last_5min: 7,
            queue_depth: 0,
            spool_size_bytes: 128,
            db_size_bytes: 4096,
            stale_jobs: 0,
            status: "healthy".to_string(),
        });
        assert_roundtrip(&DrainResult {
            ingested_count: 3,
            skipped_count: 1,
            failed_count: 0,
        });
        assert_roundtrip(&NewLesson {
            title: "Prefer WAL for write contention".to_string(),
            body: "WAL mode prevented lock stalls.".to_string(),
            source_session_id: Some("sess-1".to_string()),
            initial_confidence: 0.8,
            tags_json: Some("[\"sqlite\",\"wal\"]".to_string()),
            req_ids_json: Some("[\"REQ-008\"]".to_string()),
        });
        assert_roundtrip(&Lesson {
            lesson_id: 9,
            title: "WAL avoids write lock contention".to_string(),
            body: "Enable WAL mode before ingestion.".to_string(),
            source_session_id: Some("sess-1".to_string()),
            created_at: "2026-04-07T00:00:02Z".to_string(),
            last_validated_at: "2026-04-07T00:00:03Z".to_string(),
            initial_confidence: 0.8,
            tags_json: Some("[\"sqlite\"]".to_string()),
            req_ids_json: Some("[\"REQ-008\"]".to_string()),
        });
        assert_roundtrip(&GcResult {
            removed_events: 2,
            removed_summaries: 1,
            removed_lessons: 0,
        });
        assert_roundtrip(&ManualRecord {
            session_id: Some("sess-1".to_string()),
            title: "Manual note".to_string(),
            narrative: "Captured architecture decision".to_string(),
            facts_json: Some("[\"decision\"]".to_string()),
            concepts_json: Some("[\"architecture\"]".to_string()),
            affected_files_json: Some("[\"daemon/src/main.rs\"]".to_string()),
            summary_type: "architecture_decision".to_string(),
        });
        assert_roundtrip(&SessionDetail {
            session_id: "sess-1".to_string(),
            project: "/tmp/repo".to_string(),
            branch: Some("feat/mcp-first".to_string()),
            started_at: "2026-04-07T00:00:00Z".to_string(),
            ended_at: Some("2026-04-07T00:10:00Z".to_string()),
            event_count: 10,
            summary_count: 3,
            events: vec![RawEvent {
                session_id: "sess-1".to_string(),
                event_type: "PostToolUse".to_string(),
                sequence: 1,
                timestamp: "2026-04-07T00:00:01Z".to_string(),
                payload_json: "{}".to_string(),
            }],
            summaries: vec![Summary {
                id: Some(1),
                event_id: Some(1),
                title: "A".to_string(),
                narrative: "B".to_string(),
                facts_json: None,
                concepts_json: None,
                affected_files_json: None,
                summary_type: "extractive".to_string(),
                created_at: Some("2026-04-07T00:00:02Z".to_string()),
            }],
        });
    }
}
