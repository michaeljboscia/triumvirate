use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetTask {
    pub task_id: String,
    pub fleet_id: String,
    pub title: String,
    pub assigned_agent: Option<String>,
    pub state: String,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FleetTaskStore {
    db_path: PathBuf,
}

impl FleetTaskStore {
    pub fn new(project_root: PathBuf) -> anyhow::Result<Self> {
        if !project_root.is_absolute() {
            anyhow::bail!("project_root must be absolute");
        }
        Ok(Self {
            db_path: project_root.join(".triumvirate").join("ledger.db"),
        })
    }

    pub fn insert_fleet(&self, fleet_id: &str, task_description: &str) -> anyhow::Result<()> {
        let conn = self.open_conn()?;
        conn.execute(
            "INSERT INTO fleets (fleet_id, task_description, agent_composition, source_project_root)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![fleet_id, task_description, "{\"codex\":1}", "."],
        )?;
        Ok(())
    }

    pub fn insert_task(
        &self,
        task_id: &str,
        fleet_id: &str,
        title: &str,
        depends_on: &[String],
    ) -> anyhow::Result<()> {
        let conn = self.open_conn()?;
        let depends_json = if depends_on.is_empty() {
            None
        } else {
            Some(serde_json::to_string(depends_on)?)
        };
        conn.execute(
            "INSERT INTO tasks (task_id, fleet_id, title, depends_on) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![task_id, fleet_id, title, depends_json],
        )?;
        Ok(())
    }

    pub fn list_claimable(&self, fleet_id: &str) -> anyhow::Result<Vec<FleetTask>> {
        let conn = self.open_conn()?;
        let mut stmt = conn.prepare(
            "SELECT task_id, fleet_id, title, assigned_agent, state, depends_on
             FROM tasks
             WHERE fleet_id = ?1 AND state = 'pending'
             ORDER BY task_id ASC",
        )?;
        let rows = stmt.query_map([fleet_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (task_id, fleet_id, title, assigned_agent, state, depends_on_json) = row?;
            let depends_on = parse_depends(depends_on_json.as_deref())?;
            if dependencies_satisfied(&conn, &depends_on)? {
                out.push(FleetTask {
                    task_id,
                    fleet_id,
                    title,
                    assigned_agent,
                    state,
                    depends_on,
                });
            }
        }
        Ok(out)
    }

    pub fn claim_task(&self, task_id: &str, assigned_agent: &str) -> anyhow::Result<bool> {
        let conn = self.open_conn()?;
        conn.execute_batch("BEGIN IMMEDIATE")?;

        let pending_and_deps: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT state, depends_on FROM tasks WHERE task_id = ?1",
                [task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let Some((state, depends_on_json)) = pending_and_deps else {
            conn.execute_batch("ROLLBACK")?;
            return Ok(false);
        };

        if state != "pending" {
            conn.execute_batch("ROLLBACK")?;
            return Ok(false);
        }

        let depends_on = parse_depends(depends_on_json.as_deref())?;
        if !dependencies_satisfied(&conn, &depends_on)? {
            conn.execute_batch("ROLLBACK")?;
            return Ok(false);
        }

        let updated = conn.execute(
            "UPDATE tasks
             SET state = 'claimed', assigned_agent = ?2
             WHERE task_id = ?1 AND state = 'pending'",
            rusqlite::params![task_id, assigned_agent],
        )?;

        if updated == 1 {
            conn.execute_batch("COMMIT")?;
            Ok(true)
        } else {
            conn.execute_batch("ROLLBACK")?;
            Ok(false)
        }
    }

    pub fn complete_task(&self, task_id: &str) -> anyhow::Result<()> {
        let conn = self.open_conn()?;
        conn.execute(
            "UPDATE tasks SET state = 'done', completed_at = datetime('now') WHERE task_id = ?1",
            [task_id],
        )?;
        Ok(())
    }

    fn open_conn(&self) -> anyhow::Result<Connection> {
        if !self.db_path.exists() {
            anyhow::bail!("ledger database missing at {}", self.db_path.display());
        }
        let conn = Connection::open(&self.db_path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(conn)
    }
}

fn parse_depends(raw: Option<&str>) -> anyhow::Result<Vec<String>> {
    match raw {
        Some(json) if !json.trim().is_empty() => Ok(serde_json::from_str::<Vec<String>>(json)?),
        _ => Ok(Vec::new()),
    }
}

fn dependencies_satisfied(conn: &Connection, depends_on: &[String]) -> anyhow::Result<bool> {
    for dep in depends_on {
        let state: Option<String> = conn
            .query_row(
                "SELECT state FROM tasks WHERE task_id = ?1",
                [dep],
                |row| row.get(0),
            )
            .optional()?;
        if state.as_deref() != Some("done") {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Arc, thread};

    use ledger::LedgerStore;

    use super::FleetTaskStore;

    #[test]
    fn claimable_respects_dependencies_and_atomic_claims() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool dir");
        let _store = LedgerStore::open(project_root.clone()).expect("open ledger store");

        let tasks = FleetTaskStore::new(project_root.clone()).expect("task store");
        tasks
            .insert_fleet("fleet-1", "test fleet")
            .expect("insert fleet");
        tasks
            .insert_task("T-001", "fleet-1", "first", &[])
            .expect("insert t1");
        tasks
            .insert_task(
                "T-002",
                "fleet-1",
                "second",
                &["T-001".to_string()],
            )
            .expect("insert t2");

        let claimable_initial = tasks.list_claimable("fleet-1").expect("list claimable");
        assert_eq!(claimable_initial.len(), 1);
        assert_eq!(claimable_initial[0].task_id, "T-001");

        assert!(tasks.claim_task("T-001", "codex").expect("claim first"));
        assert!(!tasks
            .claim_task("T-001", "gemini")
            .expect("claim first again"));

        tasks.complete_task("T-001").expect("complete first");
        let claimable_after = tasks.list_claimable("fleet-1").expect("list claimable after");
        assert_eq!(claimable_after.len(), 1);
        assert_eq!(claimable_after[0].task_id, "T-002");

        tasks
            .insert_task("T-003", "fleet-1", "race", &[])
            .expect("insert t3");

        let task_a = Arc::new(FleetTaskStore::new(project_root.clone()).expect("task store a"));
        let task_b = Arc::new(FleetTaskStore::new(project_root).expect("task store b"));

        let a = {
            let task_a = Arc::clone(&task_a);
            thread::spawn(move || task_a.claim_task("T-003", "codex").expect("claim a"))
        };
        let b = {
            let task_b = Arc::clone(&task_b);
            thread::spawn(move || task_b.claim_task("T-003", "gemini").expect("claim b"))
        };

        let first = a.join().expect("join a");
        let second = b.join().expect("join b");
        assert_ne!(first, second, "exactly one claim should succeed");
    }
}
