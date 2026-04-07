use std::{
    fs,
    path::Path,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use ledger::LedgerStore;
use shared_types::{GitOps, RawEvent};
use tokio::process::Command;

use crate::worktree::WorktreeManager;

#[derive(Debug, Clone)]
pub struct FleetSpawnRequest {
    pub project_root: PathBuf,
    pub agents: Vec<String>,
    pub dry_run: bool,
    pub wait: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct FleetSpawnResult {
    pub fleet_id: String,
    pub plan_text: String,
    pub head_sha: String,
    pub worktree_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct FleetOrchestrator<G: GitOps, L: AgentLauncher = ShellAgentLauncher> {
    worktree: WorktreeManager<G>,
    git_ops: G,
    launcher: L,
}

#[async_trait]
pub trait AgentLauncher: Clone + Send + Sync + 'static {
    async fn launch(
        &self,
        agent: &str,
        project_root: &Path,
        worktree_path: &Path,
    ) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct ShellAgentLauncher;

#[async_trait]
impl AgentLauncher for ShellAgentLauncher {
    async fn launch(
        &self,
        _agent: &str,
        project_root: &Path,
        worktree_path: &Path,
    ) -> anyhow::Result<()> {
        let mut child = Command::new("sh")
            .arg("-lc")
            .arg("true")
            .current_dir(worktree_path)
            .env("TRIUMVIRATE_PROJECT_ROOT", project_root.as_os_str())
            .spawn()?;
        let status = child.wait().await?;
        if !status.success() {
            anyhow::bail!("agent subprocess failed to start");
        }
        Ok(())
    }
}

impl<G: GitOps + Clone + 'static> FleetOrchestrator<G, ShellAgentLauncher> {
    pub fn new(git_ops: G) -> Self {
        Self::with_launcher(git_ops, ShellAgentLauncher)
    }
}

impl<G: GitOps + Clone + 'static, L: AgentLauncher> FleetOrchestrator<G, L> {
    pub fn with_launcher(git_ops: G, launcher: L) -> Self {
        Self {
            worktree: WorktreeManager::new(git_ops.clone()),
            git_ops,
            launcher,
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
            if req.wait.unwrap_or(false) {
                worktree_paths = self
                    .spawn_fleet_members(
                        req.project_root.clone(),
                        fleet_id.clone(),
                        head_sha.clone(),
                        req.agents.clone(),
                    )
                    .await?;
            } else {
                let orchestrator = self.clone();
                let project_root = req.project_root.clone();
                let agents = req.agents.clone();
                let fleet_id_bg = fleet_id.clone();
                let head_sha_bg = head_sha.clone();
                tokio::spawn(async move {
                    let _ = orchestrator
                        .spawn_fleet_members(project_root, fleet_id_bg, head_sha_bg, agents)
                        .await;
                });
            }
        }

        Ok(FleetSpawnResult {
            fleet_id,
            plan_text,
            head_sha,
            worktree_paths,
        })
    }

    async fn spawn_fleet_members(
        &self,
        project_root: PathBuf,
        fleet_id: String,
        head_sha: String,
        agents: Vec<String>,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let base = project_root.join(".triumvirate").join("worktrees");
        fs::create_dir_all(&base)?;
        let store = LedgerStore::open(project_root.clone())?;
        let mut worktree_paths = Vec::new();
        for (idx, agent) in agents.iter().enumerate() {
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
                    "---\ntask_id: {task_id}\nfleet_id: {fleet_id}\nassigned_agent: {agent}\ndepends_on: []\n---\n\nImplement the assigned fleet task for {agent}.\nDocument changes and tests in your final report.\n"
                ),
            )?;
            self.launcher
                .launch(agent, &project_root, &worktree_path)
                .await?;
            let sequence = event_sequence_for(&project_root, &fleet_id, "agent_started")?;
            store.ingest_event(RawEvent {
                session_id: fleet_id.clone(),
                event_type: "agent_started".to_string(),
                sequence,
                timestamp: "2030-01-01T00:00:00Z".to_string(),
                payload_json: serde_json::json!({
                    "fleet_id": fleet_id,
                    "task_id": task_id,
                    "agent": agent
                })
                .to_string(),
            })?;
            worktree_paths.push(worktree_path);
        }

        store.ingest_event(RawEvent {
            session_id: fleet_id,
            event_type: "fleet_spawned".to_string(),
            sequence: 1,
            timestamp: "2030-01-01T00:00:00Z".to_string(),
            payload_json: serde_json::json!({
                "head_sha": head_sha,
                "agent_count": agents.len()
            })
            .to_string(),
        })?;
        Ok(worktree_paths)
    }
}

