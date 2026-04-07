use std::{
    fs,
    path::Path,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use ledger::LedgerStore;
use shared_types::{GitOps, RawEvent};

use crate::{
    merge::{MergeCoordinator, ReviewGateState},
    tasks::FleetTaskStore,
    worktree::WorktreeManager,
};

#[derive(Debug, Clone)]
pub struct FleetSpawnRequest {
    pub project_root: PathBuf,
    pub agents: Vec<String>,
    pub dry_run: bool,
    pub wait: Option<bool>,
    pub task_description: String,
}

#[derive(Debug, Clone)]
pub struct FleetSpawnResult {
    pub fleet_id: String,
    pub plan_text: String,
    pub head_sha: String,
    pub worktree_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct FleetOrchestrator<G: GitOps, L: AgentLauncher = DaemonAgentLauncher> {
    worktree: WorktreeManager<G>,
    git_ops: G,
    launcher: L,
}

#[async_trait]
pub trait AgentLauncher: Clone + Send + Sync + 'static {
    /// Launch an agent in the given worktree. Returns when the agent completes.
    /// Uses the daemon's ask_agent path — not a raw subprocess.
    async fn launch(
        &self,
        agent: &str,
        project_root: &Path,
        worktree_path: &Path,
        task_prompt: &str,
    ) -> anyhow::Result<String>;
}

#[derive(Debug, Clone, Default)]
pub struct DaemonAgentLauncher;

#[async_trait]
impl AgentLauncher for DaemonAgentLauncher {
    async fn launch(
        &self,
        agent: &str,
        _project_root: &Path,
        worktree_path: &Path,
        task_prompt: &str,
    ) -> anyhow::Result<String> {
        let req = shared_types::AskAgentRequest {
            agent: agent.to_string(),
            message: task_prompt.to_string(),
            cwd: Some(worktree_path.to_string_lossy().to_string()),
            repo: None,
            branch: None,
        };
        let resp = daemon_http::fetch_daemon_ask_agent(&req).await?;
        Ok(resp.response)
    }
}

impl<G: GitOps + Clone + 'static> FleetOrchestrator<G, DaemonAgentLauncher> {
    pub fn new(git_ops: G) -> Self {
        Self::with_launcher(git_ops, DaemonAgentLauncher)
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
                        req.task_description.clone(),
                    )
                    .await?;
            } else {
                let orchestrator = self.clone();
                let project_root = req.project_root.clone();
                let agents = req.agents.clone();
                let fleet_id_bg = fleet_id.clone();
                let head_sha_bg = head_sha.clone();
                let task_description = req.task_description.clone();
                tokio::spawn(async move {
                    if let Err(err) = orchestrator
                        .spawn_fleet_members(
                            project_root.clone(),
                            fleet_id_bg.clone(),
                            head_sha_bg,
                            agents,
                            task_description,
                        )
                        .await
                    {
                        tracing::error!(
                            fleet_id = %fleet_id_bg,
                            error = %err,
                            "fleet background spawn failed"
                        );
                        let db_path = project_root.join(".triumvirate").join("ledger.db");
                        if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                            let _ = conn.execute(
                                "UPDATE fleets
                                 SET state = 'failed', failure_reason = ?2
                                 WHERE fleet_id = ?1",
                                rusqlite::params![fleet_id_bg.as_str(), err.to_string()],
                            );
                        }
                        if let Ok(store) = LedgerStore::open(project_root.clone()) {
                            let sequence = event_sequence_for(&project_root, &fleet_id_bg, "fleet_failed")
                                .unwrap_or(1);
                            let _ = store.ingest_event(RawEvent {
                                session_id: fleet_id_bg.clone(),
                                event_type: "fleet_failed".to_string(),
                                sequence,
                                timestamp: "2030-01-01T00:00:00Z".to_string(),
                                payload_json: serde_json::json!({
                                    "error": err.to_string()
                                })
                                .to_string(),
                            });
                        }
                    }
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
        task_description: String,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let base = project_root.join(".triumvirate").join("worktrees");
        fs::create_dir_all(&base)?;
        let store = LedgerStore::open(project_root.clone())?;
        let task_store = FleetTaskStore::new(project_root.clone())?;
        task_store.insert_fleet(&fleet_id, &task_description)?;
        let mut worktree_paths = Vec::new();
        let mut running_agents = Vec::new();
        for (idx, agent) in agents.iter().enumerate() {
            let task_id = format!("T-{:03}", idx + 1);
            let branch = format!("fleet/{fleet_id}/{task_id}");
            let worktree_path = base.join(format!("{fleet_id}-{task_id}-{agent}"));
            task_store.insert_task(&task_id, &fleet_id, &task_id, &[])?;
            let conn = rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))?;
            conn.execute(
                "UPDATE tasks
                 SET state = 'in_progress', assigned_agent = ?2
                 WHERE task_id = ?1",
                rusqlite::params![task_id.as_str(), agent.as_str()],
            )?;
            self.worktree
                .create_worktree(&worktree_path, &branch)
                .await?;
            fs::create_dir_all(worktree_path.join(".triumvirate"))?;
            fs::write(
                worktree_path.join(".triumvirate").join("fleet-task.md"),
                format!(
                    "---\ntask_id: {task_id}\nfleet_id: {fleet_id}\nassigned_agent: {agent}\ndepends_on: []\n---\n\n{task_description}\n"
                ),
            )?;
            let task_prompt = format!(
                "You are a fleet agent working in a git worktree. Read your task assignment at .triumvirate/fleet-task.md and complete the work. Commit your changes when done.\n\nTask: {task_description}"
            );
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
            let sequence = event_sequence_for(&project_root, &fleet_id, "task_claimed")?;
            store.ingest_event(RawEvent {
                session_id: fleet_id.clone(),
                event_type: "task_claimed".to_string(),
                sequence,
                timestamp: "2030-01-01T00:00:00Z".to_string(),
                payload_json: serde_json::json!({
                    "task_id": task_id,
                    "assigned_agent": agent
                })
                .to_string(),
            })?;
            // Launch via daemon's ask_agent — blocks until agent completes
            running_agents.push((task_prompt, task_id.clone(), agent.to_string()));
            worktree_paths.push(worktree_path);
        }

        let conn = rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))?;
        conn.execute(
            "UPDATE fleets SET state = 'running' WHERE fleet_id = ?1",
            [fleet_id.as_str()],
        )?;
        store.ingest_event(RawEvent {
            session_id: fleet_id.clone(),
            event_type: "fleet_spawned".to_string(),
            sequence: 1,
            timestamp: "2030-01-01T00:00:00Z".to_string(),
            payload_json: serde_json::json!({
                "head_sha": head_sha,
                "agent_count": agents.len()
            })
            .to_string(),
        })?;

        // Launch all agents in parallel via daemon's ask_agent
        let mut join_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
        for (task_prompt, task_id, agent_name) in running_agents {
            let launcher = self.launcher.clone();
            let orchestrator = self.clone();
            let project_root = project_root.clone();
            let fleet_id = fleet_id.clone();
            let worktree_path = worktree_paths[join_handles.len()].clone();
            let jh = tokio::spawn(async move {
                tracing::info!(fleet_id = %fleet_id, task_id = %task_id, agent = %agent_name, "launching fleet agent via ask_agent");
                let result = launcher.launch(&agent_name, &project_root, &worktree_path, &task_prompt).await;
                match result {
                    Ok(response) => {
                        tracing::info!(
                            fleet_id = %fleet_id,
                            task_id = %task_id,
                            "fleet agent completed successfully"
                        );
                        if let Ok(task_store) = FleetTaskStore::new(project_root.clone()) {
                            let _ = task_store.complete_task(&task_id);
                        }
                        if let Ok(store) = LedgerStore::open(project_root.clone()) {
                            let seq = event_sequence_for(&project_root, &fleet_id, "task_completed").unwrap_or(1);
                            let _ = store.ingest_event(RawEvent {
                                session_id: fleet_id.clone(),
                                event_type: "task_completed".to_string(),
                                sequence: seq,
                                timestamp: "2030-01-01T00:00:00Z".to_string(),
                                payload_json: serde_json::json!({"task_id": task_id, "agent": agent_name}).to_string(),
                            });
                        }
                        if let Ok(review_engine) = peer_review::PeerReviewEngine::new(project_root.clone()) {
                            let _ = review_engine.request_review(peer_review::ReviewRequest {
                                fleet_id: Some(fleet_id.clone()),
                                author_agent: agent_name.clone(),
                                artifact: format!("fleet/{fleet_id}/{task_id}"),
                                review_type: "code".to_string(),
                            });
                            tracing::info!(
                                fleet_id = %fleet_id,
                                task_id = %task_id,
                                reviewer_target = %agent_name,
                                "peer review requested for completed task"
                            );
                        }
                    }
                    Err(err) => {
                        tracing::error!(
                            fleet_id = %fleet_id,
                            task_id = %task_id,
                            error = %err,
                            "fleet agent failed via ask_agent"
                        );
                        let db_path = project_root.join(".triumvirate").join("ledger.db");
                        if let Ok(conn) = rusqlite::Connection::open(db_path) {
                            let _ = conn.execute(
                                "UPDATE tasks SET state = 'failed' WHERE task_id = ?1",
                                [task_id.as_str()],
                            );
                        }
                        if let Ok(store) = LedgerStore::open(project_root.clone()) {
                            let sequence = event_sequence_for(&project_root, &fleet_id, "task_failed")
                                .unwrap_or(1);
                            let _ = store.ingest_event(RawEvent {
                                session_id: fleet_id.clone(),
                                event_type: "task_failed".to_string(),
                                sequence,
                                timestamp: "2030-01-01T00:00:00Z".to_string(),
                                payload_json: serde_json::json!({
                                    "task_id": task_id,
                                    "error": err.to_string()
                                })
                                .to_string(),
                            });
                        }
                    }
                }

                let db_path = project_root.join(".triumvirate").join("ledger.db");
                let pending = rusqlite::Connection::open(db_path)
                    .and_then(|conn| {
                        conn.query_row(
                            "SELECT COUNT(*) FROM tasks
                             WHERE fleet_id = ?1 AND state NOT IN ('done', 'failed')",
                            [fleet_id.as_str()],
                            |row| row.get::<_, i64>(0),
                        )
                    })
                    .unwrap_or(1);
                if pending == 0 {
                    tracing::info!(fleet_id = %fleet_id, "all fleet agents complete, starting merge phase");
                    let _ = orchestrator.complete_fleet(&fleet_id, &project_root).await;
                }
            });
            join_handles.push(jh);
        }
        // Await all agent completions
        for jh in join_handles {
            let _ = jh.await;
        }
        Ok(worktree_paths)
    }

    async fn complete_fleet(&self, fleet_id: &str, project_root: &Path) -> anyhow::Result<()> {
        let db_path = project_root.join(".triumvirate").join("ledger.db");
        let conn = rusqlite::Connection::open(&db_path)?;
        let updated = conn.execute(
            "UPDATE fleets
             SET state = 'merging'
             WHERE fleet_id = ?1 AND state IN ('spawning', 'running')",
            [fleet_id],
        )?;
        if updated == 0 {
            return Ok(());
        }
        tracing::info!(fleet_id = %fleet_id, "starting sequential merge");
        let store = LedgerStore::open(project_root.to_path_buf())?;
        let merge_started_seq = event_sequence_for(project_root, fleet_id, "merge_started")?;
        store.ingest_event(RawEvent {
            session_id: fleet_id.to_string(),
            event_type: "merge_started".to_string(),
            sequence: merge_started_seq,
            timestamp: "2030-01-01T00:00:00Z".to_string(),
            payload_json: serde_json::json!({
                "fleet_id": fleet_id
            })
            .to_string(),
        })?;

        let task_ids: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT task_id FROM tasks
                 WHERE fleet_id = ?1 AND state = 'done'
                 ORDER BY task_id ASC",
            )?;
            let rows = stmt.query_map([fleet_id], |row| row.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        };
        let failed_tasks: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE fleet_id = ?1 AND state = 'failed'",
            [fleet_id],
            |row| row.get(0),
        )?;
        drop(conn);

        if failed_tasks > 0 {
            let conn = rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))?;
            conn.execute(
                "UPDATE fleets
                 SET state = 'failed', failure_reason = ?2
                 WHERE fleet_id = ?1",
                rusqlite::params![
                    fleet_id,
                    format!("{failed_tasks} task(s) failed before merge"),
                ],
            )?;
            let failed_seq = event_sequence_for(project_root, fleet_id, "fleet_failed")?;
            store.ingest_event(RawEvent {
                session_id: fleet_id.to_string(),
                event_type: "fleet_failed".to_string(),
                sequence: failed_seq,
                timestamp: "2030-01-01T00:00:00Z".to_string(),
                payload_json: serde_json::json!({
                    "fleet_id": fleet_id,
                    "failed_tasks": failed_tasks
                })
                .to_string(),
            })?;
            tracing::error!(
                fleet_id = %fleet_id,
                failed_tasks,
                "fleet contains failed tasks; skipping merge"
            );
            return Ok(());
        }

        let mut coordinator =
            MergeCoordinator::new(self.git_ops.clone()).with_project_root(project_root.to_path_buf());
        for task_id in task_ids {
            coordinator.enqueue_completed(task_id.clone(), format!("fleet/{fleet_id}/{task_id}"));
            coordinator.set_review_status(task_id, ReviewGateState::Approved, None);
        }

        let merge_result = async {
            while coordinator.merge_next().await?.is_some() {}
            Ok::<(), anyhow::Error>(())
        }
        .await;

        let conn = rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))?;
        match merge_result {
            Ok(()) => {
                conn.execute(
                    "UPDATE fleets
                     SET state = 'done', completed_at = datetime('now')
                     WHERE fleet_id = ?1",
                    [fleet_id],
                )?;
                let done_seq = event_sequence_for(project_root, fleet_id, "fleet_done")?;
                store.ingest_event(RawEvent {
                    session_id: fleet_id.to_string(),
                    event_type: "fleet_done".to_string(),
                    sequence: done_seq,
                    timestamp: "2030-01-01T00:00:00Z".to_string(),
                    payload_json: serde_json::json!({
                        "fleet_id": fleet_id
                    })
                    .to_string(),
                })?;
            }
            Err(err) => {
                conn.execute(
                    "UPDATE fleets
                     SET state = 'failed', failure_reason = ?2
                     WHERE fleet_id = ?1",
                    rusqlite::params![fleet_id, err.to_string()],
                )?;
                let failed_seq = event_sequence_for(project_root, fleet_id, "fleet_failed")?;
                store.ingest_event(RawEvent {
                    session_id: fleet_id.to_string(),
                    event_type: "fleet_failed".to_string(),
                    sequence: failed_seq,
                    timestamp: "2030-01-01T00:00:00Z".to_string(),
                    payload_json: serde_json::json!({
                        "fleet_id": fleet_id,
                        "error": err.to_string()
                    })
                    .to_string(),
                })?;
                tracing::error!(fleet_id = %fleet_id, error = %err, "fleet merge failed");
            }
        }
        Ok(())
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
            _task_prompt: &str,
        ) -> anyhow::Result<String> {
            tokio::time::sleep(Duration::from_millis(25)).await;
            self.seen_project_roots
                .lock()
                .await
                .push(project_root.to_path_buf());
            Ok("mock agent completed".to_string())
        }
    }

    #[derive(Debug, Clone, Default)]
    struct FailingLauncher;

    #[async_trait]
    impl AgentLauncher for FailingLauncher {
        async fn launch(
            &self,
            _agent: &str,
            _project_root: &Path,
            _worktree_path: &Path,
            _task_prompt: &str,
        ) -> anyhow::Result<String> {
            anyhow::bail!("launcher failure");
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
                task_description: "test task".to_string(),
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
                task_description: "real task description".to_string(),
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
            assert!(contents.contains("real task description"));
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
            task_description: "task a".to_string(),
        });
        let spawn_b = orchestrator.fleet_spawn(FleetSpawnRequest {
            project_root: project_b.clone(),
            agents: vec!["gemini".to_string()],
            dry_run: false,
            wait: Some(true),
            task_description: "task b".to_string(),
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
                task_description: "lifecycle task".to_string(),
            })
            .await
            .expect("spawn");

        let tasks = FleetTaskStore::new(project_root.clone()).expect("task store");
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
                task_description: "wait false task".to_string(),
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
                task_description: "wait true task".to_string(),
            })
            .await
            .expect("spawn wait");
        assert!(t1.elapsed() >= Duration::from_millis(20));
        assert_eq!(wait.worktree_paths.len(), 1);
    }

    #[tokio::test]
    async fn wait_false_background_spawn_failure_records_fleet_failed_event() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(project_root.join(".triumvirate").join("spool"))
            .expect("create spool");
        let _ = ledger::LedgerStore::open(project_root.clone()).expect("open ledger");

        let orchestrator = FleetOrchestrator::with_launcher(
            MockGitOps {
                touched: Arc::new(Mutex::new(Vec::new())),
            },
            FailingLauncher,
        );
        let spawned = orchestrator
            .fleet_spawn(FleetSpawnRequest {
                project_root: project_root.clone(),
                agents: vec!["codex".to_string()],
                dry_run: false,
                wait: Some(false),
                task_description: "failing background task".to_string(),
            })
            .await
            .expect("spawn no-wait");

        let conn = rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))
            .expect("open sqlite");
        let mut failed_events = 0_i64;
        for _ in 0..20 {
            failed_events = conn
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND event_type = 'fleet_failed'",
                    rusqlite::params![spawned.fleet_id],
                    |row| row.get(0),
                )
                .expect("count failed events");
            if failed_events >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(failed_events >= 1);
    }
}
