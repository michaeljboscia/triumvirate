use std::{
    fs,
    path::Path,
    path::PathBuf,
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use ledger::LedgerStore;
use shared_types::{GitOps, RawEvent};
use tokio::process::{Child, Command};

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
    /// Launch an agent subprocess in the given worktree and return the child handle.
    async fn launch(
        &self,
        agent: &str,
        project_root: &Path,
        worktree_path: &Path,
        task_prompt: &str,
    ) -> anyhow::Result<Child>;
}

#[derive(Debug, Clone, Default)]
pub struct DaemonAgentLauncher;

#[async_trait]
impl AgentLauncher for DaemonAgentLauncher {
    async fn launch(
        &self,
        agent: &str,
        project_root: &Path,
        worktree_path: &Path,
        task_prompt: &str,
    ) -> anyhow::Result<Child> {
        if cfg!(test) {
            let child = Command::new("sh")
                .arg("-lc")
                .arg("exit 0")
                .current_dir(worktree_path)
                .env("TRIUMVIRATE_PROJECT_ROOT", project_root.as_os_str())
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            return Ok(child);
        }

        let (cmd, args): (String, Vec<String>) = match agent {
            "codex" => ("codex".to_string(), fleet_codex_argv(task_prompt)),
            "gemini" => match mcp_bridge::gemini_backend() {
                // REQ-090: fleet's second Gemini site honors TRIUMVIRATE_GEMINI_BACKEND.
                // Under agy it spawns the shared sandbox-exec invocation (single-turn,
                // no resume flags — REQ-091) instead of the soon-dead `gemini` binary;
                // pipe capture is fine (agy doesn't drop over a pipe). The per-dispatch
                // profile/log temp files are reaped by the OS from the temp dir.
                mcp_bridge::GeminiBackend::Agy => {
                    let (bin, extra) = mcp_bridge::agy_command();
                    let cwd = worktree_path.to_string_lossy();
                    let inv = mcp_bridge::agy::build_agy_invocation(
                        &bin,
                        &extra,
                        task_prompt,
                        &cwd,
                        // Fleet workers WRITE code by design, so they keep the operator
                        // default. read_only is for review dispatches, where a write is
                        // never legitimate.
                        false,
                    )
                        .map_err(|e| anyhow::anyhow!("failed to assemble agy invocation for fleet: {e}"))?;
                    (inv.program, inv.args)
                }
                mcp_bridge::GeminiBackend::GeminiCli => {
                    ("gemini".to_string(), vec!["-p".to_string(), task_prompt.to_string()])
                }
            },
            // REQ-GROK-004: fleet reuses the SAME invocation builder as the consult path, so
            // a fleet worker inherits the forbidden-flag guard, the sandbox default, and the
            // session-flag rules rather than assembling a second, divergent argv.
            //
            // No session id: a fleet worker is single-turn in its own worktree, so passing one
            // would either create a session nothing resumes or, worse, resume a stranger's.
            "grok" => {
                let (bin, extra) = mcp_bridge::grok_command();
                let cwd = worktree_path.to_string_lossy();
                let inv = mcp_bridge::grok::build_grok_invocation(
                    &bin, &extra, task_prompt, &cwd, None, false,
                )
                .map_err(|e| anyhow::anyhow!("failed to assemble grok invocation for fleet: {e}"))?;
                (inv.program, inv.args)
            }
            _ => anyhow::bail!("unsupported fleet agent: {agent}"),
        };
        let mut child = Command::new(&cmd);
        let child = child
            .args(&args)
            .current_dir(worktree_path)
            .env("TRIUMVIRATE_PROJECT_ROOT", project_root.as_os_str())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Own process group, so cancel and timeout can signal the worker AND its children
        // (codex is a node wrapper around a vendor binary; SIGTERM to the wrapper alone left
        // the binary running when it was killed by hand on 2026-09-13).
        #[cfg(unix)]
        let child = child.process_group(0);
        let child = child.spawn()?;
        Ok(child)
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
            // D-015: task ids must be unique across fleets, not just within one. `tasks.task_id`
            // is the ledger's PRIMARY KEY, so the bare `T-001` made the SECOND fleet in any repo
            // die at once on `UNIQUE constraint failed: tasks.task_id`. One fleet per repo, ever.
            //
            // The fleet id goes INTO the task id rather than into a composite key, on Grok's
            // analysis of the whole change surface. A composite (fleet_id, task_id) key closes
            // only the INSERT: every `WHERE task_id = ?1` (claim, complete, four fail paths,
            // dependency checks) would then silently update EVERY fleet's `T-001` at once, the
            // MCP claim request carries no fleet_id, and `CREATE TABLE IF NOT EXISTS` would never
            // rebuild an existing ledger.db, so real repos would keep the old key. Making the
            // string unique keeps every one of those statements correct as written.
            //
            // Path-safe on purpose: a `/` here would nest directories under `Path::join`.
            // The worktree name becomes `{fleet_id}-{fleet_id}-T-001-{agent}`. That repetition is
            // ugly and CORRECT. Do not tidy it by dropping the leading `{fleet_id}-` unless the
            // formula below, `fleet_ledger_snapshot`, and recovery's prefix match change together.
            let task_id = format!("{fleet_id}-T-{:03}", idx + 1);
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
            // Launch subprocesses after all worktrees are prepared.
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

        // Launch all agent processes in parallel and monitor completion.
        let mut join_handles: Vec<(String, tokio::task::JoinHandle<()>)> = Vec::new();
        for (task_prompt, task_id, agent_name) in running_agents {
            let launcher = self.launcher.clone();
            let orchestrator = self.clone();
            let project_root = project_root.clone();
            let fleet_id = fleet_id.clone();
            let worktree_path = worktree_paths[join_handles.len()].clone();
            // Cloned BEFORE the move: the join arm needs the id to mark a panicked worker's
            // task failed, and the spawned body takes ownership of the original.
            let joined_task_id = task_id.clone();
            let jh = tokio::spawn(async move {
                tracing::info!(fleet_id = %fleet_id, task_id = %task_id, agent = %agent_name, "launching fleet agent subprocess");
                let task_started = std::time::Instant::now();
                let agy_backend = agent_name == "gemini"
                    && mcp_bridge::gemini_backend() == mcp_bridge::GeminiBackend::Agy;
                let backend_label = if agent_name == "gemini" {
                    Some(match mcp_bridge::gemini_backend() {
                        mcp_bridge::GeminiBackend::Agy => "agy",
                        mcp_bridge::GeminiBackend::GeminiCli => "gemini-cli",
                    })
                } else {
                    None
                };
                // REQ-101: honour the circuit breaker BEFORE launching. Fleet used to ignore
                // it entirely, so while the ask path correctly routed around a quota-tripped
                // agy, fleet kept hammering the same pool and starved every half-open probe
                // that would have let the breaker close. A breaker only 2 of 3 callers
                // respect is not a breaker.
                let breaker_open = agy_backend
                    && mcp_bridge::agy_resilience::agy_breaker_should_skip();
                if breaker_open {
                    tracing::warn!(
                        fleet_id = %fleet_id,
                        task_id = %task_id,
                        "agy circuit breaker OPEN — skipping agy, degrading fleet task to codex"
                    );
                    // No breaker event emitted here: agy_breaker_should_skip() already emits
                    // "blocked_call" for every caller. Emitting again would double-count
                    // fleet's shed traffic against the ask path's.
                }
                // No substitution unless the operator opted in. Launching codex here put Codex
                // in the Gemini seat and recorded it as a success.
                if breaker_open && !mcp_bridge::agy_resilience::degraded_route_allows_codex() {
                    tracing::error!(
                        fleet_id = %fleet_id,
                        task_id = %task_id,
                        agent = %agent_name,
                        "agy circuit breaker OPEN and TRIUMVIRATE_GEMINI_DEGRADED_ROUTE does not allow codex; failing task"
                    );
                    mcp_bridge::posthog::record_fleet_task(
                        &agent_name,
                        backend_label,
                        "skipped_breaker_open",
                        task_started.elapsed().as_millis() as u64,
                        &fleet_id,
                        &task_id,
                    );
                    // A silently dropped UPDATE leaves the row `in_progress` forever, and the
                    // fleet terminal check below counts it as still pending. Log it loud.
                    match rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))
                        .and_then(|conn| {
                            conn.execute(
                                "UPDATE tasks SET state = 'failed' WHERE task_id = ?1",
                                rusqlite::params![task_id.as_str()],
                            )
                        }) {
                        Ok(0) => tracing::error!(fleet_id = %fleet_id, task_id = %task_id, "breaker-blocked task row not marked failed: no row matched"),
                        Ok(_) => {}
                        Err(e) => tracing::error!(fleet_id = %fleet_id, task_id = %task_id, error = %e, "breaker-blocked task row not marked failed"),
                    }
                    if let Ok(store) = LedgerStore::open(project_root.clone()) {
                        let sequence = event_sequence_for(&project_root, &fleet_id, "task_failed").unwrap_or(1);
                        let _ = store.ingest_event(RawEvent {
                            session_id: fleet_id.clone(),
                            event_type: "task_failed".to_string(),
                            sequence,
                            timestamp: "2030-01-01T00:00:00Z".to_string(),
                            // No agent ran, so there is no answering `agent` to name. Everywhere
                            // else in this file `agent` means who ANSWERED; writing the requested
                            // seat there would claim gemini ran and failed (Codex, panel review).
                            payload_json: serde_json::json!({
                                "task_id": task_id,
                                "agent": serde_json::Value::Null,
                                "attempted_agent": agent_name,
                                "requested_agent": agent_name,
                                "error": "agy circuit breaker open; substitution disabled",
                            })
                            .to_string(),
                        });
                    }
                    // Every exit from this worker MUST pass through the fleet terminal check.
                    // Returning straight out left the fleet in `running` forever when this was
                    // the last worker to finish: no merge phase, no fleet_failed (Codex, panel
                    // review of this fix).
                    orchestrator.finalize_if_all_tasks_terminal(&fleet_id, &project_root).await;
                    return;
                }
                // Route around agy for real. Emitting "skipped" and then launching agy
                // anyway would be a lying event, which is the failure mode this whole pass
                // exists to kill. codex is a different provider (different quota pool), and
                // is already the degraded target for a failed agy task below (REQ-092).
                let launch_agent: String = if breaker_open { "codex".to_string() } else { agent_name.clone() };
                let use_agy = agy_backend && !breaker_open;
                // The backend label must describe what we LAUNCHED, not what was asked for.
                // Reporting codex-with-backend=agy would be a lie: codex has no agy backend,
                // it is a different provider on a different quota pool, which is the entire
                // reason it is the degraded target. Both reviewers caught this independently.
                let launched_backend = if breaker_open { None } else { backend_label };
                // REQ-055: hold a shared agy concurrency slot for the lifetime of this
                // child, so fleet's agy fan-out is bounded by the SAME global cap as the
                // ask path (not unbounded against the shared quota pool).
                let _agy_slot = if use_agy {
                    let slot = mcp_bridge::agy_resilience::agy_acquire_slot().await;
                    // REQ-102: fleet held the concurrency slot but NEVER took a rate-limit
                    // token, so its fan-out was concurrency-bounded and RPM-unbounded
                    // against a pool the ask path was carefully throttling. The module doc
                    // claimed these limits were global across "ask path + fleet"; for the
                    // RPM ceiling that was simply untrue until this line.
                    mcp_bridge::agy_resilience::agy_rate_limit().await;
                    Some(slot)
                } else {
                    None
                };
                // A fleet cancelled before this worker started must not start it (Codex,
                // review of step 6: cancel between the in-memory insert and the background
                // spawn returned `canceled: true` and the workers launched anyway).
                if fleet_is_cancelled(&fleet_id) {
                    tracing::warn!(fleet_id = %fleet_id, task_id = %task_id, "fleet cancelled before worker launch; skipping");
                    if let Ok(conn) = rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db")) {
                        let _ = conn.execute(
                            "UPDATE tasks SET state = 'failed' WHERE task_id = ?1",
                            rusqlite::params![task_id.as_str()],
                        );
                    }
                    // This return is a worker exit too (Grok, panel review): cancelling
                    // between the in-memory flag and the ledger write left the fleet
                    // `running` with nothing left to drive it terminal.
                    orchestrator.finalize_if_all_tasks_terminal(&fleet_id, &project_root).await;
                    return;
                }
                let launch_result = launcher
                    .launch(&launch_agent, &project_root, &worktree_path, &task_prompt)
                    .await;
                match launch_result {
                    Ok(child) => {
                        // One helper for every fleet child: drain both pipes (a worker that
                        // printed more than the pipe buffer used to block on write forever and
                        // `wait()` never returned; the audit's codex worker committed in a minute
                        // and was alive 13 minutes later), register the pid for cancel, bound
                        // the wait, kill on expiry, keep an output tail for the failure reason.
                        let (limit, timeout_msg) = if use_agy {
                            (
                                mcp_bridge::agy::agy_connector_timeout() + std::time::Duration::from_secs(30),
                                "agy fleet task exceeded connector timeout",
                            )
                        } else {
                            (fleet_task_timeout(), "fleet task exceeded TRIUMVIRATE_FLEET_TASK_TIMEOUT_SECS")
                        };
                        let (wait_result, stdout_tail, stderr_tail) =
                            wait_fleet_child(child, &fleet_id, limit, timeout_msg).await;
                        if !matches!(&wait_result, Ok(s) if s.success()) {
                            tracing::warn!(
                                fleet_id = %fleet_id,
                                task_id = %task_id,
                                stdout_tail = %stdout_tail,
                                stderr_tail = %stderr_tail,
                                "fleet agent subprocess did not succeed"
                            );
                        }
                        match wait_result {
                            Ok(status) if status.success() => {
                                tracing::info!(
                                    fleet_id = %fleet_id,
                                    task_id = %task_id,
                                    agent = %agent_name,
                                    "fleet agent subprocess completed successfully"
                                );
                                // A working agy closes the breaker for EVERY caller. Fleet
                                // never reported its successes, so its healthy traffic could
                                // not help the shared breaker recover.
                                if use_agy {
                                    mcp_bridge::agy_resilience::agy_breaker_record_success();
                                }
                                mcp_bridge::posthog::record_fleet_task(
                                    &launch_agent,
                                    launched_backend,
                                    if breaker_open { "degraded_success" } else { "success" },
                                    task_started.elapsed().as_millis() as u64,
                                    &fleet_id,
                                    &task_id,
                                );
                                if let Ok(task_store) = FleetTaskStore::new(project_root.clone()) {
                                    let _ = task_store.complete_task(&task_id);
                                }
                                if let Ok(store) = LedgerStore::open(project_root.clone()) {
                                    let seq = event_sequence_for(&project_root, &fleet_id, "task_completed")
                                        .unwrap_or(1);
                                    let _ = store.ingest_event(RawEvent {
                                        session_id: fleet_id.clone(),
                                        event_type: "task_completed".to_string(),
                                        sequence: seq,
                                        timestamp: "2030-01-01T00:00:00Z".to_string(),
                                        // The agent that DID the work, plus what was asked
                                        // for when they differ. Recording the requested agent
                                        // alone would credit gemini for a codex commit once
                                        // the breaker degrades a task, and the ledger is the
                                        // record we reason about later.
                                        payload_json: serde_json::json!({
                                            "task_id": task_id,
                                            "agent": launch_agent,
                                            "requested_agent": agent_name,
                                            "degraded_from": if breaker_open { Some(agent_name.clone()) } else { None },
                                        }).to_string(),
                                    });
                                }
                                if let Ok(review_engine) = peer_review::PeerReviewEngine::new(project_root.clone()) {
                                    // author_agent must be whoever actually wrote the code, or
                                    // peer review can hand a codex diff back to codex to review
                                    // its own work.
                                    let _ = review_engine.request_review(peer_review::ReviewRequest {
                                        fleet_id: Some(fleet_id.clone()),
                                        author_agent: launch_agent.clone(),
                                        artifact: format!("fleet/{fleet_id}/{task_id}"),
                                        review_type: "code".to_string(),
                                        // FIND-REVIEW-03: fleet queues a review for a human or
                                        // an agent to pick up over MCP. It does not conduct it
                                        // in-process, so it must stay client-writable.
                                        dispatch_owned: false,
                                    });
                                    tracing::info!(
                                        fleet_id = %fleet_id,
                                        task_id = %task_id,
                                        author_agent = %launch_agent,
                                        "peer review requested for completed task"
                                    );
                                }
                            }
                            Ok(status) => {
                                // REQ-092: degraded route — when an agy gemini task fails,
                                // degrade to codex (cross-provider) before failing loud.
                                // Fleet doesn't capture output to classify quota, so it
                                // skips the shared-pool gemini-cli and goes straight to
                                // codex, which is the safe always-available fallback.
                                // Fleet reads only an exit status, so it CANNOT tell a quota
                                // failure from a crash. Recording this as quota would be a
                                // guess that trips the shared breaker on evidence we do not
                                // have; record_other_failure is the honest arm (REQ-103
                                // biases repeated ambiguous failures toward OPEN anyway).
                                // An operator cancel is not an agy failure; it must not feed
                                // the breaker (Codex, confirmation pass).
                                if use_agy && !fleet_is_cancelled(&fleet_id) {
                                    // record_other_failure emits "tripped_other" itself, and
                                    // only on the actual transition to OPEN. Emitting here
                                    // too would report a trip on every failed task.
                                    mcp_bridge::agy_resilience::agy_breaker_record_other_failure();
                                }
                                let mut degraded_ok = false;
                                // A SIGTERM from fleet_cancel is a non-zero exit too. Without this
                                // check, cancelling an agy worker launched a codex replacement
                                // (Codex, review of step 6).
                                if use_agy
                                    && !fleet_is_cancelled(&fleet_id)
                                    && mcp_bridge::agy_resilience::degraded_route_allows_codex()
                                {
                                    tracing::warn!(
                                        fleet_id = %fleet_id,
                                        task_id = %task_id,
                                        code = status.code(),
                                        "agy fleet task failed; degrading to codex"
                                    );
                                    let codex_ok = match launcher
                                        .launch("codex", &project_root, &worktree_path, &task_prompt)
                                        .await
                                    {
                                        Ok(codex_child) => {
                                            // Same helper as the primary wait: the degrade child
                                            // was piped-and-not-drained and never registered for
                                            // cancel (Grok, review of step 6).
                                            let (r, out_tail, err_tail) = wait_fleet_child(
                                                codex_child,
                                                &fleet_id,
                                                fleet_task_timeout(),
                                                "degraded codex fleet task exceeded TRIUMVIRATE_FLEET_TASK_TIMEOUT_SECS",
                                            )
                                            .await;
                                            let ok = matches!(&r, Ok(s) if s.success());
                                            if !ok {
                                                tracing::warn!(fleet_id = %fleet_id, task_id = %task_id, stdout_tail = %out_tail, stderr_tail = %err_tail, "degraded codex fleet worker did not succeed");
                                            }
                                            ok
                                        }
                                        Err(e) => {
                                            tracing::error!(fleet_id = %fleet_id, task_id = %task_id, error = %e, "fleet codex degraded launch failed");
                                            false
                                        }
                                    };
                                    if codex_ok {
                                        degraded_ok = true;
                                        tracing::info!(fleet_id = %fleet_id, task_id = %task_id, "fleet task completed by codex (degraded from agy)");
                                        mcp_bridge::posthog::record_fleet_task(
                                            "codex",
                                            None,
                                            "degraded_success",
                                            task_started.elapsed().as_millis() as u64,
                                            &fleet_id,
                                            &task_id,
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
                                                payload_json: serde_json::json!({"task_id": task_id, "agent": "codex", "degraded_from": "agy"}).to_string(),
                                            });
                                        }
                                    }
                                }

                                if !degraded_ok {
                                    tracing::error!(
                                        fleet_id = %fleet_id,
                                        task_id = %task_id,
                                        agent = %agent_name,
                                        code = status.code(),
                                        "fleet agent subprocess failed"
                                    );
                                    mcp_bridge::posthog::record_fleet_task(
                                        &launch_agent,
                                        launched_backend,
                                        "failed",
                                        task_started.elapsed().as_millis() as u64,
                                        &fleet_id,
                                        &task_id,
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
                                            // Without the agent, a task_failed row cannot tell
                                            // a degraded codex failure from the requested
                                            // gemini failing: the two demand opposite fixes.
                                            payload_json: serde_json::json!({
                                                "task_id": task_id,
                                                "agent": launch_agent,
                                                "requested_agent": agent_name,
                                                "error": format!("agent exited with status {:?}", status.code()),
                                            })
                                            .to_string(),
                                        });
                                    }
                                }
                            }
                            Err(err) => {
                                tracing::error!(
                                    fleet_id = %fleet_id,
                                    task_id = %task_id,
                                    error = %err,
                                    "fleet agent process wait failed"
                                );
                                // This arm swallows the agy TIMEOUT synthesized above, and a
                                // timeout is a classic quota symptom (the provider stalls
                                // before it refuses). It fed neither the breaker nor
                                // PostHog, so a fleet full of agy timeouts left the breaker
                                // closed and the dashboard empty. Cause is unknowable here
                                // (fleet reads no output), hence the honest "other" arm.
                                if use_agy {
                                    mcp_bridge::agy_resilience::agy_breaker_record_other_failure();
                                }
                                mcp_bridge::posthog::record_fleet_task(
                                    &launch_agent,
                                    launched_backend,
                                    if err.kind() == std::io::ErrorKind::TimedOut { "timeout" } else { "failed" },
                                    task_started.elapsed().as_millis() as u64,
                                    &fleet_id,
                                    &task_id,
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
                                            "agent": launch_agent,
                                            "requested_agent": agent_name,
                                            "error": err.to_string()
                                        })
                                        .to_string(),
                                    });
                                }
                            }
                        }
                    }
                    Err(err) => {
                        tracing::info!(
                            fleet_id = %fleet_id,
                            task_id = %task_id,
                            error = %err,
                            "fleet agent failed to launch"
                        );
                        // "launch_failed" was documented in record_fleet_task's taxonomy and
                        // never emitted by anyone: a value that can only ever be absent. A
                        // documented-but-unreachable outcome reads as "this never happens"
                        // when it means "we never looked".
                        mcp_bridge::posthog::record_fleet_task(
                            &launch_agent,
                            launched_backend,
                            "launch_failed",
                            task_started.elapsed().as_millis() as u64,
                            &fleet_id,
                            &task_id,
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
                                    "agent": launch_agent,
                                    "requested_agent": agent_name,
                                    "error": err.to_string()
                                })
                                .to_string(),
                            });
                        }
                    }
                }

                orchestrator.finalize_if_all_tasks_terminal(&fleet_id, &project_root).await;
            });
            join_handles.push((joined_task_id, jh));
        }
        // Await all agent completions. A panicking worker body reaches NONE of its own exits,
        // so its task row stays `in_progress` and every other worker's terminal count sees it
        // as pending: the fleet never merges and never fails (Grok, panel review). The
        // JoinError is the only place that panic is visible, and it used to be discarded.
        for (task_id, jh) in join_handles {
            if let Err(join_err) = jh.await {
                tracing::error!(
                    fleet_id = %fleet_id,
                    task_id = %task_id,
                    error = %join_err,
                    "fleet worker panicked or was aborted; marking its task failed"
                );
                match rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))
                    .and_then(|conn| {
                        conn.execute(
                            "UPDATE tasks SET state = 'failed' WHERE task_id = ?1 AND state NOT IN ('done', 'failed')",
                            rusqlite::params![task_id.as_str()],
                        )
                    }) {
                    Ok(_) => {}
                    Err(e) => tracing::error!(fleet_id = %fleet_id, task_id = %task_id, error = %e, "panicked worker's task row not marked failed"),
                }
                self.finalize_if_all_tasks_terminal(&fleet_id, &project_root).await;
            }
        }
        Ok(worktree_paths)
    }

    /// Start the merge phase once no task of this fleet is still pending. Every worker exit
    /// path calls this, including the ones that never launch a child: a fleet whose last
    /// worker returns early otherwise stays `running` with no merge and no failure event.
    async fn finalize_if_all_tasks_terminal(&self, fleet_id: &str, project_root: &Path) {
        let db_path = project_root.join(".triumvirate").join("ledger.db");
        let pending = rusqlite::Connection::open(db_path)
            .and_then(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM tasks
                     WHERE fleet_id = ?1 AND state NOT IN ('done', 'failed')",
                    [fleet_id],
                    |row| row.get::<_, i64>(0),
                )
            })
            .unwrap_or_else(|e| {
                // Failing closed is right (never merge on an unknown count), but doing it
                // SILENTLY is how a fleet sits in `running` with nobody able to say why.
                tracing::error!(fleet_id = %fleet_id, error = %e, "fleet terminal count failed; treating fleet as still pending");
                1
            });
        if pending == 0 {
            tracing::info!(fleet_id = %fleet_id, "all fleet agents complete, starting merge phase");
            let _ = self.complete_fleet(fleet_id, project_root).await;
        }
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

