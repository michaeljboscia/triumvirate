use serde_json::Value;

use crate::LedgerStore;

fn extract_tool_hint(payload_json: &str) -> String {
    match serde_json::from_str::<Value>(payload_json) {
        Ok(value) => {
            if let Some(tool) = value.get("tool").and_then(Value::as_str) {
                return tool.to_string();
            }
            if let Some(event_type) = value.get("event_type").and_then(Value::as_str) {
                return event_type.to_string();
            }
            "unknown".to_string()
        }
        Err(_) => "unknown".to_string(),
    }
}

pub(crate) fn process_pending_events(store: &LedgerStore) -> anyhow::Result<usize> {
    store.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, event_type, payload_json
             FROM events
             WHERE compression_state = 'pending'
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;

        let mut processed = 0usize;
        for row in rows {
            let (event_id, event_type, payload_json) = row?;
            let tool_hint = extract_tool_hint(&payload_json);
            let title = format!("Event {event_id}: {event_type}");
            let narrative = format!("Extractive summary from {event_type}; tool={tool_hint}");
            let facts_json = serde_json::json!([format!("tool:{tool_hint}"), format!("event_type:{event_type}")]).to_string();

            let existing_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM summaries WHERE event_id = ?1",
                [event_id],
                |r| r.get(0),
            )?;
            if existing_count == 0 {
                conn.execute(
                    "INSERT INTO summaries (event_id, title, narrative, facts_json, summary_type)
                     VALUES (?1, ?2, ?3, ?4, 'extractive')",
                    rusqlite::params![event_id, title, narrative, facts_json],
                )?;
            }

            conn.execute(
                "UPDATE events
                 SET compression_state = 'done', compression_heartbeat = NULL
                 WHERE id = ?1",
                [event_id],
            )?;
            processed += 1;
        }

        Ok(processed)
    })
}
