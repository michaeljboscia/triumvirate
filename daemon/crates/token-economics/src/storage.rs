use std::{
    fs,
    path::Path,
    sync::MutexGuard,
    time::Duration,
};

use anyhow::Context;
use rusqlite::{Connection, params};
use tracing::debug;

use crate::{TokenDb, TokenRecord, TokenSummaryRow};

const CREATE_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS token_records (
    id INTEGER PRIMARY KEY,
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    model TEXT,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cached_tokens INTEGER DEFAULT 0,
    thinking_tokens INTEGER DEFAULT 0,
    total_tokens INTEGER NOT NULL,
    cost_usd REAL,
    latency_ms INTEGER,
    tool_calls INTEGER,
    lines_added INTEGER,
    lines_removed INTEGER,
    rate_limit_pct REAL,
    context_window INTEGER,
    build_id TEXT,
    task_id TEXT,
    wave INTEGER
);

CREATE TABLE IF NOT EXISTS scan_state (
    file_path TEXT PRIMARY KEY,
    last_mtime INTEGER NOT NULL,
    last_offset INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS price_table (
    id INTEGER PRIMARY KEY,
    model TEXT NOT NULL,
    input_per_mtok REAL NOT NULL,
    output_per_mtok REAL NOT NULL,
    cached_per_mtok REAL DEFAULT 0,
    effective_date TEXT NOT NULL,
    end_date TEXT
);

CREATE INDEX IF NOT EXISTS idx_price_model_date ON price_table (model, effective_date);
"#;

pub fn open(path: &Path) -> anyhow::Result<TokenDb> {
    if !path.is_absolute() {
        anyhow::bail!("database path must be absolute");
    }

    let parent = path
        .parent()
        .context("database path must include a parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create token DB directory: {}", parent.display()))?;

    let conn = Connection::open(path)
        .with_context(|| format!("failed to open token DB at {}", path.display()))?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(CREATE_SCHEMA_SQL)?;

    debug!(db_path = %path.display(), "token economics DB initialized");

    Ok(TokenDb {
        conn: std::sync::Mutex::new(conn),
    })
}

pub fn insert_record(db: &TokenDb, record: &TokenRecord) -> anyhow::Result<()> {
    with_conn(db, |conn| {
        conn.execute(
            r#"
            INSERT INTO token_records (
                agent,
                session_id,
                timestamp,
                model,
                input_tokens,
                output_tokens,
                cached_tokens,
                thinking_tokens,
                total_tokens,
                cost_usd,
                latency_ms,
                tool_calls,
                lines_added,
                lines_removed,
                rate_limit_pct,
                context_window,
                build_id,
                task_id,
                wave
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
            )
            "#,
            params![
                record.agent,
                record.session_id,
                record.timestamp,
                record.model,
                record.input_tokens,
                record.output_tokens,
                record.cached_tokens,
                record.thinking_tokens,
                record.total_tokens,
                record.cost_usd,
                record.latency_ms,
                record.tool_calls,
                record.lines_added,
                record.lines_removed,
                record.rate_limit_pct,
                record.context_window,
                record.build_id,
                record.task_id,
                record.wave,
            ],
        )?;
        Ok(())
    })
}

pub fn query_summary(
    db: &TokenDb,
    since: Option<&str>,
    until: Option<&str>,
    agent: Option<&str>,
) -> anyhow::Result<Vec<TokenSummaryRow>> {
    with_conn(db, |conn| {
        let mut sql = String::from(
            r#"
            SELECT
                id,
                agent,
                session_id,
                timestamp,
                model,
                input_tokens,
                output_tokens,
                cached_tokens,
                thinking_tokens,
                total_tokens,
                cost_usd,
                latency_ms,
                tool_calls,
                lines_added,
                lines_removed,
                rate_limit_pct,
                context_window,
                build_id,
                task_id,
                wave
            FROM token_records
            WHERE 1 = 1
            "#,
        );

        let mut bind_values: Vec<String> = Vec::new();
        if let Some(since) = since {
            sql.push_str(" AND timestamp >= ? ");
            bind_values.push(since.to_owned());
        }
        if let Some(until) = until {
            sql.push_str(" AND timestamp <= ? ");
            bind_values.push(until.to_owned());
        }
        if let Some(agent) = agent {
            sql.push_str(" AND agent = ? ");
            bind_values.push(agent.to_owned());
        }
        sql.push_str(" ORDER BY timestamp ASC, id ASC ");

        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(bind_values.iter()))?;

        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(TokenSummaryRow {
                id: row.get(0)?,
                agent: row.get(1)?,
                session_id: row.get(2)?,
                timestamp: row.get(3)?,
                model: row.get(4)?,
                input_tokens: row.get(5)?,
                output_tokens: row.get(6)?,
                cached_tokens: row.get(7)?,
                thinking_tokens: row.get(8)?,
                total_tokens: row.get(9)?,
                cost_usd: row.get(10)?,
                latency_ms: row.get(11)?,
                tool_calls: row.get(12)?,
                lines_added: row.get(13)?,
                lines_removed: row.get(14)?,
                rate_limit_pct: row.get(15)?,
                context_window: row.get(16)?,
                build_id: row.get(17)?,
                task_id: row.get(18)?,
                wave: row.get(19)?,
            });
        }

        Ok(out)
    })
}