/// Wall-clock bound for one non-agy fleet worker. `TRIUMVIRATE_FLEET_TASK_TIMEOUT_SECS`,
/// default 900s. The agy path already had its connector timeout; codex, claude and grok had
/// none, so a stuck worker was forever.
fn fleet_task_timeout() -> std::time::Duration {
    std::env::var("TRIUMVIRATE_FLEET_TASK_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(900))
}

/// Read a pipe to the end on its own task, keeping the last 8 KiB for diagnostics. A `None`
/// pipe (not captured) yields an empty tail.
fn spawn_drain<R>(reader: Option<R>) -> tokio::task::JoinHandle<String>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    use tokio::io::AsyncReadExt;
    tokio::spawn(async move {
        let Some(mut reader) = reader else {
            return String::new();
        };
        const KEEP: usize = 8 * 1024;
        let mut tail: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    tail.extend_from_slice(&buf[..n]);
                    if tail.len() > KEEP {
                        let cut = tail.len() - KEEP;
                        tail.drain(..cut);
                    }
                }
            }
        }
        String::from_utf8_lossy(&tail).into_owned()
    })
}

/// Unregisters the pid on drop, so a panic or a cancelled future between register and the
/// end of the wait cannot strand an entry in the registry (Antigravity, review of step 6).
struct FleetChildRegistration {
    fleet_id: String,
    pid: u32,
}

