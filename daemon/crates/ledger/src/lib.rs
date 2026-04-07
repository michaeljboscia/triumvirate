mod compression;
mod gc;
mod health;
mod init;
mod ingest;
mod lessons;
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
    DrainResult, GcResult, HealthStatus, Lesson, ManualRecord, NewLesson, RawEvent,
    SessionDetail, Summary,
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

    pub fn get_session(&self, session_id: &str) -> anyhow::Result<SessionDetail> {
        query::get_session_detail(self, session_id)
    }

    pub fn record(&self, record: ManualRecord) -> anyhow::Result<()> {
        ingest::record_manual(self, record)
    }

    pub fn health(&self) -> anyhow::Result<HealthStatus> {
        health::health(self)
    }

    pub fn add_lesson(&self, lesson: NewLesson) -> anyhow::Result<i64> {
        lessons::add_lesson(self, lesson)
    }

    pub fn query_lessons(
        &self,
        query: &str,
        min_confidence: f64,
    ) -> anyhow::Result<Vec<Lesson>> {
        lessons::query_lessons(self, query, min_confidence)
    }

    pub fn validate_lesson(&self, lesson_id: i64) -> anyhow::Result<()> {
        lessons::validate_lesson(self, lesson_id)
    }

    pub fn list_lessons(
        &self,
        tags: Option<&[String]>,
        stale_days: Option<f64>,
    ) -> anyhow::Result<Vec<Lesson>> {
        lessons::list_lessons(self, tags, stale_days)
    }

    pub fn gc(&self) -> anyhow::Result<GcResult> {
        gc::gc(self)
    }

    pub fn has_active_fleets(&self) -> anyhow::Result<bool> {
        gc::has_active_fleets(self)
    }

    pub fn last_gc_timestamp(&self) -> anyhow::Result<Option<String>> {
        gc::last_gc_timestamp(self)
    }

    pub fn should_run_startup_gc(&self) -> anyhow::Result<bool> {
        gc::should_run_startup_gc(self)
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

    pub fn queue_lag_seconds(&self) -> anyhow::Result<f64> {
        self.with_conn(store::queue_lag_seconds_conn)
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

    use shared_types::{ManualRecord, RawEvent};

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

        for expected in [
            "events",
            "summaries",
            "sessions",
            "health",
            "lessons",
            "fleets",
            "tasks",
            "reviews",
        ] {
            assert!(tables.contains(expected), "missing table: {expected}");
        }
    }

    #[test]
    fn fleet_tables_have_expected_columns() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project root");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        let columns = |table: &str| -> anyhow::Result<Vec<String>> {
            store.with_conn(|conn| {
                let sql = format!("PRAGMA table_info({table})");
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row?);
                }
                Ok(out)
            })
        };

        let fleets_cols = columns("fleets").expect("fleets columns");
        for col in [
            "fleet_id",
            "task_description",
            "agent_composition",
            "source_project_root",
            "state",
            "created_at",
            "completed_at",
            "failure_reason",
        ] {
            assert!(fleets_cols.iter().any(|c| c == col), "missing fleets.{col}");
        }

        let tasks_cols = columns("tasks").expect("tasks columns");
        for col in [
            "task_id",
            "fleet_id",
            "title",
            "description",
            "assigned_agent",
            "state",
            "depends_on",
            "created_at",
            "completed_at",
        ] {
            assert!(tasks_cols.iter().any(|c| c == col), "missing tasks.{col}");
        }

        let reviews_cols = columns("reviews").expect("reviews columns");
        for col in [
            "review_id",
            "fleet_id",
            "author_agent",
            "reviewer_agent",
            "artifact",
            "review_type",
            "verdict",
            "comments",
            "requested_at",
            "reviewed_at",
            "state",
        ] {
            assert!(reviews_cols.iter().any(|c| c == col), "missing reviews.{col}");
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
    fn ingest_succeeds_when_compression_path_is_broken() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool dir");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        store
            .with_conn(|conn| {
                conn.execute_batch(
                    "CREATE TRIGGER fail_compression_running
                     BEFORE UPDATE OF compression_state ON events
                     WHEN NEW.compression_state = 'running'
                     BEGIN
                         SELECT RAISE(FAIL, 'compression broken');
                     END;",
                )?;
                Ok(())
            })
            .expect("install failing compression trigger");

        store
            .ingest_event(RawEvent {
                session_id: "session-broken-compression".to_string(),
                event_type: "PostToolUse".to_string(),
                sequence: 1,
                timestamp: "2026-04-07T00:00:00Z".to_string(),
                payload_json: "{\"tool\":\"Edit\"}".to_string(),
            })
            .expect("ingest should not depend on compression");

        let count = store
            .with_conn(|conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM events WHERE session_id = ?1",
                    rusqlite::params!["session-broken-compression"],
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
    fn health_degrades_when_spool_exceeds_100mb() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        let spool_dir = project_root.join(".triumvirate").join("spool");
        fs::create_dir_all(&spool_dir).expect("create spool dir");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        let oversized = vec![0u8; 100_000_001];
        fs::write(spool_dir.join("oversized.ndjson"), oversized).expect("write oversized spool file");

        let status = store.health().expect("health status");
        assert!(status.spool_size_bytes > 100_000_000);
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
        crate::compression::process_pending_events(&store).expect("run compression");

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
    fn compression_auto_creates_lessons_for_selected_summary_types() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool dir");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        store
            .ingest_event(RawEvent {
                session_id: "lesson-session".to_string(),
                event_type: "PostToolUse".to_string(),
                sequence: 1,
                timestamp: "2030-01-01T00:00:00Z".to_string(),
                payload_json: "{\"tool\":\"Edit\",\"summary_type\":\"bug_fix\"}".to_string(),
            })
            .expect("ingest bug_fix event");

        store
            .ingest_event(RawEvent {
                session_id: "lesson-session".to_string(),
                event_type: "PostToolUse".to_string(),
                sequence: 2,
                timestamp: "2030-01-01T00:00:01Z".to_string(),
                payload_json: "{\"tool\":\"Read\",\"summary_type\":\"extractive\"}".to_string(),
            })
            .expect("ingest extractive event");
        crate::compression::process_pending_events(&store).expect("run compression");

        let lessons = store
            .with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT title, initial_confidence FROM lessons ORDER BY lesson_id ASC",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
                })?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row?);
                }
                Ok(out)
            })
            .expect("read lessons");

        assert_eq!(lessons.len(), 1);
        assert!(lessons[0].0.contains("Event"));
        assert!((lessons[0].1 - 0.6).abs() < f64::EPSILON);
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

    #[test]
    fn ingest_is_prioritized_under_concurrent_task_updates() {
        use std::{sync::Arc, thread, time::Instant};

        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool dir");
        let store = Arc::new(LedgerStore::open(project_root).expect("open ledger store"));

        store
            .with_conn(|conn| {
                for seq in 0..50_i64 {
                    conn.execute(
                        "INSERT INTO events (session_id, event_type, sequence, timestamp, payload_json, compression_state)
                         VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
                        rusqlite::params!["task-session", "PostToolUse", seq, "2026-04-07T00:00:00Z", "{}"],
                    )?;
                }
                Ok(())
            })
            .expect("seed task rows");

        let start = Instant::now();
        let mut handles = Vec::new();
        for idx in 0..50_i64 {
            let s = Arc::clone(&store);
            handles.push(thread::spawn(move || {
                s.with_conn(|conn| {
                    crate::store::with_task_state_priority(|| {
                        conn.execute(
                            "UPDATE events SET compression_state = 'running' WHERE sequence = ?1 AND session_id = 'task-session'",
                            [idx],
                        )?;
                        Ok(())
                    })
                })
                .expect("task update");
            }));
        }
        for seq in 0..50_i64 {
            let s = Arc::clone(&store);
            handles.push(thread::spawn(move || {
                s.ingest_event(RawEvent {
                    session_id: "ingest-session".to_string(),
                    event_type: "PostToolUse".to_string(),
                    sequence: seq,
                    timestamp: "2030-01-01T00:00:00Z".to_string(),
                    payload_json: "{\"tool\":\"Edit\"}".to_string(),
                })
                .expect("ingest event");
            }));
        }
        for h in handles {
            h.join().expect("join thread");
        }
        let elapsed = start.elapsed().as_secs_f64();
        assert!(elapsed < 5.0, "ingestion burst took too long: {elapsed:.3}s");

        let ingested = store
            .with_conn(|conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM events WHERE session_id = 'ingest-session'",
                    [],
                    |row| row.get(0),
                )?;
                Ok(count)
            })
            .expect("count ingested");
        assert_eq!(ingested, 50);

        let lag = store.queue_lag_seconds().expect("queue lag");
        assert!(lag >= 0.0);
    }

    #[test]
    fn get_session_reconstructs_events_and_summaries() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool dir");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        for seq in 1..=10_i64 {
            store
                .ingest_event(RawEvent {
                    session_id: "abc".to_string(),
                    event_type: "PostToolUse".to_string(),
                    sequence: seq,
                    timestamp: "2030-01-01T00:00:00Z".to_string(),
                    payload_json: "{\"tool\":\"Edit\"}".to_string(),
                })
                .expect("ingest event");
        }

        store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO summaries (event_id, title, narrative, facts_json, summary_type) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![1_i64, "sum1", "n1", "[]", "extractive"],
                )?;
                conn.execute(
                    "INSERT INTO summaries (event_id, title, narrative, facts_json, summary_type) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![2_i64, "sum2", "n2", "[]", "extractive"],
                )?;
                Ok(())
            })
            .expect("insert summaries");

        let detail = store.get_session("abc").expect("session detail");
        assert_eq!(detail.events.len(), 10);
        assert!(detail.summaries.len() >= 2);
        let titles = detail
            .summaries
            .iter()
            .map(|s| s.title.as_str())
            .collect::<Vec<_>>();
        assert!(titles.contains(&"sum1"));
        assert!(titles.contains(&"sum2"));
        assert_eq!(detail.session_id, "abc");

        let err = store.get_session("nonexistent");
        assert!(err.is_err());
    }

    #[test]
    fn record_inserts_manual_summary_and_is_searchable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool dir");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        store
            .record(ManualRecord {
                session_id: None,
                title: "Architecture decision: use WAL".to_string(),
                narrative: "Prefer WAL journaling for concurrent reads".to_string(),
                facts_json: Some("[\"wal\"]".to_string()),
                concepts_json: None,
                affected_files_json: None,
                summary_type: "architecture_decision".to_string(),
            })
            .expect("record manual summary");

        let results = store.query("WAL", 10).expect("query manual record");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Architecture decision: use WAL");
        assert_eq!(results[0].summary_type, "architecture_decision");
    }

    #[test]
    fn lessons_decay_and_validate_behave_as_expected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool dir");
        let store = LedgerStore::open(project_root).expect("open ledger store");

        let lesson_id = store
            .add_lesson(shared_types::NewLesson {
                title: "WAL lesson".to_string(),
                body: "Use WAL for concurrent readers".to_string(),
                source_session_id: Some("sess-1".to_string()),
                initial_confidence: 0.8,
                tags_json: Some("[\"sqlite\",\"wal\"]".to_string()),
                req_ids_json: Some("[\"REQ-019\"]".to_string()),
            })
            .expect("insert lesson");

        let immediate = store.query_lessons("WAL", 0.0).expect("query immediate");
        assert_eq!(immediate.len(), 1);
        assert!((immediate[0].initial_confidence - 0.8).abs() < 0.05);

        store
            .with_conn(|conn| {
                conn.execute(
                    "UPDATE lessons SET last_validated_at = datetime('now', '-80 days') WHERE lesson_id = ?1",
                    [lesson_id],
                )?;
                Ok(())
            })
            .expect("backdate lesson");

        let decayed = store.query_lessons("WAL", 0.0).expect("query decayed");
        assert_eq!(decayed.len(), 1);
        assert!((decayed[0].initial_confidence - 0.36).abs() < 0.08);

        let filtered = store.query_lessons("WAL", 0.4).expect("query min confidence");
        assert!(filtered.is_empty());

        let stale = store
            .list_lessons(None, Some(30.0))
            .expect("list stale lessons");
        assert_eq!(stale.len(), 1);

        store.validate_lesson(lesson_id).expect("validate lesson");
        let after_validate = store
            .query_lessons("WAL", 0.7)
            .expect("query validated lesson");
        assert_eq!(after_validate.len(), 1);
        assert!((after_validate[0].initial_confidence - 0.8).abs() < 0.08);

        let tagged = store
            .list_lessons(Some(&["sqlite".to_string()]), None)
            .expect("list tagged lessons");
        assert_eq!(tagged.len(), 1);
    }

}
