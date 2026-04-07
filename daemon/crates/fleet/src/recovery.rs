use std::{fs, path::PathBuf};

use ledger::LedgerStore;
use rusqlite::Connection;
use shared_types::RawEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryResult {
    pub failed_fleets: Vec<String>,
    pub cleaned_worktrees: usize,
}

pub fn recover_crashed_fleets(project_root: PathBuf) -> anyhow::Result<RecoveryResult> {
    if !project_root.is_absolute() {
        anyhow::bail!("project_root must be absolute");
    }
    let db_path = project_root.join(".triumvirate").join("ledger.db");
    let conn = Connection::open(&db_path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;

    let mut stmt = conn.prepare(
        "SELECT fleet_id FROM fleets
         WHERE state IN ('spawning', 'running', 'merging', 'recovery_required')",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;

    let mut failed_fleets = Vec::new();
    for row in rows {
        failed_fleets.push(row?);
    }

    let mut cleaned_worktrees = 0usize;
    let worktree_base = project_root.join(".triumvirate").join("worktrees");
    for fleet_id in &failed_fleets {
        conn.execute(
            "UPDATE fleets
             SET state = 'failed',
                 completed_at = datetime('now'),
                 failure_reason = 'crash recovery: stale fleet detected'
             WHERE fleet_id = ?1",
            [fleet_id],
        )?;
        conn.execute(
            "UPDATE tasks
             SET state = 'pending',
                 assigned_agent = NULL
             WHERE fleet_id = ?1
               AND state IN ('claimed', 'in_progress')",
            [fleet_id],
        )?;

        if worktree_base.exists() {
            for entry in fs::read_dir(&worktree_base)? {
                let entry = entry?;
                let path = entry.path();
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                if name.starts_with(&format!("{fleet_id}-")) && path.is_dir() {
                    fs::remove_dir_all(&path)?;
                    cleaned_worktrees += 1;
                }
            }
        }
    }

    if !failed_fleets.is_empty() {
        let store = LedgerStore::open(project_root)?;
        for (idx, fleet_id) in failed_fleets.iter().enumerate() {
            store.ingest_event(RawEvent {
                session_id: fleet_id.clone(),
                event_type: "fleet_recovery".to_string(),
                sequence: (idx + 1) as i64,
                timestamp: "2030-01-01T00:00:00Z".to_string(),
                payload_json: serde_json::json!({
                    "fleet_id": fleet_id,
                    "reason": "crash recovery: stale fleet detected"
                })
                .to_string(),
            })?;
        }
    }

    Ok(RecoveryResult {
        failed_fleets,
        cleaned_worktrees,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ledger::LedgerStore;

    use super::recover_crashed_fleets;

    #[test]
    fn recovery_marks_failed_cleans_worktrees_and_logs_event() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool")).expect("spool");
        let _store = LedgerStore::open(project_root.clone()).expect("open ledger");
        let conn = rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))
            .expect("open sqlite");
        conn.execute(
            "INSERT INTO fleets (fleet_id, task_description, agent_composition, source_project_root, state)
             VALUES ('fleet-1', 'test', '{\"codex\":1}', ?1, 'running')",
            [project_root.display().to_string()],
        )
        .expect("insert fleet");
        conn.execute(
            "INSERT INTO tasks (fleet_id, task_id, title, assigned_agent, state, depends_on)
             VALUES ('fleet-1', 'T-001', 'task one', 'codex', 'claimed', '[]')",
            [],
        )
        .expect("insert claimed task");
        conn.execute(
            "INSERT INTO tasks (fleet_id, task_id, title, assigned_agent, state, depends_on)
             VALUES ('fleet-1', 'T-002', 'task two', 'gemini', 'in_progress', '[]')",
            [],
        )
        .expect("insert in progress task");
        conn.execute(
            "INSERT INTO tasks (fleet_id, task_id, title, assigned_agent, state, depends_on)
             VALUES ('fleet-1', 'T-003', 'task three', 'claude', 'done', '[]')",
            [],
        )
        .expect("insert done task");

        let wt = project_root
            .join(".triumvirate")
            .join("worktrees")
            .join("fleet-1-T-001-codex");
        fs::create_dir_all(&wt).expect("create worktree");
        fs::write(wt.join("file.txt"), "x").expect("write worktree file");

        let result = recover_crashed_fleets(project_root.clone()).expect("run recovery");
        assert_eq!(result.failed_fleets, vec!["fleet-1".to_string()]);
        assert!(result.cleaned_worktrees >= 1);
        assert!(!wt.exists());

        let fleet_state: (String, String) = conn
            .query_row(
                "SELECT state, failure_reason FROM fleets WHERE fleet_id = 'fleet-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read fleet state");
        assert_eq!(fleet_state.0, "failed");
        assert!(fleet_state.1.contains("crash recovery"));

        let reset_tasks: Vec<(String, Option<String>)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT state, assigned_agent
                     FROM tasks
                     WHERE fleet_id = 'fleet-1' AND task_id IN ('T-001', 'T-002')
                     ORDER BY task_id ASC",
                )
                .expect("prepare reset tasks query");
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))
                .expect("query reset tasks");
            let mut out = Vec::new();
            for row in rows {
                out.push(row.expect("row"));
            }
            out
        };
        assert_eq!(reset_tasks.len(), 2);
        assert_eq!(reset_tasks[0].0, "pending");
        assert!(reset_tasks[0].1.is_none());
        assert_eq!(reset_tasks[1].0, "pending");
        assert!(reset_tasks[1].1.is_none());

        let done_task: (String, Option<String>) = conn
            .query_row(
                "SELECT state, assigned_agent
                 FROM tasks
                 WHERE fleet_id = 'fleet-1' AND task_id = 'T-003'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read done task");
        assert_eq!(done_task.0, "done");
        assert_eq!(done_task.1.as_deref(), Some("claude"));

        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE event_type = 'fleet_recovery'",
                [],
                |row| row.get(0),
            )
            .expect("count recovery events");
        assert!(events >= 1);
    }
}
