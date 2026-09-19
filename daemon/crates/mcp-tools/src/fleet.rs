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

    let orchestrator = orchestrator_factory(project_root.clone())?;
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
    // which repo's ledger this fleet lives in. Persist that one fact so the fleet can be found.
    record_fleet_root(&result.fleet_id, &project_root.display().to_string());
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
    daemon_core::triumvirate_home_dir().ok().map(|h| h.join("fleets.json"))
}

fn index_lock() -> &'static std::sync::Mutex<()> {
    static L: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    L.get_or_init(|| std::sync::Mutex::new(()))
}

/// Best effort: a failure to write the index costs restart recovery, never the spawn itself.
fn record_fleet_root(fleet_id: &str, project_root: &str) {
    let Some(index) = fleet_index_path() else { return };
    if let Err(e) = record_fleet_root_in(&index, fleet_id, project_root) {
        tracing::warn!(fleet_id, error = %e, "could not record the fleet in the restart index");
    }
}

fn record_fleet_root_in(index: &Path, fleet_id: &str, project_root: &str) -> std::io::Result<()> {
    let _guard = index_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut map: std::collections::BTreeMap<String, String> = fs::read(index)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    map.insert(fleet_id.to_string(), project_root.to_string());
    if let Some(dir) = index.parent() {
        fs::create_dir_all(dir)?;
    }
    // Write-then-rename, so a crash mid-write leaves the old index rather than a torn one.
    let tmp = index.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&map).map_err(std::io::Error::other)?)?;
    fs::rename(&tmp, index)
}

fn lookup_fleet_root_in(index: &Path, fleet_id: &str) -> Option<String> {
    let _guard = index_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let map: std::collections::BTreeMap<String, String> = serde_json::from_slice(&fs::read(index).ok()?).ok()?;
    map.get(fleet_id).cloned()
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
    let mut fleet_states = fleet_states.lock().await;
    // Keep the removed status so the cancel event can report the fleet's ACTUAL width
    // (worktree_paths.len()) instead of a misleading zero.
    let removed = fleet_states.remove(&req.fleet_id);
    let canceled = removed.is_some();
    // Reach the processes, not only the record. Before this the workers ran on after cancel.
    let killed = fleet::orchestrator::kill_fleet_children(&req.fleet_id);
    if let Some(root) = removed.as_ref().and_then(|s| s.project_root.clone()) {
        fleet::orchestrator::mark_fleet_cancelled(
            Path::new(&root),
            &req.fleet_id,
            &format!("cancelled by operator; {killed} worker(s) signalled"),
        );
    }
    tracing::info!(fleet_id = %req.fleet_id, killed, "fleet cancel");
    let cancelled_width = removed.map(|s| s.worktree_paths.len()).unwrap_or(0);
    let active = fleet_states
        .values()
        .filter(|status| status.state == "running" || status.state == "spawning")
        .count();
    metrics.fleet_active_total.set(active as i64);
    if canceled {
        emit_fleet_progress(ws_events, &req.fleet_id, "cancelled", active);
        // Cancelling an in-flight fleet aborts real agent work / spend. Dark until now. This
        // rides tv_fleet_spawn as another point in the fleet lifecycle (tv_state=cancelled),
        // reporting the fleet's real width from the status we just removed. `canceled=false`
        // (unknown fleet_id) is not reported: nothing was aborted, so there is no work event.
        mcp_bridge::posthog::record_fleet_spawn("cancelled", false, cancelled_width, None);
    }
    Ok(FleetCancelResponse { canceled })
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
}
