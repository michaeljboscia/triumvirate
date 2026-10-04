use daemon_core::{encode_ws_event, metrics::DaemonMetrics};
use tracing::instrument;
use fleet::orchestrator::{FleetOrchestrator, FleetSpawnRequest as FleetSpawnRunRequest};
use fleet::tasks::FleetTaskStore;
use shared_types::{
    GitOps,
    FleetCancelRequest, FleetCancelResponse, FleetClaimTaskRequest, FleetClaimTaskResponse,
    FleetSpawnRequest, FleetSpawnResponse, FleetStatusRequest, FleetStatusResponse,
    FleetTaskListRequest, FleetTaskListResponse,
};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;
use tokio::sync::broadcast;

fn emit_fleet_progress(
    ws_events: Option<&broadcast::Sender<String>>,
    fleet_id: &str,
    state: &str,
    active_fleets: usize,
) {
    let Some(ws_events) = ws_events else {
        return;
    };
    let payload = serde_json::json!({
        "fleet_id": fleet_id,
        "state": state,
        "active_fleets": active_fleets,
    });
    let _ = ws_events.send(encode_ws_event("fleet_progress", payload));
}

#[instrument(skip_all)]
pub async fn fleet_spawn<G, F>(
    fleet_states: &Arc<Mutex<HashMap<String, FleetStatusResponse>>>,
    metrics: &DaemonMetrics,
    ws_events: Option<&broadcast::Sender<String>>,
    req: FleetSpawnRequest,
    orchestrator_factory: F,
) -> Result<FleetSpawnResponse, String>
where
    G: GitOps + Clone + 'static,
    F: FnOnce(PathBuf) -> Result<FleetOrchestrator<G>, String>,
{
    let project_root = req
        .project_root
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "failed to resolve project root".to_string())?;
    let agents = req
        .agents
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec!["codex".to_string(), "gemini".to_string()]);
    let dry_run = req.dry_run.unwrap_or(true);
    let wait = req.wait.unwrap_or(false);
    let task_description = req
        .task_description
        .unwrap_or_else(|| "Implement the assigned fleet task.".to_string());

    if !dry_run && fleet_temporal::engine_enabled() {
        return fleet_spawn_temporal(fleet_states, metrics, ws_events, project_root, agents, wait, task_description).await;
    }
    let mut orchestrator = orchestrator_factory(project_root.clone())?;
    if !dry_run {
        // A real fleet must be findable after a restart, so the index write is part of the spawn
        // and happens before any worker launches (inside the orchestrator).
        let index = fleet_index_path()
            .ok_or_else(|| "fleet_spawn failed: cannot resolve the triumvirate home for fleets.json".to_string())?;
        orchestrator = orchestrator.with_index_path(index);
    }
    let run = orchestrator
        .fleet_spawn(FleetSpawnRunRequest {
            project_root: project_root.clone(),
            agents: agents.clone(),
            dry_run,
            wait: Some(wait),
            task_description,
        })
        .await;
    // A fleet that FAILS to spawn (gitops error, worktree failure) is a spawn attempt worth
    // seeing; emitting only after the `?` would hide every failure and show 100% success
    // (Antigravity's survivorship catch). Emit here on error, before returning.
    if run.is_err() {
        mcp_bridge::posthog::record_fleet_spawn(
            "spawn_failed",
            dry_run,
            agents.len(),
            Some(&project_root.display().to_string()),
        );
    }
    let result = run.map_err(|e| format!("fleet_spawn failed: {e}"))?;

    let state = if dry_run {
        "planned".to_string()
    } else if wait {
        "running".to_string()
    } else {
        "spawning".to_string()
    };

    let status = FleetStatusResponse {
        fleet_id: result.fleet_id.clone(),
        state: state.clone(),
        worktree_paths: result
            .worktree_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>(),
        project_root: Some(project_root.display().to_string()),
    };
    // D-016: the in-memory map is gone after a restart, and it was the only thing that knew
    // which repo's ledger this fleet lives in. A real spawn recorded it before launching; a dry
    // run (no ledger rows, no workers) is recorded best effort, as before.
    if dry_run {
        record_fleet_root(&result.fleet_id, &project_root.display().to_string());
    }
    let mut fleet_states = fleet_states.lock().await;
    fleet_states.insert(result.fleet_id.clone(), status);
    let active = fleet_states
        .values()
        .filter(|status| status.state == "running" || status.state == "spawning")
        .count();
    metrics.fleet_active_total.set(active as i64);
    emit_fleet_progress(ws_events, &result.fleet_id, &state, active);

    // Intent-level event: a fleet was launched (or planned). The per-task tv_fleet_task
    // events only fire when a real fleet runs its agents, so without this the default
    // dry_run planning path and the "how often / how wide" question were invisible.
    mcp_bridge::posthog::record_fleet_spawn(
        &state,
        dry_run,
        agents.len(),
        Some(&project_root.display().to_string()),
    );

    Ok(FleetSpawnResponse {
        fleet_id: result.fleet_id,
        plan: result.plan_text,
        head_sha: result.head_sha,
        state,
    })
}

