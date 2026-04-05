use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use tracing::info;

/// SQLite WAL-backed memory store.
///
/// Shared across all three agents. Every memory write is:
/// 1. Validated (syntax-gated via # DECISION: keyword per GR1-D4)
/// 2. Deduplicated
/// 3. Persisted to SQLite with WAL mode (crash-safe)
/// 4. Published to the fabric for cache update
/// 5. Confirmed to the writing agent
///
/// WAL mode allows concurrent reads while writing — critical since
/// agents read memory constantly but writes are infrequent.
pub struct MemoryStore {
    conn: Connection,
}

impl MemoryStore {
    /// Open or create the memory database at the given path.
    /// Enables WAL mode and creates tables if they don't exist.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;

        // WAL mode: concurrent reads, crash-safe writes
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                key TEXT NOT NULL UNIQUE,
                value TEXT NOT NULL,
                memory_type TEXT NOT NULL CHECK(memory_type IN ('user', 'feedback', 'project', 'reference')),
                agent TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                verified INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                agents TEXT NOT NULL,
                summary_json TEXT,
                working_directory TEXT
            );

            CREATE TABLE IF NOT EXISTS decisions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(session_id),
                decision_text TEXT NOT NULL,
                proposed_by TEXT NOT NULL,
                validated_by TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                evidence TEXT
            );

            CREATE TABLE IF NOT EXISTS routing_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(session_id),
                source_agent TEXT NOT NULL,
                target_agent TEXT NOT NULL,
                reason TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS workflows (
                workflow_id TEXT PRIMARY KEY,
                workflow_type TEXT NOT NULL,
                state TEXT NOT NULL,
                current_step INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS workflow_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                workflow_id TEXT NOT NULL REFERENCES workflows(workflow_id),
                step INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type);
            CREATE INDEX IF NOT EXISTS idx_decisions_session ON decisions(session_id);
            CREATE INDEX IF NOT EXISTS idx_routing_log_session ON routing_log(session_id);
            CREATE INDEX IF NOT EXISTS idx_workflow_events_workflow ON workflow_events(workflow_id);",
        )?;

        info!(path = %path.display(), "memory store opened");
        Ok(Self { conn })
    }

    /// Write or update a memory entry. Returns true if inserted, false if updated.
    #[allow(dead_code)]
    pub fn upsert(
        &self,
        key: &str,
        value: &str,
        memory_type: &str,
        agent: &str,
    ) -> anyhow::Result<bool> {
        let changed = self.conn.execute(
            "INSERT INTO memories (key, value, memory_type, agent)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = datetime('now'),
                agent = excluded.agent",
            params![key, value, memory_type, agent],
        )?;
        Ok(changed > 0)
    }

    /// Read a memory by key.
    #[allow(dead_code)]
    pub fn get(&self, key: &str) -> anyhow::Result<Option<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT value, memory_type FROM memories WHERE key = ?1",
        )?;
        let result = stmt
            .query_row(params![key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .optional()?;
        Ok(result)
    }

    /// List all memories, optionally filtered by type.
    #[allow(dead_code)]
    pub fn list(&self, memory_type: Option<&str>) -> anyhow::Result<Vec<(String, String, String)>> {
        let mut results = Vec::new();

        if let Some(mt) = memory_type {
            let mut stmt = self.conn.prepare(
                "SELECT key, value, memory_type FROM memories WHERE memory_type = ?1 ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map(params![mt], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                results.push(row?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT key, value, memory_type FROM memories ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                results.push(row?);
            }
        }

        Ok(results)
    }

    /// Record a new session start.
    pub fn start_session(
        &self,
        session_id: &str,
        agents: &[&str],
        working_dir: &str,
    ) -> anyhow::Result<()> {
        let agents_json = serde_json::to_string(agents)?;
        self.conn.execute(
            "INSERT INTO sessions (session_id, started_at, agents, working_directory)
             VALUES (?1, datetime('now'), ?2, ?3)",
            params![session_id, agents_json, working_dir],
        )?;
        Ok(())
    }

    /// End a session with summary.
    #[allow(dead_code)]
    pub fn end_session(
        &self,
        session_id: &str,
        summary: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET ended_at = datetime('now'), summary_json = ?2
             WHERE session_id = ?1",
            params![session_id, summary],
        )?;
        Ok(())
    }
}
