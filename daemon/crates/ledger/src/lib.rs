mod ingest;
mod store;

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use rusqlite::Connection;
use shared_types::{
    DrainResult, GcResult, HealthStatus, Lesson, ManualRecord, NewLesson, RawEvent, SessionDetail,
    Summary,
};

#[derive(Debug)]
pub struct LedgerStore {
    project_root: PathBuf,
    conn: Mutex<Connection>,
}

impl LedgerStore {
    pub fn open(project_root: PathBuf) -> anyhow::Result<Self> {
        store::open(project_root)
    }

    pub fn ingest_event(&self, event: RawEvent) -> anyhow::Result<()> {
        ingest::ingest_event(self, event)
    }

    pub fn drain_spool(&self, _spool_dir: &Path) -> anyhow::Result<DrainResult> {
        anyhow::bail!("not implemented")
    }

    pub fn query(&self, _query: &str, _limit: usize) -> anyhow::Result<Vec<Summary>> {
        anyhow::bail!("not implemented")
    }

    pub fn get_session(&self, _session_id: &str) -> anyhow::Result<SessionDetail> {
        anyhow::bail!("not implemented")
    }

    pub fn record(&self, _record: ManualRecord) -> anyhow::Result<()> {
        anyhow::bail!("not implemented")
    }

    pub fn health(&self) -> anyhow::Result<HealthStatus> {
        anyhow::bail!("not implemented")
    }

    pub fn add_lesson(&self, _lesson: NewLesson) -> anyhow::Result<i64> {
        anyhow::bail!("not implemented")
    }

    pub fn query_lessons(
        &self,
        _query: &str,
        _min_confidence: f64,
    ) -> anyhow::Result<Vec<Lesson>> {
        anyhow::bail!("not implemented")
    }

    pub fn validate_lesson(&self, _lesson_id: i64) -> anyhow::Result<()> {
        anyhow::bail!("not implemented")
    }

    pub fn gc(&self) -> anyhow::Result<GcResult> {
        anyhow::bail!("not implemented")
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub(crate) fn with_conn<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&Connection) -> anyhow::Result<T>,
    {
        let guard = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("ledger connection mutex poisoned"))?;
        f(&guard)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, fs, path::PathBuf};

    use shared_types::RawEvent;

    use super::LedgerStore;

    #[test]
    fn open_rejects_relative_paths() {
        let err = LedgerStore::open(PathBuf::from("relative/path"))
            .expect_err("relative paths must be rejected");
        assert!(err.to_string().contains("absolute path"));
    }

    #[test]
    fn open_sets_wal_and_creates_tables() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project root");

        let store = LedgerStore::open(project_root).expect("open ledger store");
        let journal_mode = store
            .with_conn(|conn| {
                let mode: String = conn.query_row("PRAGMA journal_mode;", [], |row| row.get(0))?;
                Ok(mode)
            })
            .expect("query pragma journal_mode");
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let tables = store
            .with_conn(|conn| {
                let mut stmt =
                    conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                let mut names = HashSet::new();
                for row in rows {
                    names.insert(row?);
                }
                Ok(names)
            })
            .expect("read sqlite_master tables");

        for expected in ["events", "summaries", "sessions", "health", "lessons"] {
            assert!(tables.contains(expected), "missing table: {expected}");
        }
    }

    #[test]
    fn summaries_fts_match_returns_expected_rows() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project root");

        let store = LedgerStore::open(project_root).expect("open ledger store");
        store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO summaries (title, narrative, facts_json, summary_type) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![
                        "authentication middleware bug",
                        "fixed auth ordering",
                        "[\"authentication\",\"middleware\"]",
                        "bug_fix"
                    ],
                )?;
                Ok(())
            })
            .expect("insert summary");

        let match_count = store
            .with_conn(|conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM summaries_fts WHERE summaries_fts MATCH 'authentication'",
                    [],
                    |row| row.get(0),
                )?;
                Ok(count)
            })
            .expect("fts match query");
        assert_eq!(match_count, 1);

        let miss_count = store
            .with_conn(|conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM summaries_fts WHERE summaries_fts MATCH 'nonexistent'",
                    [],
                    |row| row.get(0),
                )?;
                Ok(count)
            })
            .expect("fts miss query");
        assert_eq!(miss_count, 0);
    }

    #[test]
    fn ingest_event_is_idempotent_on_duplicate_keys() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project root");

        let store = LedgerStore::open(project_root).expect("open ledger store");
        let event = RawEvent {
            session_id: "session-a".to_string(),
            event_type: "PostToolUse".to_string(),
            sequence: 7,
            timestamp: "2026-04-07T00:00:00Z".to_string(),
            payload_json: "{\"tool\":\"Edit\"}".to_string(),
        };
        store.ingest_event(event.clone()).expect("first ingest");
        store.ingest_event(event).expect("duplicate ingest");

        let count = store
            .with_conn(|conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM events WHERE session_id = ?1",
                    rusqlite::params!["session-a"],
                    |row| row.get(0),
                )?;
                Ok(count)
            })
            .expect("count events");
        assert_eq!(count, 1);
    }
}
