use serde_json::Value;

use crate::LedgerStore;
use crate::store::with_task_state_priority;

#[allow(dead_code)]
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

#[allow(dead_code)]
fn extract_summary_type(payload_json: &str) -> String {
    match serde_json::from_str::<Value>(payload_json) {
        Ok(value) => value
            .get("summary_type")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| "extractive".to_string()),
        Err(_) => "extractive".to_string(),
    }
}

#[allow(dead_code)]
fn should_auto_create_lesson(summary_type: &str) -> bool {
    matches!(
        summary_type,
        "error_resolution" | "bug_fix" | "architecture_decision"
    )
}

fn reclaim_stale_running_conn(conn: &rusqlite::Connection) -> anyhow::Result<usize> {
    let rows = with_task_state_priority(|| {
        let rows = conn.execute(
            "UPDATE events
             SET compression_state = 'pending', compression_heartbeat = NULL
             WHERE compression_state = 'running'
               AND compression_heartbeat IS NOT NULL
               AND datetime(compression_heartbeat) < datetime('now', '-90 seconds')",
            [],
        )?;
        Ok(rows)
    })?;
    Ok(rows)
}

#[allow(dead_code)]
pub(crate) fn reclaim_stale_running(store: &LedgerStore) -> anyhow::Result<usize> {
    store.with_conn(reclaim_stale_running_conn)
}

#[allow(dead_code)]
pub(crate) fn process_pending_events(store: &LedgerStore) -> anyhow::Result<usize> {
    store.with_conn(|conn| {
        let _ = reclaim_stale_running_conn(conn)?;

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
            with_task_state_priority(|| {
                conn.execute(
                    "UPDATE events
                     SET compression_state = 'running',
                         compression_heartbeat = datetime('now')
                     WHERE id = ?1",
                    [event_id],
                )?;
                Ok(())
            })?;

            let tool_hint = extract_tool_hint(&payload_json);
            let summary_type = extract_summary_type(&payload_json);
            let title = format!("Event {event_id}: {event_type}");
            let narrative = format!("Extractive summary from {event_type}; tool={tool_hint}");
            let facts_json = serde_json::json!([format!("tool:{tool_hint}"), format!("event_type:{event_type}")]).to_string();

            let existing_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM summaries WHERE event_id = ?1",
                [event_id],
                |r| r.get(0),
            )?;
            if existing_count == 0 {
                with_task_state_priority(|| {
                    conn.execute(
                        "INSERT INTO summaries (event_id, title, narrative, facts_json, summary_type)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![event_id, title, narrative, facts_json, summary_type],
                    )?;
                    conn.execute(
                        "UPDATE sessions
                         SET summary_count = summary_count + 1
                         WHERE session_id IN (SELECT session_id FROM events WHERE id = ?1)",
                        [event_id],
                    )?;
                    if should_auto_create_lesson(&summary_type) {
                        conn.execute(
                            "INSERT INTO lessons (title, body, source_session_id, initial_confidence, tags_json, req_ids_json)
                             VALUES (
                                ?1,
                                ?2,
                                (SELECT session_id FROM events WHERE id = ?3),
                                0.6,
                                ?4,
                                NULL
                             )",
                            rusqlite::params![
                                title,
                                narrative,
                                event_id,
                                serde_json::json!([summary_type]).to_string()
                            ],
                        )?;
                    }
                    Ok(())
                })?;
            }

            with_task_state_priority(|| {
                conn.execute(
                    "UPDATE events
                     SET compression_state = 'done', compression_heartbeat = NULL
                     WHERE id = ?1",
                    [event_id],
                )?;
                Ok(())
            })?;
            processed += 1;
        }

        Ok(processed)
    })
}
