use daemon_core::{
    load_json_file_if_exists as core_load_json_file_if_exists,
    persist_json_file_if_enabled as core_persist_json_file_if_enabled,
    unix_time_ms as core_unix_time_ms,
};
#[cfg(not(test))]
use daemon_core::triumvirate_home_dir as core_triumvirate_home_dir;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, OnceLock},
};
use tokio::sync::Mutex;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkerState {
    pub agent: String,
    pub cwd: String,
    pub session_id: Option<String>,
    pub spawn_count: u64,
    pub ask_count: u64,
    pub last_used_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerAcquireMode {
    Spawned,
    Reused,
}

#[derive(Debug, Clone)]
pub struct WorkerAcquireResult {
    pub mode: WorkerAcquireMode,
    pub session_id: Option<String>,
    pub spawn_count: u64,
}

type WorkerRegistry = Arc<Mutex<HashMap<String, WorkerState>>>;

fn worker_key(agent: &str, cwd: &str) -> String {
    format!("{agent}::{cwd}")
}

#[cfg(not(test))]
fn worker_store_path() -> Option<PathBuf> {
    core_triumvirate_home_dir()
        .ok()
        .map(|home| home.join("workers.json"))
}

#[cfg(test)]
fn worker_store_path() -> Option<PathBuf> {
    None
}

fn load_worker_registry_from_disk() -> HashMap<String, WorkerState> {
    worker_store_path()
        .as_ref()
        .and_then(|path| core_load_json_file_if_exists::<HashMap<String, WorkerState>>(path).ok())
        .unwrap_or_default()
}

fn persist_worker_registry_if_enabled(workers: &HashMap<String, WorkerState>) {
    if let Some(path) = worker_store_path()
        && let Err(err) = core_persist_json_file_if_enabled(Some(&path), workers)
    {
        tracing::warn!("failed to persist worker registry: {err}");
    }
}

fn worker_registry_store() -> &'static WorkerRegistry {
    static STORE: OnceLock<WorkerRegistry> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(Mutex::new(load_worker_registry_from_disk())))
}

pub async fn acquire_worker(agent: &str, cwd: &str) -> WorkerAcquireResult {
    let key = worker_key(agent, cwd);
    let mut workers = worker_registry_store().lock().await;
    let now = core_unix_time_ms();
    let state = workers.entry(key).or_insert_with(|| WorkerState {
        agent: agent.to_string(),
        cwd: cwd.to_string(),
        session_id: None,
        spawn_count: 0,
        ask_count: 0,
        last_used_ms: now,
    });
    let mode = if state.ask_count == 0 {
        WorkerAcquireMode::Spawned
    } else {
        WorkerAcquireMode::Reused
    };
    if mode == WorkerAcquireMode::Spawned {
        state.spawn_count = state.spawn_count.saturating_add(1);
    }
    state.ask_count = state.ask_count.saturating_add(1);
    state.last_used_ms = now;
    let result = WorkerAcquireResult {
        mode,
        session_id: state.session_id.clone(),
        spawn_count: state.spawn_count,
    };
    persist_worker_registry_if_enabled(&workers);
    result
}

pub async fn require_reused_worker(agent: &str, cwd: &str) -> Result<WorkerState, String> {
    let key = worker_key(agent, cwd);
    let mut workers = worker_registry_store().lock().await;
    let now = core_unix_time_ms();
    let Some(state) = workers.get_mut(&key) else {
        return Err(format!("worker_missing agent={agent} cwd={cwd}"));
    };
    if state.session_id.is_none() {
        return Err(format!("worker_missing_session agent={agent} cwd={cwd}"));
    }
    state.ask_count = state.ask_count.saturating_add(1);
    state.last_used_ms = now;
    let out = state.clone();
    persist_worker_registry_if_enabled(&workers);
    Ok(out)
}

pub async fn update_worker_session(agent: &str, cwd: &str, session_id: Option<String>) {
    let key = worker_key(agent, cwd);
    let mut workers = worker_registry_store().lock().await;
    if let Some(state) = workers.get_mut(&key) {
        state.session_id = session_id;
        state.last_used_ms = core_unix_time_ms();
        persist_worker_registry_if_enabled(&workers);
    }
}

pub async fn dismiss_worker(agent: &str, cwd: &str) -> bool {
    let key = worker_key(agent, cwd);
    let mut workers = worker_registry_store().lock().await;
    let removed = workers.remove(&key).is_some();
    if removed {
        persist_worker_registry_if_enabled(&workers);
    }
    removed
}

pub fn should_invalidate_cached_session(error_text: &str) -> bool {
    let msg = error_text.to_lowercase();
    msg.contains("invalid session identifier")
        || msg.contains("error resuming session")
        || msg.contains("session not found")
        || msg.contains("unknown session")
}

pub async fn reset_worker_registry_for_tests() {
    let mut workers = worker_registry_store().lock().await;
    workers.clear();
}