fn with_conn<T, F>(db: &TokenDb, f: F) -> anyhow::Result<T>
where
    F: FnOnce(&Connection) -> anyhow::Result<T>,
{
    let guard: MutexGuard<'_, Connection> = db
        .conn
        .lock()
        .map_err(|_| anyhow::anyhow!("token DB connection mutex poisoned"))?;
    f(&guard)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, fs};

    use super::{insert_record, open, query_summary};
    use crate::TokenRecord;

    #[test]
    fn open_sets_wal_and_creates_required_tables() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp.path().join("token-economics.db");

        let db = open(&db_path).expect("open token DB");

        let journal_mode = super::with_conn(&db, |conn| {
            let mode: String = conn.query_row("PRAGMA journal_mode;", [], |row| row.get(0))?;
            Ok(mode)
        })
        .expect("query journal mode");
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let tables = super::with_conn(&db, |conn| {
            let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut names = HashSet::new();
            for row in rows {
                names.insert(row?);
            }
            Ok(names)
        })
        .expect("read sqlite_master tables");

        for expected in ["token_records", "scan_state", "price_table"] {
            assert!(tables.contains(expected), "missing table: {expected}");
        }
    }

    #[test]
    fn round_trip_insert_and_query() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project root");
        let db_path = project_root.join(".triumvirate").join("token-economics.db");

        let db = open(&db_path).expect("open token DB");
        let record = TokenRecord {
            agent: "codex".to_string(),
            session_id: "session-123".to_string(),
            timestamp: "2026-04-10T12:34:56Z".to_string(),
            model: Some("gpt-5.3-codex".to_string()),
            input_tokens: 1200,
            output_tokens: 340,
            cached_tokens: 100,
            thinking_tokens: 50,
            total_tokens: 1590,
            cost_usd: Some(0.0525),
            latency_ms: Some(2100),
            tool_calls: Some(3),
            lines_added: Some(42),
            lines_removed: Some(7),
            rate_limit_pct: Some(13.2),
            context_window: Some(200000),
            build_id: Some("abe-v3-main".to_string()),
            task_id: Some("T-102".to_string()),
            wave: Some(0),
        };

        insert_record(&db, &record).expect("insert token record");
        let rows = query_summary(
            &db,
            Some("2026-04-10T00:00:00Z"),
            Some("2026-04-11T00:00:00Z"),
            Some("codex"),
        )
        .expect("query token summary");

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.agent, record.agent);
        assert_eq!(row.session_id, record.session_id);
        assert_eq!(row.timestamp, record.timestamp);
        assert_eq!(row.model, record.model);
        assert_eq!(row.input_tokens, record.input_tokens);
        assert_eq!(row.output_tokens, record.output_tokens);
        assert_eq!(row.cached_tokens, record.cached_tokens);
        assert_eq!(row.thinking_tokens, record.thinking_tokens);
        assert_eq!(row.total_tokens, record.total_tokens);
        assert_eq!(row.cost_usd, record.cost_usd);
        assert_eq!(row.latency_ms, record.latency_ms);
        assert_eq!(row.tool_calls, record.tool_calls);
        assert_eq!(row.lines_added, record.lines_added);
        assert_eq!(row.lines_removed, record.lines_removed);
        assert_eq!(row.rate_limit_pct, record.rate_limit_pct);
        assert_eq!(row.context_window, record.context_window);
        assert_eq!(row.build_id, record.build_id);
        assert_eq!(row.task_id, record.task_id);
        assert_eq!(row.wave, record.wave);
    }
}
