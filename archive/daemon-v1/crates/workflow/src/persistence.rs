use std::path::Path;

use anyhow::Context;
use rusqlite::{Connection, params};

use crate::state::{WorkflowState, WorkflowType};

pub struct WorkflowStore {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct WorkflowSummary {
    pub workflow_id: String,
    pub workflow_type: String,
    pub state: String,
    pub current_step: i64,
}

#[derive(Debug, Clone)]
pub struct WorkflowEventRow {
    pub step: i64,
    pub event_type: String,
    pub payload_json: String,
}

impl WorkflowStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path).context("open workflow sqlite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS workflows (
                workflow_id TEXT PRIMARY KEY,
                workflow_type TEXT NOT NULL,
                state TEXT NOT NULL,
                current_step INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS workflow_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                workflow_id TEXT NOT NULL,
                step INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY(workflow_id) REFERENCES workflows(workflow_id)
            );

            CREATE INDEX IF NOT EXISTS idx_workflow_events_workflow ON workflow_events(workflow_id);",
        )?;

        Ok(Self { conn })
    }

    pub fn create_workflow(
        &self,
        workflow_id: &str,
        workflow_type: WorkflowType,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO workflows (workflow_id, workflow_type, state, current_step)
             VALUES (?1, ?2, ?3, 0)",
            params![workflow_id, workflow_type.as_str(), WorkflowState::Pending.as_str()],
        )?;
        Ok(())
    }

    pub fn set_state(
        &self,
        workflow_id: &str,
        state: WorkflowState,
        current_step: i64,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE workflows
             SET state = ?2,
                 current_step = ?3,
                 updated_at = datetime('now')
             WHERE workflow_id = ?1",
            params![workflow_id, state.as_str(), current_step],
        )?;
        Ok(())
    }

    pub fn append_event(
        &self,
        workflow_id: &str,
        step: i64,
        event_type: &str,
        payload_json: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO workflow_events (workflow_id, step, event_type, payload_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![workflow_id, step, event_type, payload_json],
        )?;
        Ok(())
    }

    pub fn resumable_workflows(&self) -> anyhow::Result<Vec<WorkflowSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT workflow_id, workflow_type, state, current_step
             FROM workflows
             WHERE state IN ('pending', 'running', 'paused')
             ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WorkflowSummary {
                workflow_id: row.get(0)?,
                workflow_type: row.get(1)?,
                state: row.get(2)?,
                current_step: row.get(3)?,
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn events(&self, workflow_id: &str) -> anyhow::Result<Vec<WorkflowEventRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT step, event_type, payload_json
             FROM workflow_events
             WHERE workflow_id = ?1
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![workflow_id], |row| {
            Ok(WorkflowEventRow {
                step: row.get(0)?,
                event_type: row.get(1)?,
                payload_json: row.get(2)?,
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use uuid::Uuid;

    use super::WorkflowStore;
    use crate::state::{WorkflowState, WorkflowType};

    fn temp_db() -> PathBuf {
        std::env::temp_dir().join(format!("workflow-{}.db", Uuid::new_v4()))
    }

    #[test]
    fn create_and_resume_workflow() {
        let db = temp_db();
        let store = WorkflowStore::open(&db).expect("open");
        let id = Uuid::new_v4().to_string();

        store
            .create_workflow(&id, WorkflowType::Conversation)
            .expect("create");
        store
            .set_state(&id, WorkflowState::Running, 1)
            .expect("set state");

        let resumable = store.resumable_workflows().expect("resumable");
        assert_eq!(resumable.len(), 1);
        assert_eq!(resumable[0].workflow_id, id);
    }

    #[test]
    fn append_and_read_events() {
        let db = temp_db();
        let store = WorkflowStore::open(&db).expect("open");
        let id = Uuid::new_v4().to_string();

        store
            .create_workflow(&id, WorkflowType::Conversation)
            .expect("create");
        store
            .append_event(&id, 0, "started", r#"{"ok":true}"#)
            .expect("event");

        let events = store.events(&id).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "started");
    }
}
