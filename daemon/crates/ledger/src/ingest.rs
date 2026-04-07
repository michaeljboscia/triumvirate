use shared_types::RawEvent;

use crate::LedgerStore;
use crate::compression::process_pending_events;
use crate::pool::{reap_idle, register_activity};
use crate::store::with_ingest_priority;

pub(crate) fn ingest_event(store: &LedgerStore, event: RawEvent) -> anyhow::Result<()> {
    register_activity(store.project_root());
    reap_idle();
    with_ingest_priority(|| {
        store.with_conn(|conn| {
            conn.execute(
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
            Ok(())
        })
    })?;
    let _ = process_pending_events(store)?;
    Ok(())
}
