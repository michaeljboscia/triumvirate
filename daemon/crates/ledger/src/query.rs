use shared_types::{RawEvent, SessionDetail, Summary};
use rusqlite::OptionalExtension;

use crate::LedgerStore;

type SessionRow = (String, Option<String>, String, Option<String>, i64, i64);

pub(crate) fn query_summaries(
    store: &LedgerStore,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<Summary>> {
    store.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT s.id, s.event_id, s.title, s.narrative, s.facts_json, s.concepts_json,
                    s.affected_files_json, s.summary_type, s.created_at
             FROM summaries_fts f
             JOIN summaries s ON s.id = f.rowid
             WHERE summaries_fts MATCH ?1
             ORDER BY bm25(summaries_fts)
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![query, limit as i64], |row| {
            Ok(Summary {
                id: row.get(0)?,
                event_id: row.get(1)?,
                title: row.get(2)?,
                narrative: row.get(3)?,
                facts_json: row.get(4)?,
                concepts_json: row.get(5)?,
                affected_files_json: row.get(6)?,
                summary_type: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })
}

pub(crate) fn get_session_detail(
    store: &LedgerStore,
    session_id: &str,
) -> anyhow::Result<SessionDetail> {
    store.with_conn(|conn| {
        let mut event_stmt = conn.prepare(
            "SELECT session_id, event_type, sequence, timestamp, payload_json
             FROM events
             WHERE session_id = ?1
             ORDER BY sequence ASC, id ASC",
        )?;
        let event_rows = event_stmt.query_map([session_id], |row| {
            Ok(RawEvent {
                session_id: row.get(0)?,
                event_type: row.get(1)?,
                sequence: row.get(2)?,
                timestamp: row.get(3)?,
                payload_json: row.get(4)?,
            })
        })?;
        let mut events = Vec::new();
        for row in event_rows {
            events.push(row?);
        }

        let mut summary_stmt = conn.prepare(
            "SELECT s.id, s.event_id, s.title, s.narrative, s.facts_json, s.concepts_json,
                    s.affected_files_json, s.summary_type, s.created_at
             FROM summaries s
             LEFT JOIN events e ON e.id = s.event_id
             WHERE e.session_id = ?1
             ORDER BY s.id ASC",
        )?;
        let summary_rows = summary_stmt.query_map([session_id], |row| {
            Ok(Summary {
                id: row.get(0)?,
                event_id: row.get(1)?,
                title: row.get(2)?,
                narrative: row.get(3)?,
                facts_json: row.get(4)?,
                concepts_json: row.get(5)?,
                affected_files_json: row.get(6)?,
                summary_type: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;
        let mut summaries = Vec::new();
        for row in summary_rows {
            summaries.push(row?);
        }

        if events.is_empty() {
            anyhow::bail!("session not found: {session_id}");
        }

        let session_row: Option<SessionRow> = conn
            .query_row(
                "SELECT project, branch, started_at, ended_at, event_count, summary_count
                 FROM sessions
                 WHERE session_id = ?1",
                [session_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?;

        let (project, branch, started_at, ended_at, event_count, summary_count) =
            if let Some(row) = session_row {
                row
            } else {
                (
                    store.project_root().display().to_string(),
                    None,
                    events
                        .first()
                        .map(|e| e.timestamp.clone())
                        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string()),
                    None,
                    events.len() as i64,
                    summaries.len() as i64,
                )
            };

        Ok(SessionDetail {
            session_id: session_id.to_string(),
            project,
            branch,
            started_at,
            ended_at,
            event_count,
            summary_count,
            events,
            summaries,
        })
    })
}
