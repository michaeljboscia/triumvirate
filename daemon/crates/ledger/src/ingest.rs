use shared_types::{ManualRecord, RawEvent};

use crate::LedgerStore;
use crate::pool::{reap_idle, register_activity};
use crate::store::with_ingest_priority;

pub(crate) fn ingest_event(store: &LedgerStore, event: RawEvent) -> anyhow::Result<()> {
    let session_id = event.session_id.clone();
    let event_timestamp = event.timestamp.clone();
    register_activity(store.project_root());
    reap_idle();
    with_ingest_priority(|| {
        store.with_conn(|conn| {
            let inserted = conn.execute(
                "INSERT INTO events (session_id, event_type, sequence, timestamp, payload_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(session_id, event_type, sequence) DO NOTHING",
                rusqlite::params![
                    event.session_id,
                    event.event_type,
                    event.sequence,
                    event.timestamp,
                    event.payload_json
                ],
            )?;
            if inserted == 1 {
                conn.execute(
                    "INSERT INTO sessions (session_id, project, branch, started_at, event_count, summary_count)
                     VALUES (?1, ?2, NULL, ?3, 1, 0)
                     ON CONFLICT(session_id) DO UPDATE SET
                        event_count = sessions.event_count + 1",
                    rusqlite::params![
                        session_id,
                        store.project_root().display().to_string(),
                        event_timestamp
                    ],
                )?;
            }
            Ok(())
        })
    })?;
    Ok(())
}

pub(crate) fn record_manual(store: &LedgerStore, record: ManualRecord) -> anyhow::Result<()> {
    with_ingest_priority(|| {
        store.with_conn(|conn| {
            conn.execute(
                "INSERT INTO summaries (event_id, title, narrative, facts_json, concepts_json, affected_files_json, summary_type)
                 VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    record.title,
                    record.narrative,
                    record.facts_json,
                    record.concepts_json,
                    record.affected_files_json,
                    record.summary_type
                ],
            )?;
            Ok(())
        })
    })
}
