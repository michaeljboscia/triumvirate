use daemon_core::metrics::DaemonMetrics;
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

pub async fn fleet_spawn<G, F>(
    fleet_states: &Arc<Mutex<HashMap<String, FleetStatusResponse>>>,
    metrics: &DaemonMetrics,
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
    let result = orchestrator
        .fleet_spawn(FleetSpawnRunRequest {
            project_root: project_root.clone(),
            agents: agents.clone(),
            dry_run,
            wait: Some(wait),
            task_description,
        })
        .await
        .map_err(|e| format!("fleet_spawn failed: {e}"))?;

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

    Ok(FleetSpawnResponse {
        fleet_id: result.fleet_id,
        plan: result.plan_text,
        head_sha: result.head_sha,
        state,
    })
}

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

pub async fn fleet_cancel(
    fleet_states: &Arc<Mutex<HashMap<String, FleetStatusResponse>>>,
    metrics: &DaemonMetrics,
    req: FleetCancelRequest,
) -> Result<FleetCancelResponse, String> {
    let mut fleet_states = fleet_states.lock().await;
    let canceled = fleet_states.remove(&req.fleet_id).is_some();
    let active = fleet_states
        .values()
        .filter(|status| status.state == "running" || status.state == "spawning")
        .count();
    metrics.fleet_active_total.set(active as i64);
    Ok(FleetCancelResponse { canceled })
}