impl Drop for FleetChildRegistration {
    fn drop(&mut self) {
        unregister_fleet_child(&self.fleet_id, self.pid);
    }
}

/// Wait for one fleet worker: drain both pipes on their own tasks, register the pid for
/// `fleet_cancel`, bound the wait and kill on expiry. Returns the exit result and the last
/// 8 KiB of each stream. The drain awaits are bounded too: a grandchild holding the pipe
/// open must not hang the fleet after the worker has exited.
async fn wait_fleet_child(
    mut child: Child,
    fleet_id: &str,
    limit: std::time::Duration,
    timeout_msg: &str,
) -> (std::io::Result<std::process::ExitStatus>, String, String) {
    let stdout_tail = spawn_drain(child.stdout.take());
    let stderr_tail = spawn_drain(child.stderr.take());
    let _registration = child.id().map(|pid| {
        register_fleet_child(fleet_id, pid);
        // A cancel that landed between the pre-launch check and this registration would
        // otherwise leave an unregistered, unsignalled worker (Codex, confirmation pass).
        if fleet_is_cancelled(fleet_id) {
            signal_group(pid, "TERM");
        }
        FleetChildRegistration { fleet_id: fleet_id.to_string(), pid }
    });
    let wait_result = match tokio::time::timeout(limit, child.wait()).await {
        Ok(r) => r,
        Err(_) => {
            if let Some(pid) = child.id() {
                signal_group(pid, "KILL");
            }
            let _ = child.start_kill();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("{timeout_msg} ({}s)", limit.as_secs()),
            ))
        }
    };
    async fn bounded(h: tokio::task::JoinHandle<String>) -> String {
        tokio::time::timeout(std::time::Duration::from_secs(5), h)
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_default()
    }
    (wait_result, bounded(stdout_tail).await, bounded(stderr_tail).await)
}

