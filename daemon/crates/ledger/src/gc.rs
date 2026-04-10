use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

use rusqlite::{OptionalExtension, params};
use shared_types::GcResult;
use tracing::instrument;

use crate::LedgerStore;

const EVENT_RETENTION_DAYS: i64 = 30;
const DEAD_DROP_RETENTION_DAYS: u64 = 7;

#[instrument(
    skip_all,
    fields(
        event_type = "gc",
        spool_size = tracing::field::Empty,
        operation = "gc"
    )
)]
pub(crate) fn gc(store: &LedgerStore) -> anyhow::Result<GcResult> {
    gc_at(store, SystemTime::now())
}

#[instrument(
    skip_all,
    fields(
        event_type = "fleet_state_check",
        spool_size = tracing::field::Empty,
        operation = "has_active_fleets"
    )
)]
pub(crate) fn has_active_fleets(store: &LedgerStore) -> anyhow::Result<bool> {
    store.with_conn(|conn| {
        let has_active: i64 = conn.query_row(
            "SELECT EXISTS(
                SELECT 1
                FROM fleets
                WHERE state NOT IN ('done', 'failed')
            )",
            [],
            |row| row.get(0),
        )?;
        Ok(has_active == 1)
    })
}

#[instrument(
    skip_all,
    fields(
        event_type = "gc_gate_check",
        spool_size = tracing::field::Empty,
        operation = "should_run_startup_gc"
    )
)]
pub(crate) fn should_run_startup_gc(store: &LedgerStore) -> anyhow::Result<bool> {
    if has_active_fleets(store)? {
        return Ok(false);
    }

    store.with_conn(|conn| {
        let hours_since_last_gc: Option<f64> = conn
            .query_row(
                "SELECT (julianday('now') - julianday(timestamp)) * 24.0
                 FROM health
                 WHERE status = 'gc_completed'
                 ORDER BY timestamp DESC
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(hours_since_last_gc.map(|hours| hours >= 24.0).unwrap_or(true))
    })
}

#[instrument(
    skip_all,
    fields(
        event_type = "gc_timestamp_query",
        spool_size = tracing::field::Empty,
        operation = "last_gc_timestamp"
    )
)]
pub(crate) fn last_gc_timestamp(store: &LedgerStore) -> anyhow::Result<Option<String>> {
    store.with_conn(|conn| {
        let value: Option<String> = conn
            .query_row(
            "SELECT timestamp
             FROM health
             WHERE status = 'gc_completed'
             ORDER BY timestamp DESC
             LIMIT 1",
            [],
            |row| row.get(0),
        )
            .optional()?;
        Ok(value)
    })
}

fn gc_at(store: &LedgerStore, now: SystemTime) -> anyhow::Result<GcResult> {
    let db_path = store.project_root().join(".triumvirate").join("ledger.db");
    let dead_drop_dir = store.project_root().join(".triumvirate").join("dead-drop");
    let db_before = file_size(&db_path);
    let dead_drop_before = dir_size(&dead_drop_dir)?;

    let (events_scanned, events_deleted) = store.with_conn(|conn| {
        let events_scanned: i64 = conn.query_row(
            "SELECT COUNT(*)
             FROM events e
             WHERE datetime(e.created_at) < datetime('now', ?1)
               AND NOT EXISTS (
                 SELECT 1
                 FROM summaries s
                 WHERE s.event_id = e.id
               )",
            params![format!("-{EVENT_RETENTION_DAYS} days")],
            |row| row.get(0),
        )?;

        let events_deleted = conn.execute(
            "DELETE FROM events
             WHERE datetime(created_at) < datetime('now', ?1)
               AND NOT EXISTS (
                 SELECT 1
                 FROM summaries
                 WHERE summaries.event_id = events.id
               )",
            params![format!("-{EVENT_RETENTION_DAYS} days")],
        )?;

        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;

        Ok((events_scanned as usize, events_deleted))
    })?;

    let dead_drop_deleted = gc_dead_drop_tickets(&dead_drop_dir, now, DEAD_DROP_RETENTION_DAYS)?;

    let db_after = file_size(&db_path);
    let dead_drop_after = dir_size(&dead_drop_dir)?;
    let space_reclaimed_bytes = db_before
        .saturating_add(dead_drop_before)
        .saturating_sub(db_after.saturating_add(dead_drop_after));

    let result = GcResult {
        events_scanned,
        events_deleted,
        space_reclaimed_bytes,
        dead_drop_deleted,
    };

    store.with_conn(|conn| {
        conn.execute(
            "INSERT INTO health (db_size_bytes, spool_size_bytes, queue_depth, status)
             VALUES (?1, ?2, ?3, 'gc_completed')",
            params![db_after as i64, dead_drop_after as i64, 0_i64],
        )?;
        Ok(())
    })?;

    Ok(result)
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn dir_size(path: &Path) -> anyhow::Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0_u64;
    for entry in fs::read_dir(path)?.filter_map(Result::ok) {
        if let Ok(meta) = entry.metadata()
            && meta.is_file()
        {
            total = total.saturating_add(meta.len());
        }
    }
    Ok(total)
}

