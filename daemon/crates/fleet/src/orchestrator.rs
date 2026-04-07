use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use ledger::LedgerStore;
use shared_types::{GitOps, RawEvent};

use crate::worktree::WorktreeManager;

#[derive(Debug, Clone)]
pub struct FleetSpawnRequest {
    pub project_root: PathBuf,
    pub agents: Vec<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub struct FleetSpawnResult {
    pub fleet_id: String,
    pub plan_text: String,
    pub head_sha: String,
    pub worktree_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct FleetOrchestrator<G: GitOps> {
    worktree: WorktreeManager<G>,
    git_ops: G,
}

impl<G: GitOps + Clone> FleetOrchestrator<G> {
    pub fn new(git_ops: G) -> Self {
        Self {
            worktree: WorktreeManager::new(git_ops.clone()),
            git_ops,
        }
    }

    pub async fn fleet_spawn(&self, req: FleetSpawnRequest) -> anyhow::Result<FleetSpawnResult> {
        if !req.project_root.is_absolute() {
            anyhow::bail!("project_root must be absolute");
        }
        if req.agents.is_empty() {
            anyhow::bail!("at least one agent is required");
        }

        let head_sha = self.git_ops.current_head().await?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let fleet_id = format!("fleet-{now}");
        let plan_text = format!(
            "fleet_id: {fleet_id}\nagent count: {}\nhead sha: {head_sha}\ndry_run: {}",
            req.agents.len(),
            req.dry_run
        );

        let mut worktree_paths = Vec::new();
        if !req.dry_run {
            let base = req.project_root.join(".triumvirate").join("worktrees");
            fs::create_dir_all(&base)?;
            for (idx, agent) in req.agents.iter().enumerate() {
                let task_id = format!("T-{:03}", idx + 1);
                let branch = format!("fleet/{fleet_id}/{task_id}");
                let worktree_path = base.join(format!("{fleet_id}-{task_id}-{agent}"));
                self.worktree
                    .create_worktree(&worktree_path, &branch)
                    .await?;
                fs::create_dir_all(worktree_path.join(".triumvirate"))?;
                fs::write(
                    worktree_path.join(".triumvirate").join("fleet-task.md"),
                    format!(
                        "---\ntask_id: {task_id}\nfleet_id: {fleet_id}\nassigned_agent: {agent}\n---\n"
                    ),
                )?;
                worktree_paths.push(worktree_path);
            }

            // SAFETY: process-level env var is intentionally set for fleet child process context.
            unsafe {
                std::env::set_var("TRIUMVIRATE_PROJECT_ROOT", req.project_root.as_os_str());
            }

            let store = LedgerStore::open(req.project_root.clone())?;
            store.ingest_event(RawEvent {
                session_id: fleet_id.clone(),
                event_type: "fleet_spawned".to_string(),
                sequence: 1,
                timestamp: "2030-01-01T00:00:00Z".to_string(),
                payload_json: serde_json::json!({
                    "fleet_id": fleet_id,
                    "head_sha": head_sha,
                    "agent_count": req.agents.len()
                })
                .to_string(),
            })?;
        }

        Ok(FleetSpawnResult {
            fleet_id,
            plan_text,
            head_sha,
            worktree_paths,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use async_trait::async_trait;

    use shared_types::MergeResult;

    use super::{FleetOrchestrator, FleetSpawnRequest, GitOps, PathBuf};

    #[derive(Debug, Clone)]
    struct MockGitOps {
        touched: Arc<tokio::sync::Mutex<Vec<PathBuf>>>,
    }

    #[async_trait]
    impl GitOps for MockGitOps {
        async fn worktree_add(&self, path: &Path, _branch: &str) -> anyhow::Result<()> {
            self.touched.lock().await.push(path.to_path_buf());
            std::fs::create_dir_all(path)?;
            std::fs::write(path.join(".git"), "gitdir: mock\n")?;
            Ok(())
        }

        async fn worktree_remove(&self, path: &Path) -> anyhow::Result<()> {
            if path.exists() {
                std::fs::remove_dir_all(path)?;
            }
            Ok(())
        }

        async fn is_clean(&self) -> anyhow::Result<bool> {
            Ok(true)
        }

        async fn current_head(&self) -> anyhow::Result<String> {
            Ok("abc123".to_string())
        }

        async fn merge(&self, _branch: &str) -> anyhow::Result<MergeResult> {
            Ok(MergeResult::Success)
        }

        async fn diff(&self, _branch: &str) -> anyhow::Result<String> {
            Ok(String::new())
        }

        async fn rev_parse_toplevel(&self, cwd: &Path) -> anyhow::Result<PathBuf> {
            Ok(cwd.to_path_buf())
        }
    }

    #[tokio::test]
    async fn fleet_spawn_dry_run_and_execute_behave_realistically() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool");
        let _ = ledger::LedgerStore::open(project_root.clone()).expect("open ledger");

        let orchestrator = FleetOrchestrator::new(MockGitOps {
            touched: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        });

        let dry_run = orchestrator
            .fleet_spawn(FleetSpawnRequest {
                project_root: project_root.clone(),
                agents: vec!["codex".to_string(), "gemini".to_string()],
                dry_run: true,
            })
            .await
            .expect("dry run");
        assert!(dry_run.plan_text.contains("agent count: 2"));
        assert!(dry_run.plan_text.contains("head sha: abc123"));
        assert!(dry_run.worktree_paths.is_empty());

        let executed = orchestrator
            .fleet_spawn(FleetSpawnRequest {
                project_root: project_root.clone(),
                agents: vec!["codex".to_string(), "gemini".to_string()],
                dry_run: false,
            })
            .await
            .expect("execute");
        assert_eq!(executed.worktree_paths.len(), 2);
        for path in &executed.worktree_paths {
            assert!(path.exists());
            let task_file = path.join(".triumvirate").join("fleet-task.md");
            assert!(task_file.exists());
            let contents = std::fs::read_to_string(task_file).expect("read task file");
            assert!(contents.contains("task_id:"));
            assert!(contents.contains("fleet_id:"));
            assert!(contents.contains("assigned_agent:"));
        }

        let conn = rusqlite::Connection::open(
            project_root.join(".triumvirate").join("ledger.db"),
        )
        .expect("open sqlite");
        let event_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE event_type = 'fleet_spawned'",
                [],
                |row| row.get(0),
            )
            .expect("count fleet events");
        assert!(event_count >= 1);
    }
}