/// Live worker pids per fleet, so `fleet_cancel` can reach the processes. Before this, cancel
/// removed the in-memory record and the workers ran on (audit, 2026-09-13: a cancelled codex
/// worker was killed by hand).
static FLEET_CHILDREN: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u32>>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

fn register_fleet_child(fleet_id: &str, pid: u32) {
    if let Ok(mut m) = FLEET_CHILDREN.lock() {
        m.entry(fleet_id.to_string()).or_default().push(pid);
    }
}

fn unregister_fleet_child(fleet_id: &str, pid: u32) {
    let Ok(mut m) = FLEET_CHILDREN.lock() else {
        return;
    };
    let Some(v) = m.get_mut(fleet_id) else {
        return;
    };
    v.retain(|p| *p != pid);
    if v.is_empty() {
        m.remove(fleet_id);
    }
}

/// Fleets the operator cancelled, so a worker not yet launched stays unlaunched and a
/// SIGTERMed agy worker is not replaced by a codex one. Process-global like the pid registry.
static CANCELLED_FLEETS: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

pub fn fleet_is_cancelled(fleet_id: &str) -> bool {
    CANCELLED_FLEETS.lock().map(|s| s.contains(fleet_id)).unwrap_or(false)
}

/// SIGTERM every live worker of `fleet_id` and remember the cancellation. Returns how many
/// were signalled.
pub fn kill_fleet_children(fleet_id: &str) -> usize {
    if let Ok(mut s) = CANCELLED_FLEETS.lock() {
        s.insert(fleet_id.to_string());
    }
    let pids: Vec<u32> = FLEET_CHILDREN
        .lock()
        .ok()
        .and_then(|mut m| m.remove(fleet_id))
        .unwrap_or_default();
    for pid in &pids {
        signal_group(*pid, "TERM");
    }
    pids.len()
}