/// `fleet_spawn` on the Temporal engine (TRIUMVIRATE_FLEET_ENGINE=temporal). The fleet ID is
/// generated HERE, never inside workflow code (determinism). The restart index and the
/// Temporal-ownership marker are written BEFORE the workflow starts, so the fleet is findable and
/// legacy recovery leaves it alone from its first instant. A dirty repo is refused up front, as
/// the legacy spawn does, instead of failing later inside the workflow.
async fn fleet_spawn_temporal(
    fleet_states: &Arc<Mutex<HashMap<String, FleetStatusResponse>>>,
    metrics: &DaemonMetrics,
    ws_events: Option<&broadcast::Sender<String>>,
    project_root: PathBuf,
    agents: Vec<String>,
    wait: bool,
    task_description: String,
) -> Result<FleetSpawnResponse, String> {
    let fail = |e: String| {
        mcp_bridge::posthog::record_fleet_spawn("spawn_failed", false, agents.len(), Some(&project_root.display().to_string()));
        format!("fleet_spawn failed: {e}")
    };
    if !project_root.is_absolute() {
        return Err(fail("project_root must be absolute".to_string()));
    }
    let git = fleet::git_ops::RealGitOps::new(project_root.clone()).map_err(|e| fail(e.to_string()))?;
    if !git.is_clean().await.map_err(|e| fail(e.to_string()))? {
        return Err(fail("cannot create worktrees with uncommitted or dirty changes; commit or stash first".to_string()));
    }
    let head_sha = git.current_head().await.map_err(|e| fail(e.to_string()))?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| fail(e.to_string()))?
        .as_nanos();
    let fleet_id = format!("fleet-{nanos}");
    let index = fleet_index_path().ok_or_else(|| fail("cannot resolve the triumvirate home for fleets.json".to_string()))?;
    fleet::index::record_fleet_root_in(&index, &fleet_id, &project_root.display().to_string())
        .map_err(|e| fail(format!("fleet restart index could not be written: {e}")))?;
    fleet::worker_token::mark_temporal_engine(&project_root, &fleet_id)
        .map_err(|e| fail(format!("engine marker could not be written: {e}")))?;
    let cfg = fleet_temporal::WorkerConfig::from_env().map_err(|e| fail(e.to_string()))?;
    let input = fleet_temporal::fleet_workflow::FleetInput {
        fleet_id: fleet_id.clone(),
        project_root: project_root.display().to_string(),
        agents: agents.clone(),
        task_description,
    };
    let state = fleet_temporal::start_fleet(&cfg, input, wait)
        .await
        .map_err(|e| fail(format!("Temporal start failed: {e}")))?;

    let status = FleetStatusResponse {
        fleet_id: fleet_id.clone(),
        state: state.clone(),
        worktree_paths: Vec::new(),
        project_root: Some(project_root.display().to_string()),
    };
    let mut fleet_states = fleet_states.lock().await;
    fleet_states.insert(fleet_id.clone(), status);
    let active = fleet_states
        .values()
        .filter(|status| status.state == "running" || status.state == "spawning")
        .count();
    metrics.fleet_active_total.set(active as i64);
    drop(fleet_states);
    emit_fleet_progress(ws_events, &fleet_id, &state, active);
    mcp_bridge::posthog::record_fleet_spawn(&state, false, agents.len(), Some(&project_root.display().to_string()));
    Ok(FleetSpawnResponse {
        plan: format!(
            "fleet_id: {fleet_id}\nengine: temporal\nagent count: {}\nhead sha: {head_sha}\ndry_run: false",
            agents.len()
        ),
        fleet_id,
        head_sha,
        state,
    })
}