fn event_sequence_for(
    project_root: &Path,
    session_id: &str,
    event_type: &str,
) -> anyhow::Result<i64> {
    let conn = rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))?;
    let max_seq: Option<i64> = conn.query_row(
        "SELECT MAX(sequence) FROM events WHERE session_id = ?1 AND event_type = ?2",
        rusqlite::params![session_id, event_type],
        |row| row.get::<_, Option<i64>>(0),
    )?;
    Ok(max_seq.unwrap_or(0) + 1)
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc, time::Duration};

    use async_trait::async_trait;
    use tokio::sync::Mutex;

    use shared_types::MergeResult;

    use crate::merge::{MergeCoordinator, ReviewGateState};
    use crate::tasks::FleetTaskStore;

    use super::{AgentLauncher, FleetOrchestrator, FleetSpawnRequest, GitOps, PathBuf};

    #[derive(Debug, Clone)]
    struct MockGitOps {
        touched: Arc<Mutex<Vec<PathBuf>>>,
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

    #[derive(Debug, Clone, Default)]
    struct RecordingLauncher {
        seen_project_roots: Arc<Mutex<Vec<PathBuf>>>,
    }

    #[async_trait]
    impl AgentLauncher for RecordingLauncher {
        async fn launch(
            &self,
            _agent: &str,
            project_root: &Path,
            _worktree_path: &Path,
        ) -> anyhow::Result<()> {
            tokio::time::sleep(Duration::from_millis(25)).await;
            self.seen_project_roots
                .lock()
                .await
                .push(project_root.to_path_buf());
            Ok(())
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
            touched: Arc::new(Mutex::new(Vec::new())),
        });

        let dry_run = orchestrator
            .fleet_spawn(FleetSpawnRequest {
                project_root: project_root.clone(),
                agents: vec!["codex".to_string(), "gemini".to_string()],
                dry_run: true,
                wait: None,
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
                wait: Some(true),
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
            assert!(contents.contains("depends_on: []"));
            assert!(contents.contains("Implement the assigned fleet task"));
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

    #[tokio::test]
    async fn concurrent_fleet_spawns_keep_project_root_scoped_per_launch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_a = temp.path().join("project-a");
        let project_b = temp.path().join("project-b");
        std::fs::create_dir_all(project_a.join(".triumvirate").join("spool")).expect("spool a");
        std::fs::create_dir_all(project_b.join(".triumvirate").join("spool")).expect("spool b");
        let _ = ledger::LedgerStore::open(project_a.clone()).expect("open ledger a");
        let _ = ledger::LedgerStore::open(project_b.clone()).expect("open ledger b");

        let launcher = RecordingLauncher::default();
        let seen = launcher.seen_project_roots.clone();
        let orchestrator = FleetOrchestrator::with_launcher(
            MockGitOps {
                touched: Arc::new(Mutex::new(Vec::new())),
            },
            launcher,
        );

        let spawn_a = orchestrator.fleet_spawn(FleetSpawnRequest {
            project_root: project_a.clone(),
            agents: vec!["codex".to_string()],
            dry_run: false,
            wait: Some(true),
        });
        let spawn_b = orchestrator.fleet_spawn(FleetSpawnRequest {
            project_root: project_b.clone(),
            agents: vec!["gemini".to_string()],
            dry_run: false,
            wait: Some(true),
        });
        let (res_a, res_b) = tokio::join!(spawn_a, spawn_b);
        res_a.expect("spawn a");
        res_b.expect("spawn b");

        let captured = seen.lock().await.clone();
        assert_eq!(captured.len(), 2);
        assert!(captured.iter().any(|p| p == &project_a));
        assert!(captured.iter().any(|p| p == &project_b));
    }

    #[tokio::test]
    async fn lifecycle_events_include_all_required_progress_types() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool");
        let _ = ledger::LedgerStore::open(project_root.clone()).expect("open ledger");

        let launcher = RecordingLauncher::default();
        let orchestrator = FleetOrchestrator::with_launcher(
            MockGitOps {
                touched: Arc::new(Mutex::new(Vec::new())),
            },
            launcher,
        );
        let spawned = orchestrator
            .fleet_spawn(FleetSpawnRequest {
                project_root: project_root.clone(),
                agents: vec!["codex".to_string()],
                dry_run: false,
                wait: Some(true),
            })
            .await
            .expect("spawn");

        let tasks = FleetTaskStore::new(project_root.clone()).expect("task store");
        tasks
            .insert_fleet(&spawned.fleet_id, "test lifecycle")
            .expect("insert fleet");
        tasks
            .insert_task("T-001", &spawned.fleet_id, "task", &[])
            .expect("insert task");
        assert!(tasks.claim_task("T-001", "codex").expect("claim"));
        tasks.complete_task("T-001").expect("complete");

        let mut merge = MergeCoordinator::new(MockGitOps {
            touched: Arc::new(Mutex::new(Vec::new())),
        })
        .with_project_root(project_root.clone());
        merge.enqueue_completed("T-001", format!("fleet/{}/T-001", spawned.fleet_id));
        merge.set_review_status("T-001", ReviewGateState::Approved, None);
        let merged = merge.merge_next().await.expect("merge");
        assert_eq!(merged.as_deref(), Some("T-001"));

        let conn = rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))
            .expect("open sqlite");
        for event_type in [
            "agent_started",
            "task_claimed",
            "task_completed",
            "merge_started",
            "merge_result",
            "fleet_done",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND event_type = ?2",
                    rusqlite::params![spawned.fleet_id, event_type],
                    |row| row.get(0),
                )
                .expect("count lifecycle event");
            assert!(count >= 1, "missing event type: {event_type}");
        }
    }

    #[tokio::test]
    async fn fleet_spawn_wait_true_blocks_and_wait_false_returns_spawning_fast() {
        use tokio::time::Instant;

        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool");
        let _ = ledger::LedgerStore::open(project_root.clone()).expect("open ledger");

        let launcher = RecordingLauncher::default();
        let orchestrator = FleetOrchestrator::with_launcher(
            MockGitOps {
                touched: Arc::new(Mutex::new(Vec::new())),
            },
            launcher,
        );

        let t0 = Instant::now();
        let no_wait = orchestrator
            .fleet_spawn(FleetSpawnRequest {
                project_root: project_root.clone(),
                agents: vec!["codex".to_string()],
                dry_run: false,
                wait: Some(false),
            })
            .await
            .expect("spawn no-wait");
        assert!(t0.elapsed() < Duration::from_millis(20));
        assert!(no_wait.worktree_paths.is_empty());

        let t1 = Instant::now();
        let wait = orchestrator
            .fleet_spawn(FleetSpawnRequest {
                project_root,
                agents: vec!["gemini".to_string()],
                dry_run: false,
                wait: Some(true),
            })
            .await
            .expect("spawn wait");
        assert!(t1.elapsed() >= Duration::from_millis(20));
        assert_eq!(wait.worktree_paths.len(), 1);
    }
}
