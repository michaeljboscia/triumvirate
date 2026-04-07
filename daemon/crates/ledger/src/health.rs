use std::{fs, path::Path};

use shared_types::HealthStatus;

use crate::LedgerStore;
use crate::store::queue_lag_seconds_conn;

fn dir_size_bytes(path: &Path) -> i64 {
    if !path.exists() {
        return 0;
    }
    let mut total: u128 = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.filter_map(Result::ok) {
            let file_type = match entry.file_type() {
                Ok(v) => v,
                Err(_) => continue,
            };
            if file_type.is_file() {
                if let Ok(meta) = entry.metadata() {
                    total = total.saturating_add(meta.len() as u128);
                }
            } else if file_type.is_dir() {
                total = total.saturating_add(dir_size_bytes(&entry.path()) as u128);
            }
        }
    }
    total.min(i64::MAX as u128) as i64
}

pub(crate) fn health(store: &LedgerStore) -> anyhow::Result<HealthStatus> {
    let spool_dir = store.project_root().join(".triumvirate").join("spool");
    let db_path = store.project_root().join(".triumvirate").join("ledger.db");

    store.with_conn(|conn| {
        let last_event_timestamp: Option<String> =
            conn.query_row("SELECT MAX(timestamp) FROM events", [], |row| row.get(0))?;

        let events_last_5min: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE datetime(timestamp) >= datetime('now', '-5 minutes')",
            [],
            |row| row.get(0),
        )?;

        let queue_depth: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE compression_state IN ('pending', 'running')",
            [],
            |row| row.get(0),
        )?;

        let stale_jobs: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events
             WHERE compression_state = 'running'
             AND compression_heartbeat IS NOT NULL
             AND datetime(compression_heartbeat) < datetime('now', '-90 seconds')",
            [],
            |row| row.get(0),
        )?;

        let active_sessions: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE ended_at IS NULL",
            [],
            |row| row.get(0),
        )?;

        let db_size_bytes = fs::metadata(&db_path).map(|m| m.len() as i64).unwrap_or(0);
        let spool_size_bytes = dir_size_bytes(&spool_dir);

        let status = if stale_jobs > 0 || (events_last_5min == 0 && active_sessions > 0) {
            "degraded".to_string()
        } else if queue_lag_seconds_conn(conn)? > 5.0 {
            "degraded".to_string()
        } else {
            "healthy".to_string()
        };

        Ok(HealthStatus {
            last_event_timestamp,
            events_last_5min,
            queue_depth,
            spool_size_bytes,
            db_size_bytes,
            stale_jobs,
            status,
        })
    })
}