#[instrument(skip_all, fields(fleet_id = %req.fleet_id))]
pub async fn fleet_status(
    fleet_states: &Arc<Mutex<HashMap<String, FleetStatusResponse>>>,
    req: FleetStatusRequest,
) -> Result<FleetStatusResponse, String> {
    resolve_fleet(fleet_states, &req.fleet_id, fleet_index_path().as_deref()).await
}

/// The ONE way both `fleet_status` and `fleet_task_list` find a fleet (D-016).
///
/// Both used to read the in-memory map and fail with "fleet not found" on a miss. After a daemon
/// restart that map is empty, so every fleet vanished even though its ledger still held it, and
/// the map was the only record of which repo's ledger that was. They had the same bug because
/// they had the same copy of the lookup; there is now one.
///
/// Order: the map, then the daemon-level index of fleet to project root. Either way the LEDGER is
/// the truth once it has a row: the in-memory record is written once at spawn, and a no-wait
/// fleet used to report `spawning` with no worktrees for its whole life (audit, 2026-09-13).
async fn resolve_fleet(
    fleet_states: &Arc<Mutex<HashMap<String, FleetStatusResponse>>>,
    fleet_id: &str,
    index: Option<&Path>,
) -> Result<FleetStatusResponse, String> {
    let mut fleet_states = fleet_states.lock().await;
    let mut status = match fleet_states.get(fleet_id).cloned() {
        Some(s) => s,
        None => {
            let root = index
                .and_then(|i| lookup_fleet_root_in(i, fleet_id))
                .ok_or_else(|| format!("fleet not found: {fleet_id}"))?;
            FleetStatusResponse {
                fleet_id: fleet_id.to_string(),
                state: "unknown".to_string(),
                worktree_paths: Vec::new(),
                project_root: Some(root),
            }
        }
    };
    if let Some(root) = status.project_root.clone() {
        if let Some((state, paths)) = fleet::orchestrator::fleet_ledger_snapshot(Path::new(&root), fleet_id) {
            status.state = state;
            status.worktree_paths = paths.iter().map(|p| p.display().to_string()).collect();
        } else if status.state == "unknown" {
            // Found in the index but the ledger has no row: the repo moved or was wiped. Saying
            // so beats inventing a state, and beats "not found", which would be false.
            return Err(format!(
                "fleet {fleet_id} was spawned in {root}, but that ledger has no record of it"
            ));
        }
        fleet_states.insert(fleet_id.to_string(), status.clone());
    }
    Ok(status)
}

/// `{triumvirate_home}/fleets.json`: fleet id to the project root its ledger lives under.
fn fleet_index_path() -> Option<PathBuf> {
    fleet::index::fleet_index_path()
}

/// Best effort, for dry runs only. A real spawn's write is required (see `fleet_spawn`).
fn record_fleet_root(fleet_id: &str, project_root: &str) {
    let Some(index) = fleet_index_path() else { return };
    if let Err(e) = record_fleet_root_in(&index, fleet_id, project_root) {
        tracing::warn!(fleet_id, error = %e, "could not record the fleet in the restart index");
    }
}

fn record_fleet_root_in(index: &Path, fleet_id: &str, project_root: &str) -> std::io::Result<()> {
    fleet::index::record_fleet_root_in(index, fleet_id, project_root)
}

fn lookup_fleet_root_in(index: &Path, fleet_id: &str) -> Option<String> {
    fleet::index::lookup_fleet_root_in(index, fleet_id)
}

#[instrument(skip_all, fields(fleet_id = %req.fleet_id))]
pub async fn fleet_task_list(
    fleet_states: &Arc<Mutex<HashMap<String, FleetStatusResponse>>>,
    req: FleetTaskListRequest,
) -> Result<FleetTaskListResponse, String> {
    let status = resolve_fleet(fleet_states, &req.fleet_id, fleet_index_path().as_deref()).await?;
    let task_ids = status
        .worktree_paths
        .iter()
        .filter_map(|path| {
            let task_file = PathBuf::from(path)
                .join(".triumvirate")
                .join("fleet-task.md");
            let contents = fs::read_to_string(task_file).ok()?;
            contents
                .lines()
                .find_map(|line| line.strip_prefix("task_id: ").map(str::to_string))
        })
        .collect::<Vec<_>>();
    Ok(FleetTaskListResponse { task_ids })
}

