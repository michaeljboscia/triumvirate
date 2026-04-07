use std::{fs, path::PathBuf, time::Duration};

use rusqlite::Connection;

use crate::LedgerStore;

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

    Ok(LedgerStore {
        project_root,
        conn: std::sync::Mutex::new(conn),
    })
}
