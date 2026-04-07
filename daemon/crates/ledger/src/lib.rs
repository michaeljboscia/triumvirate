mod compression;
mod health;
mod init;
mod ingest;
mod pool;
mod query;
mod spool;
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
        spool::drain_spool(self, _spool_dir)
    }

    pub fn query(&self, query: &str, limit: usize) -> anyhow::Result<Vec<Summary>> {
        query::query_summaries(self, query, limit)
    }

    pub fn get_session(&self, _session_id: &str) -> anyhow::Result<SessionDetail> {
        anyhow::bail!("not implemented")
    }

    pub fn record(&self, _record: ManualRecord) -> anyhow::Result<()> {
        anyhow::bail!("not implemented")
    }

    pub fn health(&self) -> anyhow::Result<HealthStatus> {
        health::health(self)
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

    pub fn journal_mode(&self) -> anyhow::Result<String> {
        self.with_conn(|conn| {
            let mode: String = conn.query_row("PRAGMA journal_mode;", [], |row| row.get(0))?;
            Ok(mode)
        })
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

    #[test]
    fn drain_spool_ingests_and_deletes_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        let spool_dir = project_root.join(".triumvirate").join("spool");
        fs::create_dir_all(&spool_dir).expect("create spool dir");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        for idx in 1..=3 {
            let raw = serde_json::json!({
                "session_id": "session-drain",
                "event_type": "PostToolUse",
                "sequence": idx,
                "timestamp": format!("2026-04-07T00:00:0{idx}Z"),
                "payload_json": "{\"ok\":true}"
            });
            let file = spool_dir.join(format!("event-{idx}.ndjson"));
            fs::write(file, serde_json::to_string(&raw).expect("serialize event"))
                .expect("write spool event");
        }

        let result = store.drain_spool(&spool_dir).expect("drain spool");
        assert_eq!(result.ingested_count, 3);
        assert_eq!(result.failed_count, 0);
        assert_eq!(result.skipped_count, 0);

        let count = store
            .with_conn(|conn| {
                let count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM events WHERE session_id = ?1", ["session-drain"], |row| row.get(0))?;
                Ok(count)
            })
            .expect("count drained events");
        assert_eq!(count, 3);

        let remaining = fs::read_dir(&spool_dir)
            .expect("read spool dir")
            .filter_map(Result::ok)
            .count();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn drain_spool_truncates_large_json_fields() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        let spool_dir = project_root.join(".triumvirate").join("spool");
        fs::create_dir_all(&spool_dir).expect("create spool dir");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        let large_payload = serde_json::json!({
            "tool_output": "x".repeat(100_000),
            "message": "ok"
        });
        let raw = serde_json::json!({
            "session_id": "session-large",
            "event_type": "PostToolUse",
            "sequence": 1,
            "timestamp": "2026-04-07T00:00:01Z",
            "payload_json": serde_json::to_string(&large_payload).expect("serialize payload")
        });
        fs::write(
            spool_dir.join("event-large.ndjson"),
            serde_json::to_string(&raw).expect("serialize event"),
        )
        .expect("write spool file");

        let result = store.drain_spool(&spool_dir).expect("drain spool");
        assert_eq!(result.ingested_count, 1);

        let stored_payload = store
            .with_conn(|conn| {
                let payload: String = conn.query_row(
                    "SELECT payload_json FROM events WHERE session_id = ?1",
                    ["session-large"],
                    |row| row.get(0),
                )?;
                Ok(payload)
            })
            .expect("read stored payload");
        assert!(stored_payload.contains("[...truncated]"));
        let parsed: serde_json::Value =
            serde_json::from_str(&stored_payload).expect("payload should remain valid json");
        assert_eq!(parsed["message"], "ok");
    }

    #[test]
    fn health_reports_recent_event_counts() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool dir");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        for seq in 1..=5 {
            store
                .ingest_event(RawEvent {
                    session_id: "health-session".to_string(),
                    event_type: "PostToolUse".to_string(),
                    sequence: seq,
                    timestamp: "2030-01-01T00:00:00Z".to_string(),
                    payload_json: "{}".to_string(),
                })
                .expect("ingest event");
        }

        let status = store.health().expect("health status");
        assert!(status.events_last_5min >= 5);
        assert!(status.db_size_bytes > 0);
    }

    #[test]
    fn health_degrades_when_active_session_has_no_recent_events() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool dir");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO sessions (session_id, project, branch, started_at, ended_at, event_count, summary_count)
                     VALUES (?1, ?2, NULL, ?3, NULL, 0, 0)",
                    rusqlite::params![
                        "active-session",
                        "/tmp/project",
                        "2026-04-07T00:00:00Z"
                    ],
                )?;
                Ok(())
            })
            .expect("insert active session");

        let status = store.health().expect("health status");
        assert_eq!(status.events_last_5min, 0);
        assert_eq!(status.status, "degraded");
    }

    #[test]
    fn open_adds_triumvirate_gitignore_entry_once() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project root");

        let status = std::process::Command::new("git")
            .arg("init")
            .arg(&project_root)
            .status()
            .expect("run git init");
        assert!(status.success(), "git init failed");

        LedgerStore::open(project_root.clone()).expect("first open");
        LedgerStore::open(project_root.clone()).expect("second open");

        let gitignore = fs::read_to_string(project_root.join(".gitignore"))
            .expect("read .gitignore");
        let occurrences = gitignore
            .lines()
            .filter(|line| line.trim() == ".triumvirate/")
            .count();
        assert_eq!(occurrences, 1);
    }

    #[test]
    fn compression_creates_extractive_summaries_and_marks_done() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool dir");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        let payloads = [
            "{\"tool\":\"Edit\",\"file\":\"src/lib.rs\"}",
            "{\"tool\":\"Bash\",\"cmd\":\"cargo test\"}",
            "{\"tool\":\"Read\",\"file\":\"README.md\"}",
            "{\"tool\":\"Edit\",\"file\":\"src/main.rs\"}",
            "{\"tool\":\"Bash\",\"cmd\":\"cargo check\"}",
        ];
        for (idx, payload) in payloads.iter().enumerate() {
            store
                .ingest_event(RawEvent {
                    session_id: "compression-session".to_string(),
                    event_type: "PostToolUse".to_string(),
                    sequence: (idx + 1) as i64,
                    timestamp: "2030-01-01T00:00:00Z".to_string(),
                    payload_json: (*payload).to_string(),
                })
                .expect("ingest event");
        }

        let summary_count = store
            .with_conn(|conn| {
                let count: i64 = conn.query_row("SELECT COUNT(*) FROM summaries", [], |row| row.get(0))?;
                Ok(count)
            })
            .expect("count summaries");
        assert!(summary_count >= 1);

        let narratives = store
            .with_conn(|conn| {
                let mut stmt = conn.prepare("SELECT narrative FROM summaries ORDER BY id ASC")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row?);
                }
                Ok(out)
            })
            .expect("read narratives");
        let joined = narratives.join(" ");
        assert!(joined.contains("Edit") || joined.contains("Bash") || joined.contains("Read"));

        let done_count = store
            .with_conn(|conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM events WHERE compression_state = 'done'",
                    [],
                    |row| row.get(0),
                )?;
                Ok(count)
            })
            .expect("count done events");
        assert_eq!(done_count, payloads.len() as i64);
    }

    #[test]
    fn compression_ttl_reclaim_resets_only_stale_running_jobs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool dir");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO events (session_id, event_type, sequence, timestamp, payload_json, compression_state, compression_heartbeat)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'running', datetime('now', '-2 minutes'))",
                    rusqlite::params!["ttl-session", "PostToolUse", 1, "2026-04-07T00:00:00Z", "{}"],
                )?;
                conn.execute(
                    "INSERT INTO events (session_id, event_type, sequence, timestamp, payload_json, compression_state, compression_heartbeat)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'running', datetime('now', '-10 seconds'))",
                    rusqlite::params!["ttl-session", "PostToolUse", 2, "2026-04-07T00:00:01Z", "{}"],
                )?;
                Ok(())
            })
            .expect("insert running jobs");

        let reclaimed = crate::compression::reclaim_stale_running(&store).expect("reclaim stale");
        assert_eq!(reclaimed, 1);

        let states = store
            .with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT sequence, compression_state FROM events
                     WHERE session_id = 'ttl-session'
                     ORDER BY sequence ASC",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row?);
                }
                Ok(out)
            })
            .expect("query states");
        assert_eq!(states[0].1, "pending");
        assert_eq!(states[1].1, "running");
    }

    #[test]
    fn pool_manager_enforces_idle_ttl_and_capacity() {
        crate::pool::reset_pool_state_for_tests();
        let t0 = 1_000_u64;

        let p1 = PathBuf::from("/tmp/pool-a");
        let p2 = PathBuf::from("/tmp/pool-b");
        let p3 = PathBuf::from("/tmp/pool-c");
        crate::pool::register_activity_at(&p1, t0);
        crate::pool::register_activity_at(&p2, t0);
        crate::pool::register_activity_at(&p3, t0);
        let stats = crate::pool::pool_stats();
        assert_eq!(stats.active_pools, 3);
        assert_eq!(stats.queued_projects, 0);

        crate::pool::reap_idle_at(t0 + 16 * 60 * 1000);
        let stats_after_idle = crate::pool::pool_stats();
        assert_eq!(stats_after_idle.active_pools, 0);

        crate::pool::reset_pool_state_for_tests();
        for idx in 0..11 {
            let path = PathBuf::from(format!("/tmp/pool-{idx}"));
            crate::pool::register_activity_at(&path, t0);
        }
        let stats_with_cap = crate::pool::pool_stats();
        assert_eq!(stats_with_cap.active_pools, 10);
        assert_eq!(stats_with_cap.queued_projects, 1);
    }

    #[test]
    fn query_returns_ranked_fts_summary_results() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool dir");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO summaries (title, narrative, facts_json, summary_type)
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![
                        "authentication middleware bug",
                        "fixed auth ordering in middleware",
                        "[\"authentication\"]",
                        "bug_fix"
                    ],
                )?;
                conn.execute(
                    "INSERT INTO summaries (title, narrative, facts_json, summary_type)
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![
                        "cache refresh issue",
                        "resolved stale cache reads",
                        "[\"cache\"]",
                        "bug_fix"
                    ],
                )?;
                Ok(())
            })
            .expect("seed summaries");

        let hits = store.query("authentication", 10).expect("fts query");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].title.contains("authentication"));
    }
}