#[instrument(skip_all, fields(task_id = %req.task_id))]
pub async fn fleet_claim_task(
    req: FleetClaimTaskRequest,
) -> Result<FleetClaimTaskResponse, String> {
    let project_root = req
        .project_root
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "failed to resolve project root".to_string())?;
    let store = FleetTaskStore::new(project_root)
        .map_err(|e| format!("fleet_claim_task store init failed: {e}"))?;
    let claimed = store
        .claim_task(&req.task_id, &req.assigned_agent)
        .map_err(|e| format!("fleet_claim_task failed: {e}"))?;
    Ok(FleetClaimTaskResponse { claimed })
}

#[instrument(skip_all, fields(fleet_id = %req.fleet_id))]
pub async fn fleet_cancel(
    fleet_states: &Arc<Mutex<HashMap<String, FleetStatusResponse>>>,
    metrics: &DaemonMetrics,
    ws_events: Option<&broadcast::Sender<String>>,
    req: FleetCancelRequest,
) -> Result<FleetCancelResponse, String> {
    fleet_cancel_with(
        fleet_states,
        metrics,
        ws_events,
        req,
        fleet_index_path().as_deref(),
        fleet::orchestrator::fleet_kill_grace(),
    )
    .await
}

/// Cancel that works after a restart. Before this, cancel read only the in-memory map, so after
/// a restart it returned `canceled: false`, signalled nothing, and left the ledger `running`.
async fn fleet_cancel_with(
    fleet_states: &Arc<Mutex<HashMap<String, FleetStatusResponse>>>,
    metrics: &DaemonMetrics,
    ws_events: Option<&broadcast::Sender<String>>,
    req: FleetCancelRequest,
    index: Option<&Path>,
    grace: std::time::Duration,
) -> Result<FleetCancelResponse, String> {
    // Resolve BEFORE taking the fleet_states lock: resolve_fleet takes the same mutex, so calling
    // it while holding the guard deadlocks every cancel.
    let status = match resolve_fleet(fleet_states, &req.fleet_id, index).await {
        Ok(s) => s,
        Err(e) => {
            tracing::info!(fleet_id = %req.fleet_id, error = %e, "fleet cancel: fleet not found");
            return Ok(FleetCancelResponse { canceled: false, signalled: 0, detail: Some(e) });
        }
    };
    let Some(root) = status.project_root.clone().map(PathBuf::from) else {
        return Ok(FleetCancelResponse {
            canceled: false,
            signalled: 0,
            detail: Some(format!("fleet {} has no project root on record", req.fleet_id)),
        });
    };
    let already_finished = matches!(status.state.as_str(), "done" | "failed" | "cancelled");

    // A Temporal-engine fleet is cancelled through its workflow: the workflow's own cleanup stops
    // the verified workers and marks the ledger cancelled, whichever process owns the worker.
    if fleet::worker_token::is_temporal_engine(&root, &req.fleet_id) {
        if already_finished {
            return Ok(FleetCancelResponse {
                canceled: false,
                signalled: 0,
                detail: Some(format!("fleet already {}", status.state)),
            });
        }
        let requested = match fleet_temporal::WorkerConfig::from_env() {
            Ok(cfg) => fleet_temporal::cancel_fleet(&cfg, &req.fleet_id).await.map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        let mut fleet_states = fleet_states.lock().await;
        if requested.is_ok() {
            fleet_states.remove(&req.fleet_id);
        }
        let active = fleet_states
            .values()
            .filter(|status| status.state == "running" || status.state == "spawning")
            .count();
        metrics.fleet_active_total.set(active as i64);
        drop(fleet_states);
        return Ok(match requested {
            Ok(()) => {
                emit_fleet_progress(ws_events, &req.fleet_id, "cancelled", active);
                mcp_bridge::posthog::record_fleet_spawn("cancelled", false, status.worktree_paths.len(), None);
                FleetCancelResponse {
                    canceled: true,
                    signalled: 0,
                    detail: Some(
                        "cancel requested from Temporal; its workflow stops the workers and marks the ledger cancelled"
                            .to_string(),
                    ),
                }
            }
            Err(e) => FleetCancelResponse {
                canceled: false,
                signalled: 0,
                detail: Some(format!("Temporal cancel failed: {e}")),
            },
        });
    }

    // Intent first, in memory AND in the ledger, so whichever process owns the workers sees the
    // cancel when they die and does not relaunch them as a degraded codex task.
    let in_memory = fleet::orchestrator::take_fleet_children_for_cancel(&req.fleet_id);
    let marked = already_finished
        || fleet::orchestrator::mark_fleet_cancelled(&root, &req.fleet_id, "cancelled by operator");
    // Then the processes, only through their launch tokens (SIGTERM, grace, SIGKILL, every result
    // checked). Idempotent, so it also runs for an already-finished fleet in case anything lingers.
    let report = fleet::orchestrator::stop_fleet_workers(&root, &req.fleet_id, in_memory, grace).await;

    let mut detail = Vec::new();
    if already_finished {
        detail.push(format!("fleet already {}", status.state));
    }
    if !marked {
        detail.push("the ledger could not be marked cancelled".to_string());
    }
    if !report.problems.is_empty() {
        detail.push(format!("may still be running: {}", report.problems.join("; ")));
    }
    let canceled = !already_finished && marked;
    tracing::info!(
        fleet_id = %req.fleet_id,
        canceled,
        stopped = report.stopped,
        escalated = report.escalated,
        problems = report.problems.len(),
        "fleet cancel"
    );

    let mut fleet_states = fleet_states.lock().await;
    fleet_states.remove(&req.fleet_id);
    let active = fleet_states
        .values()
        .filter(|status| status.state == "running" || status.state == "spawning")
        .count();
    metrics.fleet_active_total.set(active as i64);
    drop(fleet_states);
    if canceled {
        emit_fleet_progress(ws_events, &req.fleet_id, "cancelled", active);
        // Cancelling an in-flight fleet aborts real agent work / spend. This rides tv_fleet_spawn
        // as another point in the fleet lifecycle (tv_state=cancelled) with the fleet's real width.
        mcp_bridge::posthog::record_fleet_spawn("cancelled", false, status.worktree_paths.len(), None);
    }
    Ok(FleetCancelResponse {
        canceled,
        signalled: report.stopped,
        detail: (!detail.is_empty()).then(|| detail.join("; ")),
    })
}

#[cfg(test)]
mod restart_index_tests {
    use super::*;

    /// A real ledger holding a fleet in state `running`, the way a spawned fleet leaves it.
    fn ledger_with_running_fleet(fleet_id: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".triumvirate")).expect("mkdir");
        let _ = ledger::LedgerStore::open(root.clone()).expect("ledger");
        let store = FleetTaskStore::new(root.clone()).expect("task store");
        store.insert_fleet(fleet_id, "restart test").expect("fleet row");
        store.insert_task(&format!("{fleet_id}-T-001"), fleet_id, "t", &[]).expect("task row");
        let conn = rusqlite::Connection::open(root.join(".triumvirate").join("ledger.db")).expect("sqlite");
        conn.execute("UPDATE fleets SET state = 'running' WHERE fleet_id = ?1", rusqlite::params![fleet_id])
            .expect("set state");
        (dir, root)
    }

    /// D-016's own check: spawn, restart, `fleet_status` returns the LEDGER state. A restart is
    /// an empty in-memory map, which is exactly what this starts from.
    /// RED IF: a fleet the ledger holds is reported "not found" after a restart.
    #[tokio::test]
    async fn after_a_restart_status_is_recovered_from_the_ledger() {
        let (_dir, root) = ledger_with_running_fleet("fleet-restart");
        let idx_dir = tempfile::tempdir().expect("tempdir");
        let index = idx_dir.path().join("fleets.json");
        record_fleet_root_in(&index, "fleet-restart", &root.display().to_string()).expect("index");

        let restarted: Arc<Mutex<HashMap<String, FleetStatusResponse>>> = Arc::default();
        let status = resolve_fleet(&restarted, "fleet-restart", Some(&index))
            .await
            .expect("a fleet the ledger holds must survive a restart");
        assert_eq!(status.state, "running", "state must come from the ledger");
        assert!(
            restarted.lock().await.contains_key("fleet-restart"),
            "recovered fleets are cached, so the next call does not re-read the index"
        );
    }

    /// THE SECOND SURFACE. `fleet_task_list` had its own copy of the same map-only lookup, so a
    /// fix to `fleet_status` alone would have left it broken. It now calls the same resolver.
    /// RED IF: fleet_task_list stops resolving through the index after a restart.
    #[tokio::test]
    async fn fleet_task_list_recovers_through_the_same_resolver() {
        let (_dir, root) = ledger_with_running_fleet("fleet-list");
        let idx_dir = tempfile::tempdir().expect("tempdir");
        let index = idx_dir.path().join("fleets.json");
        record_fleet_root_in(&index, "fleet-list", &root.display().to_string()).expect("index");
        let restarted: Arc<Mutex<HashMap<String, FleetStatusResponse>>> = Arc::default();
        assert!(resolve_fleet(&restarted, "fleet-list", Some(&index)).await.is_ok());
    }

    /// Two different "not found"s must read differently: never spawned, versus spawned into a
    /// ledger that no longer has it. Reporting the second as the first would send the reader
    /// looking for a fleet that did exist.
    #[tokio::test]
    async fn an_unknown_fleet_and_a_missing_ledger_row_say_different_things() {
        let idx_dir = tempfile::tempdir().expect("tempdir");
        let index = idx_dir.path().join("fleets.json");
        let empty: Arc<Mutex<HashMap<String, FleetStatusResponse>>> = Arc::default();

        let never = resolve_fleet(&empty, "fleet-never", Some(&index)).await.unwrap_err();
        assert!(never.contains("fleet not found"), "{never}");

        let gone_root = idx_dir.path().join("gone");
        std::fs::create_dir_all(gone_root.join(".triumvirate")).expect("mkdir");
        let _ = ledger::LedgerStore::open(gone_root.clone()).expect("empty ledger");
        record_fleet_root_in(&index, "fleet-gone", &gone_root.display().to_string()).expect("index");
        let gone = resolve_fleet(&empty, "fleet-gone", Some(&index)).await.unwrap_err();
        assert!(gone.contains("has no record of it"), "{gone}");
    }

    /// The index is read-modify-write. RED IF: recording one fleet erases another.
    #[test]
    fn recording_a_fleet_keeps_every_other_fleet() {
        let idx_dir = tempfile::tempdir().expect("tempdir");
        let index = idx_dir.path().join("fleets.json");
        record_fleet_root_in(&index, "a", "/repo/a").expect("a");
        record_fleet_root_in(&index, "b", "/repo/b").expect("b");
        assert_eq!(lookup_fleet_root_in(&index, "a").as_deref(), Some("/repo/a"));
        assert_eq!(lookup_fleet_root_in(&index, "b").as_deref(), Some("/repo/b"));
    }

    /// A true orphan in its own process group (reparented to launchd), started in `cwd`.
    fn orphan(script: &str, cwd: &Path) -> u32 {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("set -m; sh -c '{script}' >/dev/null 2>&1 </dev/null & echo $!"))
            .current_dir(cwd)
            .output()
            .expect("spawn orphan");
        std::thread::sleep(std::time::Duration::from_millis(200));
        String::from_utf8_lossy(&out.stdout).trim().parse().expect("pid")
    }

    fn gone_within(pid: u32, limit: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + limit;
        while std::time::Instant::now() < deadline {
            if fleet::worker_token::proc_info(pid).is_none() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    }

    struct Reap(Vec<u32>);
    impl Drop for Reap {
        fn drop(&mut self) {
            for pid in &self.0 {
                let _ = fleet::worker_token::signal_group(*pid, 9);
            }
        }
    }

    /// Cancel after a restart: the in-memory map is empty, the workers are reachable only
    /// through their launch tokens, one of them ignores SIGTERM. Bounded by a timeout, so the
    /// old resolve-under-lock deadlock fails the test instead of hanging it.
    /// RED IF: cancel deadlocks, returns canceled:false, leaves a worker running, or leaves the
    /// ledger anything but `cancelled`.
    #[tokio::test]
    async fn cancel_after_a_restart_stops_every_worker_and_marks_the_ledger() {
        let (_dir, root) = ledger_with_running_fleet("fleet-cancel");
        let idx_dir = tempfile::tempdir().expect("tempdir");
        let index = idx_dir.path().join("fleets.json");
        record_fleet_root_in(&index, "fleet-cancel", &root.display().to_string()).expect("index");
        let wt_a = root.join(".triumvirate/worktrees/fleet-cancel-fleet-cancel-T-001-codex");
        let wt_b = root.join(".triumvirate/worktrees/fleet-cancel-fleet-cancel-T-002-codex");
        std::fs::create_dir_all(&wt_a).expect("wt a");
        std::fs::create_dir_all(&wt_b).expect("wt b");
        let polite = orphan("sleep 60", &wt_a);
        let stubborn = orphan("trap \"\" TERM; while :; do sleep 1; done", &wt_b);
        let bystander = orphan("sleep 60", &wt_a);
        let _reap = Reap(vec![polite, stubborn, bystander]);
        for (pid, task) in [(polite, "fleet-cancel-T-001"), (stubborn, "fleet-cancel-T-002")] {
            let t = fleet::worker_token::WorkerToken::for_spawned_child(pid, "fleet-cancel", task, "codex").expect("token");
            fleet::worker_token::write_token(&root, &t).expect("write token");
        }

        let restarted: Arc<Mutex<HashMap<String, FleetStatusResponse>>> = Arc::default();
        let metrics = DaemonMetrics::new().expect("metrics");
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            fleet_cancel_with(
                &restarted,
                &metrics,
                None,
                FleetCancelRequest { fleet_id: "fleet-cancel".to_string() },
                Some(&index),
                std::time::Duration::from_millis(500),
            ),
        )
        .await
        .expect("cancel must not deadlock")
        .expect("cancel");

        assert!(out.canceled, "{out:?}");
        assert_eq!(out.signalled, 2, "{out:?}");
        assert!(gone_within(polite, std::time::Duration::from_secs(3)));
        assert!(gone_within(stubborn, std::time::Duration::from_secs(3)), "SIGKILL escalation");
        assert!(fleet::worker_token::proc_info(bystander).is_some(), "a shell in the worktree without a token is never signalled");
        let state: String = rusqlite::Connection::open(root.join(".triumvirate/ledger.db"))
            .expect("db")
            .query_row("SELECT state FROM fleets WHERE fleet_id = 'fleet-cancel'", [], |r| r.get(0))
            .expect("row");
        assert_eq!(state, "cancelled");
    }

    /// The twin: an unknown fleet is a clean not-found, not an error and not a hang.
    #[tokio::test]
    async fn cancel_of_an_unknown_fleet_is_a_clean_not_found() {
        let idx_dir = tempfile::tempdir().expect("tempdir");
        let index = idx_dir.path().join("fleets.json");
        let empty: Arc<Mutex<HashMap<String, FleetStatusResponse>>> = Arc::default();
        let metrics = DaemonMetrics::new().expect("metrics");
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fleet_cancel_with(
                &empty,
                &metrics,
                None,
                FleetCancelRequest { fleet_id: "fleet-nope".to_string() },
                Some(&index),
                std::time::Duration::from_millis(100),
            ),
        )
        .await
        .expect("no hang")
        .expect("a clean response, not an error");
        assert!(!out.canceled);
        assert_eq!(out.signalled, 0);
        assert!(out.detail.as_deref().unwrap_or_default().contains("fleet not found"), "{out:?}");
    }

    /// Cancelling a fleet that already finished must not rewrite its outcome.
    #[tokio::test]
    async fn cancel_of_a_finished_fleet_keeps_its_outcome() {
        let (_dir, root) = ledger_with_running_fleet("fleet-done");
        rusqlite::Connection::open(root.join(".triumvirate/ledger.db"))
            .expect("db")
            .execute("UPDATE fleets SET state = 'done' WHERE fleet_id = 'fleet-done'", [])
            .expect("done");
        let idx_dir = tempfile::tempdir().expect("tempdir");
        let index = idx_dir.path().join("fleets.json");
        record_fleet_root_in(&index, "fleet-done", &root.display().to_string()).expect("index");
        let empty: Arc<Mutex<HashMap<String, FleetStatusResponse>>> = Arc::default();
        let metrics = DaemonMetrics::new().expect("metrics");
        let out = fleet_cancel_with(
            &empty,
            &metrics,
            None,
            FleetCancelRequest { fleet_id: "fleet-done".to_string() },
            Some(&index),
            std::time::Duration::from_millis(100),
        )
        .await
        .expect("cancel");
        assert!(!out.canceled);
        let state: String = rusqlite::Connection::open(root.join(".triumvirate/ledger.db"))
            .expect("db")
            .query_row("SELECT state FROM fleets WHERE fleet_id = 'fleet-done'", [], |r| r.get(0))
            .expect("row");
        assert_eq!(state, "done");
    }
}
