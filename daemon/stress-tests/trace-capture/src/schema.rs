use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraceEvent {
    pub event_id: Uuid,
    pub event_type: String,
    pub subject: String,
    pub schema_version: u16,
    pub emitted_at: DateTime<Utc>,
    pub correlation_id: Option<Uuid>,
    pub payload: Value,
}

impl TraceEvent {
    pub fn new(
        event_type: impl Into<String>,
        subject: impl Into<String>,
        correlation_id: Option<Uuid>,
        payload: Value,
    ) -> Self {
        Self {
            event_id: Uuid::now_v7(),
            event_type: event_type.into(),
            subject: subject.into(),
            schema_version: 1,
            emitted_at: Utc::now(),
            correlation_id,
            payload,
        }
    }
}