/// Signal the worker's whole process group (the launcher puts each worker in its own).
fn signal_group(pid: u32, sig: &str) {
    let _ = std::process::Command::new("kill")
        .arg(format!("-{sig}"))
        .arg("--")
        .arg(format!("-{pid}"))
        .status();
}

/// The fleet's state as the ledger records it, with the worktrees that exist on disk for its
/// tasks. `None` when the ledger has no row. This is the truth `fleet_status` should report;
/// the in-memory record is written once at spawn.
pub fn fleet_ledger_snapshot(project_root: &Path, fleet_id: &str) -> Option<(String, Vec<PathBuf>)> {
    let db = project_root.join(".triumvirate").join("ledger.db");
    let conn = rusqlite::Connection::open(&db).ok()?;
    let state: String = conn
        .query_row("SELECT state FROM fleets WHERE fleet_id = ?1", rusqlite::params![fleet_id], |r| r.get(0))
        .ok()?;
    let mut stmt = conn
        .prepare("SELECT task_id, assigned_agent FROM tasks WHERE fleet_id = ?1 ORDER BY task_id")
        .ok()?;
    let rows = stmt
        .query_map(rusqlite::params![fleet_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .ok()?;
    let mut paths = Vec::new();
    for (task_id, agent) in rows.flatten() {
        let Some(agent) = agent else { continue };
        let p = project_root
            .join(".triumvirate")
            .join("worktrees")
            .join(format!("{fleet_id}-{task_id}-{agent}"));
        if p.exists() {
            paths.push(p);
        }
    }
    Some((state, paths))
}

/// Mark a fleet cancelled in its ledger. Returns false when the ledger could not be written.
pub fn mark_fleet_cancelled(project_root: &Path, fleet_id: &str, reason: &str) -> bool {
    let db = project_root.join(".triumvirate").join("ledger.db");
    let Ok(conn) = rusqlite::Connection::open(&db) else {
        return false;
    };
    conn.execute(
        "UPDATE fleets SET state = 'cancelled', failure_reason = ?2 WHERE fleet_id = ?1",
        rusqlite::params![fleet_id, reason],
    )
    .is_ok()
}


/// The argv a fleet member's codex is spawned with. Pure, so it can be checked against the
/// installed binary (D-011).
///
/// `codex exec` takes the prompt as a positional argument. `--message` is not a flag it accepts
/// (usage error on 0.154.0, verified 2026-09-12), so the spawn once died at argv parse before
/// running any task. `--` so a prompt that begins with a dash is a prompt, not a flag.
///
/// This was an inline tuple inside the spawn match, which is why it had no parse oracle: three
/// of four codex argv surfaces emitted a flag the binary rejected in 2026-09 and every test
/// stayed green, because the tests asserted what Triumvirate built, not what codex parses.
pub fn fleet_codex_argv(task_prompt: &str) -> Vec<String> {
    vec!["exec".to_string(), "--".to_string(), task_prompt.to_string()]
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc, time::Duration};

    use async_trait::async_trait;
    use tokio::process::{Child, Command};
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
        ) -> anyhow::Result<Child> {
            tokio::time::sleep(Duration::from_millis(25)).await;
            self.seen_project_roots
                .lock()
                .await
                .push(project_root.to_path_buf());
            let child = Command::new("sh")
                .arg("-lc")
                .arg("exit 0")
                .spawn()?;
            Ok(child)
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
        ) -> anyhow::Result<Child> {
            anyhow::bail!("launcher failure");
        }
    }

    /// Records every agent launched. gemini exits 1 (agy down); anything else exits 0.
    #[derive(Debug, Clone, Default)]
    struct GeminiDownLauncher {
        launched: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl AgentLauncher for GeminiDownLauncher {
        async fn launch(
            &self,
            agent: &str,
            _project_root: &Path,
            _worktree_path: &Path,
            _task_prompt: &str,
        ) -> anyhow::Result<Child> {
            self.launched.lock().await.push(agent.to_string());
            let code = if agent == "gemini" { "exit 1" } else { "exit 0" };
            Ok(Command::new("sh").arg("-c").arg(code).spawn()?)
        }
    }

    /// Serialises the two tests below: both set the route env and feed the global breaker.
    static ROUTE_ENV_LOCK: Mutex<()> = Mutex::const_new(());

    async fn launched_for_failing_gemini(route: Option<&str>) -> Vec<String> {
        let _lock = ROUTE_ENV_LOCK.lock().await;
        // SAFETY: serialised by ROUTE_ENV_LOCK; restored before the lock drops.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BACKEND");
            match route {
                Some(r) => std::env::set_var("TRIUMVIRATE_GEMINI_DEGRADED_ROUTE", r),
                None => std::env::remove_var("TRIUMVIRATE_GEMINI_DEGRADED_ROUTE"),
            }
        }
        mcp_bridge::agy_resilience::agy_breaker_record_success();
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(project_root.join(".triumvirate").join("spool")).expect("spool");
        let _ = ledger::LedgerStore::open(project_root.clone()).expect("open ledger");
        let launcher = GeminiDownLauncher::default();
        let launched = launcher.launched.clone();
        let orchestrator = FleetOrchestrator::with_launcher(
            MockGitOps { touched: Arc::new(Mutex::new(Vec::new())) },
            launcher,
        );
        orchestrator
            .fleet_spawn(FleetSpawnRequest {
                project_root,
                agents: vec!["gemini".to_string()],
                dry_run: false,
                wait: Some(true),
                task_description: "gemini seat task".to_string(),
            })
            .await
            .expect("spawn");
        unsafe { std::env::remove_var("TRIUMVIRATE_GEMINI_DEGRADED_ROUTE") };
        mcp_bridge::agy_resilience::agy_breaker_record_success();
        launched.lock().await.clone()
    }

    /// RED IF: a breaker-open gemini task either launches codex, or strands its fleet.
    ///
    /// `#[ignore]` because the agy breaker is PROCESS-GLOBAL. Opening it here blocked the agy
    /// task of a test running in parallel, and that test's `record_success` closed it back
    /// under this one: they failed each other, in both directions. A local lock cannot fix
    /// that, since the other tests do not take it.
    ///
    /// Both halves matter. The substitution half is the defect this pass exists to kill; the
    /// terminal half is the one the fix INTRODUCED, by returning out of the worker before the
    /// fleet completion check (Codex, panel review). A fleet left `running` never merges.
    #[tokio::test]
    #[ignore = "opens the process-global agy breaker; run with scripts/verify-live-agents.sh strict"]
    async fn breaker_open_gemini_task_launches_nobody_and_still_finishes_the_fleet() {
        let launched = breaker_open_launched(None).await;
        assert!(
            launched.is_empty(),
            "breaker open with substitution disabled must launch NOBODY, not codex: {launched:?}"
        );
    }

    /// Negative control for the test above: with the opt-in, the breaker-open path really does
    /// reach the codex launch. Without this, a `degraded_route_allows_codex()` that always
    /// returned false would leave that test green (Grok, panel review).
    #[tokio::test]
    #[ignore = "opens the process-global agy breaker; run with scripts/verify-live-agents.sh strict"]
    async fn breaker_open_gemini_task_launches_codex_only_when_opted_in() {
        let launched = breaker_open_launched(Some("codex")).await;
        assert_eq!(launched, vec!["codex"], "the opt-in must still substitute");
    }

    /// Trips the process-global breaker, runs one gemini fleet task, returns what was launched.
    /// Also asserts the fleet finished: the terminal check is the half the D-027 fix broke.
    async fn breaker_open_launched(route: Option<&str>) -> Vec<String> {
        let _lock = ROUTE_ENV_LOCK.lock().await;
        // SAFETY: serialised by ROUTE_ENV_LOCK; cleared before the lock drops.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BACKEND");
            match route {
                Some(r) => std::env::set_var("TRIUMVIRATE_GEMINI_DEGRADED_ROUTE", r),
                None => std::env::remove_var("TRIUMVIRATE_GEMINI_DEGRADED_ROUTE"),
            }
        }
        mcp_bridge::agy_resilience::agy_breaker_record_success();
        for _ in 0..8 {
            mcp_bridge::agy_resilience::agy_breaker_record_quota();
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(project_root.join(".triumvirate").join("spool")).expect("spool");
        let _ = ledger::LedgerStore::open(project_root.clone()).expect("open ledger");
        let launcher = GeminiDownLauncher::default();
        let launched = launcher.launched.clone();
        let orchestrator = FleetOrchestrator::with_launcher(
            MockGitOps { touched: Arc::new(Mutex::new(Vec::new())) },
            launcher,
        );
        orchestrator
            .fleet_spawn(FleetSpawnRequest {
                project_root: project_root.clone(),
                agents: vec!["gemini".to_string()],
                dry_run: false,
                wait: Some(true),
                task_description: "gemini seat task, breaker open".to_string(),
            })
            .await
            .expect("spawn");

        // Restore global state BEFORE any assertion can abort this helper.
        unsafe { std::env::remove_var("TRIUMVIRATE_GEMINI_DEGRADED_ROUTE") };
        mcp_bridge::agy_resilience::agy_breaker_record_success();

        let conn = rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))
            .expect("open sqlite");
        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE state NOT IN ('done', 'failed')",
                [],
                |row| row.get(0),
            )
            .expect("count pending tasks");
        assert_eq!(pending, 0, "the task must be terminal, not left in_progress");
        let fleet_state: String = conn
            .query_row("SELECT state FROM fleets LIMIT 1", [], |row| row.get(0))
            .expect("read fleet state");
        // `assert_ne!(state, "running")` passed on a fleet stuck in `merging` (Grok, panel
        // review). Name the states that mean the terminal check actually ran to completion.
        assert!(
            matches!(fleet_state.as_str(), "done" | "failed" | "merged"),
            "fleet must reach a terminal state, got `{fleet_state}`"
        );
        launched.lock().await.clone()
    }

    /// RED IF: a failed gemini fleet task relaunches as codex under the default route.
    #[tokio::test]
    async fn failed_gemini_task_is_not_relaunched_as_codex_by_default() {
        assert_eq!(launched_for_failing_gemini(None).await, vec!["gemini"]);
    }

    /// Negative control: with the opt-in, the fixture really does reach the codex relaunch.
    #[tokio::test]
    async fn failed_gemini_task_relaunches_as_codex_only_when_opted_in() {
        assert_eq!(launched_for_failing_gemini(Some("codex")).await, vec!["gemini", "codex"]);
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

        // The REAL id the spawn wrote. This used to be the literal "T-001": after D-015 that
        // names a task that does not exist and a branch nobody created, and the mock gitops
        // would still report the merge as a success. Green merge, wrong ref (Grok, D-015 review).
        let task_id = format!("{}-T-001", spawned.fleet_id);
        let tasks = FleetTaskStore::new(project_root.clone()).expect("task store");
        tasks.complete_task(&task_id).expect("complete");

        let mut merge = MergeCoordinator::new(MockGitOps {
            touched: Arc::new(Mutex::new(Vec::new())),
        })
        .with_project_root(project_root.clone());
        merge.enqueue_completed(&task_id, format!("fleet/{}/{task_id}", spawned.fleet_id));
        merge.set_review_status(&task_id, ReviewGateState::Approved, None);
        let merged = merge.merge_next().await.expect("merge");
        assert_eq!(merged.as_deref(), Some(task_id.as_str()));

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

    /// Shared setup for the D-015 class checks: one project, one ledger, a recording launcher.
    fn d015_project() -> (tempfile::TempDir, PathBuf, FleetOrchestrator<MockGitOps, RecordingLauncher>) {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(project_root.join(".triumvirate").join("spool")).expect("spool");
        let _ = ledger::LedgerStore::open(project_root.clone()).expect("open ledger");
        let orchestrator = FleetOrchestrator::with_launcher(
            MockGitOps { touched: Arc::new(Mutex::new(Vec::new())) },
            RecordingLauncher::default(),
        );
        (temp, project_root, orchestrator)
    }

    async fn d015_spawn(
        orchestrator: &FleetOrchestrator<MockGitOps, RecordingLauncher>,
        project_root: &Path,
        agents: &[&str],
    ) -> String {
        orchestrator
            .fleet_spawn(FleetSpawnRequest {
                project_root: project_root.to_path_buf(),
                agents: agents.iter().map(|a| a.to_string()).collect(),
                dry_run: false,
                wait: Some(true),
                task_description: "d015".to_string(),
            })
            .await
            .expect("a second fleet in the same repo must spawn")
            .fleet_id
    }

    fn d015_rows(project_root: &Path) -> Vec<(String, String, String)> {
        let conn = rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))
            .expect("open sqlite");
        let mut stmt = conn
            .prepare("SELECT fleet_id, task_id, state FROM tasks ORDER BY fleet_id, task_id")
            .expect("prepare");
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect()
    }

    /// D-015, THE NAMED CASE, asserted as the class: two fleets in one repo BOTH keep their rows.
    ///
    /// The defect's own check ("two consecutive spawns both reach running") is not enough on
    /// its own. The existing two-spawn test below returns before it looks at the ledger, so a
    /// fix that let the second INSERT win by overwriting the first would still pass it.
    /// RED IF: the task id loses its fleet prefix.
    #[tokio::test]
    async fn d015_two_fleets_in_one_repo_both_keep_every_row() {
        let (_temp, root, orchestrator) = d015_project();
        let a = d015_spawn(&orchestrator, &root, &["codex", "claude"]).await;
        let b = d015_spawn(&orchestrator, &root, &["codex", "claude"]).await;
        assert_ne!(a, b);

        let rows = d015_rows(&root);
        assert_eq!(rows.len(), 4, "two fleets of two tasks each must leave four rows: {rows:?}");
        for fleet in [&a, &b] {
            let mine: Vec<&(String, String, String)> = rows.iter().filter(|r| &r.0 == fleet).collect();
            assert_eq!(mine.len(), 2, "fleet {fleet} lost rows: {rows:?}");
            for r in mine {
                assert!(r.1.starts_with(&format!("{fleet}-")), "task id is not scoped to its fleet: {r:?}");
            }
        }
    }

    /// The part of the class the named check never reaches. Every mutating statement in this
    /// crate is `WHERE task_id = ?1`; if two fleets could hold the same id, finishing a task in
    /// one would silently finish it in the other.
    /// RED IF: completing fleet B's first task changes fleet A's first task.
    #[tokio::test]
    async fn d015_finishing_one_fleets_task_does_not_touch_the_other_fleet() {
        let (_temp, root, orchestrator) = d015_project();
        let a = d015_spawn(&orchestrator, &root, &["codex"]).await;
        let b = d015_spawn(&orchestrator, &root, &["codex"]).await;
        let before_a: Vec<_> = d015_rows(&root).into_iter().filter(|r| r.0 == a).collect();

        FleetTaskStore::new(root.clone())
            .expect("task store")
            .complete_task(&format!("{b}-T-001"))
            .expect("complete B's task");

        let after = d015_rows(&root);
        let after_a: Vec<_> = after.iter().filter(|r| r.0 == a).cloned().collect();
        assert_eq!(after_a, before_a, "completing fleet B's task changed fleet A: {after:?}");
        assert!(
            after.iter().any(|r| r.0 == b && r.2 == "done"),
            "and B's own task really did complete: {after:?}"
        );
    }

    /// A ledger written BEFORE this change holds a bare `T-001`. That row must not block a new
    /// fleet, and must survive it. This is the operator's real case: nobody wipes the ledger.
    /// RED IF: a legacy bare-id row collides with or is overwritten by a new fleet.
    #[tokio::test]
    async fn d015_a_ledger_with_a_legacy_bare_task_id_still_takes_a_new_fleet() {
        let (_temp, root, orchestrator) = d015_project();
        let store = FleetTaskStore::new(root.clone()).expect("task store");
        store.insert_fleet("fleet-legacy", "written before D-015").expect("legacy fleet");
        store.insert_task("T-001", "fleet-legacy", "T-001", &[]).expect("legacy bare-id row");

        let fresh = d015_spawn(&orchestrator, &root, &["codex"]).await;

        let rows = d015_rows(&root);
        assert!(rows.iter().any(|r| r.0 == "fleet-legacy" && r.1 == "T-001"), "legacy row lost: {rows:?}");
        assert!(rows.iter().any(|r| r.0 == fresh), "new fleet has no rows: {rows:?}");
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
    /// Slice H: a fleet worker must reuse the consult path's invocation builder, not assemble a
    /// second argv. Otherwise the forbidden-flag guard and the sandbox default silently do not
    /// apply to fleet work.
    #[test]
    fn u_fleet_grok_reuses_the_shared_invocation_builder() {
        let (bin, extra) = mcp_bridge::grok_command();
        let inv = mcp_bridge::grok::build_grok_invocation(
            &bin, &extra, "do the task", "/tmp/wt", None, false,
        )
        .expect("fleet must be able to build a grok invocation");
        assert!(inv.args.contains(&"--sandbox".to_string()),
            "a fleet worker must be write-contained like a consult");
        assert!(inv.args.contains(&"--output-format".to_string()));
        assert!(!inv.args.contains(&"--resume".to_string()),
            "a fleet worker is single-turn in its own worktree; resuming would attach to a stranger");
        assert!(!inv.args.contains(&"--session-id".to_string()));
        let n = inv.args.len();
        assert_eq!(inv.args[n - 2], "-p");
        assert_eq!(inv.args[n - 1], "do the task");
    }

}
