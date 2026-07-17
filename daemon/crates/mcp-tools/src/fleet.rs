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
    path::PathBuf,
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
    };
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
    let fleet_states = fleet_states.lock().await;
    let status = fleet_states
        .get(&req.fleet_id)
        .cloned()
        .ok_or_else(|| format!("fleet not found: {}", req.fleet_id))?;
    Ok(status)
}

#[instrument(skip_all, fields(fleet_id = %req.fleet_id))]
pub async fn fleet_task_list(
    fleet_states: &Arc<Mutex<HashMap<String, FleetStatusResponse>>>,
    req: FleetTaskListRequest,
) -> Result<FleetTaskListResponse, String> {
    let fleet_states = fleet_states.lock().await;
    let status = fleet_states
        .get(&req.fleet_id)
        .ok_or_else(|| format!("fleet not found: {}", req.fleet_id))?;
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
