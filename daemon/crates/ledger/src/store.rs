use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::Duration,
};

use rusqlite::Connection;

use crate::LedgerStore;
use crate::init::ensure_triumvirate_gitignore;

static INGEST_WRITES_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

const CREATE_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    timestamp TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    compression_state TEXT NOT NULL DEFAULT 'pending',
    compression_heartbeat TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(session_id, event_type, sequence)
);

CREATE TABLE IF NOT EXISTS summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id INTEGER REFERENCES events(id),
    title TEXT NOT NULL,
    narrative TEXT NOT NULL,
    facts_json TEXT,
    concepts_json TEXT,
    affected_files_json TEXT,
    summary_type TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE VIRTUAL TABLE IF NOT EXISTS summaries_fts
USING fts5(title, narrative, facts_json, content='summaries', content_rowid='id');

CREATE TRIGGER IF NOT EXISTS summaries_ai AFTER INSERT ON summaries BEGIN
    INSERT INTO summaries_fts(rowid, title, narrative, facts_json)
    VALUES (new.id, new.title, new.narrative, new.facts_json);
END;

CREATE TRIGGER IF NOT EXISTS summaries_ad AFTER DELETE ON summaries BEGIN
    INSERT INTO summaries_fts(summaries_fts, rowid, title, narrative, facts_json)
    VALUES('delete', old.id, old.title, old.narrative, old.facts_json);
END;

CREATE TRIGGER IF NOT EXISTS summaries_au AFTER UPDATE ON summaries BEGIN
    INSERT INTO summaries_fts(summaries_fts, rowid, title, narrative, facts_json)
    VALUES('delete', old.id, old.title, old.narrative, old.facts_json);
    INSERT INTO summaries_fts(rowid, title, narrative, facts_json)
    VALUES (new.id, new.title, new.narrative, new.facts_json);
END;

CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    branch TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    event_count INTEGER NOT NULL DEFAULT 0,
    summary_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS health (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    last_event_id INTEGER,
    db_size_bytes INTEGER,
    spool_size_bytes INTEGER,
    queue_depth INTEGER,
    status TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS lessons (
    lesson_id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    source_session_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_validated_at TEXT NOT NULL DEFAULT (datetime('now')),
    initial_confidence REAL NOT NULL DEFAULT 0.8,
    tags_json TEXT,
    req_ids_json TEXT
);

CREATE VIRTUAL TABLE IF NOT EXISTS lessons_fts
USING fts5(title, body, tags_json, content='lessons', content_rowid='lesson_id');

CREATE TRIGGER IF NOT EXISTS lessons_ai AFTER INSERT ON lessons BEGIN
    INSERT INTO lessons_fts(rowid, title, body, tags_json)
    VALUES (new.lesson_id, new.title, new.body, new.tags_json);
END;

CREATE TRIGGER IF NOT EXISTS lessons_ad AFTER DELETE ON lessons BEGIN
    INSERT INTO lessons_fts(lessons_fts, rowid, title, body, tags_json)
    VALUES('delete', old.lesson_id, old.title, old.body, old.tags_json);
END;

CREATE TRIGGER IF NOT EXISTS lessons_au AFTER UPDATE ON lessons BEGIN
    INSERT INTO lessons_fts(lessons_fts, rowid, title, body, tags_json)
    VALUES('delete', old.lesson_id, old.title, old.body, old.tags_json);
    INSERT INTO lessons_fts(rowid, title, body, tags_json)
    VALUES (new.lesson_id, new.title, new.body, new.tags_json);
END;

CREATE TABLE IF NOT EXISTS fleets (
    fleet_id TEXT PRIMARY KEY,
    task_description TEXT NOT NULL,
    agent_composition TEXT NOT NULL,
    source_project_root TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'spawning',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT,
    failure_reason TEXT
);

CREATE TABLE IF NOT EXISTS tasks (
    task_id TEXT PRIMARY KEY,
    fleet_id TEXT NOT NULL REFERENCES fleets(fleet_id),
    title TEXT NOT NULL,
    description TEXT,
    assigned_agent TEXT,
    state TEXT NOT NULL DEFAULT 'pending',
    depends_on TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_tasks_fleet ON tasks(fleet_id);
CREATE INDEX IF NOT EXISTS idx_tasks_state ON tasks(state);

CREATE TABLE IF NOT EXISTS reviews (
    review_id TEXT PRIMARY KEY,
    fleet_id TEXT,
    author_agent TEXT NOT NULL,
    reviewer_agent TEXT,
    artifact TEXT NOT NULL,
    review_type TEXT NOT NULL,
    verdict TEXT,
    comments TEXT,
    requested_at TEXT NOT NULL DEFAULT (datetime('now')),
    reviewed_at TEXT,
    state TEXT NOT NULL DEFAULT 'pending'
);

CREATE INDEX IF NOT EXISTS idx_reviews_state ON reviews(state);
"#;

pub(crate) fn open(project_root: PathBuf) -> anyhow::Result<LedgerStore> {
    if !project_root.is_absolute() {
        anyhow::bail!("project_root must be an absolute path");
    }

    let data_dir = project_root.join(".triumvirate");
    fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join("ledger.db");
    let conn = Connection::open(db_path)?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(CREATE_SCHEMA_SQL)?;
    ensure_triumvirate_gitignore(&project_root)?;

    Ok(LedgerStore {
        project_root,
        conn: std::sync::Mutex::new(conn),
    })
}

pub(crate) fn with_ingest_priority<T, F>(f: F) -> anyhow::Result<T>
where
    F: FnOnce() -> anyhow::Result<T>,
{
    INGEST_WRITES_IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    let out = f();
    INGEST_WRITES_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    out
}

pub(crate) fn with_task_state_priority<T, F>(f: F) -> anyhow::Result<T>
where
    F: FnOnce() -> anyhow::Result<T>,
{
    if INGEST_WRITES_IN_FLIGHT.load(Ordering::SeqCst) > 0 {
        thread::sleep(Duration::from_millis(2));
    }
    f()
}

pub(crate) fn queue_lag_seconds_conn(conn: &Connection) -> anyhow::Result<f64> {
    let lag = conn.query_row(
        "SELECT COALESCE((julianday('now') - julianday(MIN(created_at))) * 86400.0, 0.0)
         FROM events
         WHERE compression_state = 'pending'",
        [],
        |row| row.get::<_, f64>(0),
    )?;
    Ok(if lag.is_sign_negative() { 0.0 } else { lag })
}