fn gc_dead_drop_tickets(dir: &Path, now: SystemTime, max_age_days: u64) -> anyhow::Result<usize> {
    if !dir.exists() {
        return Ok(0);
    }
    let max_age = Duration::from_secs(max_age_days.saturating_mul(24 * 60 * 60));
    let mut removed = 0usize;

    for entry in fs::read_dir(dir)?.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let Ok(age) = now.duration_since(modified) else {
            continue;
        };
        if age >= max_age && fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::LedgerStore;

    #[test]
    fn gc_removes_only_unsummarized_stale_events_and_old_dead_drop() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("dead-drop"))?;
        let store = LedgerStore::open(project_root.clone())?;

        store.with_conn(|conn| {
            conn.execute(
                "INSERT INTO events (session_id, event_type, sequence, timestamp, payload_json, created_at)
                 VALUES ('s1', 'PostToolUse', 1, '2026-01-01T00:00:00Z', '{}', datetime('now', '-31 days'))",
                [],
            )?;
            conn.execute(
                "INSERT INTO events (session_id, event_type, sequence, timestamp, payload_json, created_at)
                 VALUES ('s2', 'PostToolUse', 1, '2026-01-01T00:00:00Z', '{}', datetime('now', '-31 days'))",
                [],
            )?;
            conn.execute(
                "INSERT INTO summaries (event_id, title, narrative, summary_type)
                 SELECT id, 'keep', 'keep', 'extractive' FROM events WHERE session_id='s2'",
                [],
            )?;
            Ok(())
        })?;

        let stale_ticket = project_root
            .join(".triumvirate")
            .join("dead-drop")
            .join("ticket-old.md");
        fs::write(&stale_ticket, "old ticket")?;
        let simulated_now = SystemTime::now() + Duration::from_secs(8 * 24 * 60 * 60);
        let result = gc_at(&store, simulated_now)?;
        assert_eq!(result.events_deleted, 1);
        assert_eq!(result.events_scanned, 1);
        assert_eq!(result.dead_drop_deleted, 1);

        let remaining: i64 = store.with_conn(|conn| {
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
            Ok(count)
        })?;
        assert_eq!(remaining, 1);
        Ok(())
    }

    #[test]
    fn startup_gc_gate_respects_last_gc_age_and_active_fleets() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root)?;
        let store = LedgerStore::open(project_root)?;

        assert!(should_run_startup_gc(&store)?);

        store.with_conn(|conn| {
            conn.execute(
                "INSERT INTO health (timestamp, status) VALUES (datetime('now', '-25 hours'), 'gc_completed')",
                [],
            )?;
            Ok(())
        })?;
        assert!(should_run_startup_gc(&store)?);

        store.with_conn(|conn| {
            conn.execute(
                "INSERT INTO fleets (fleet_id, task_description, agent_composition, source_project_root, state)
                 VALUES ('fleet-1', 'task', 'codex,gemini', '/tmp', 'running')",
                [],
            )?;
            Ok(())
        })?;
        assert!(!should_run_startup_gc(&store)?);
        Ok(())
    }
}
