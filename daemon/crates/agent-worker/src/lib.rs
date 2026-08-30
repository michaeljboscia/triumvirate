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
use tracing::instrument;

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

/// Identity of a cached worker.
///
/// `session_key` is the NAMED session this worker belongs to. Without it the key was
/// `(agent, cwd)` alone, so two named sessions for one agent in one directory collapsed onto a
/// single record and resumed each other's conversation. That was demonstrated live: a passphrase
/// given only to session A was returned by session B, with tools forbidden so no shared store
/// could explain it. It applied to codex and gemini identically; grok only made it visible.
///
/// `None` preserves the original behavior for one-shot `ask_agent`, which has no session of its
/// own to keep apart.
fn worker_key(agent: &str, cwd: &str, session_key: Option<&str>) -> String {
    match session_key.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => format!("{agent}::{cwd}::{name}"),
        None => format!("{agent}::{cwd}"),
    }
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

#[instrument(skip_all)]
pub async fn acquire_worker(
    agent: &str,
    cwd: &str,
    session_key: Option<&str>,
) -> WorkerAcquireResult {
    let key = worker_key(agent, cwd, session_key);
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

#[instrument(skip_all)]
pub async fn require_reused_worker(
    agent: &str,
    cwd: &str,
    session_key: Option<&str>,
) -> Result<WorkerState, String> {
    let key = worker_key(agent, cwd, session_key);
    let mut workers = worker_registry_store().lock().await;
    let now = core_unix_time_ms();
    let Some(state) = workers.get_mut(&key) else {
        // T-014 (REQ-DS-020): DeepSeek is stateless single-turn — there's no
        // remote session to reuse, so a missing worker entry is EXPECTED, not
        // an error. We still create a stub state so callers that key off
        // `worker.ask_count` get monotonic numbers.
        if agent == "deepseek" {
            let stub = WorkerState {
                agent: agent.to_string(),
                cwd: cwd.to_string(),
                session_id: None,
                spawn_count: 0,
                ask_count: 1,
                last_used_ms: now,
            };
            workers.insert(key, stub.clone());
            persist_worker_registry_if_enabled(&workers);
            return Ok(stub);
        }
        return Err(format!("worker_missing agent={agent} cwd={cwd}"));
    };
    if state.session_id.is_none() {
        // T-014 (REQ-DS-020): same as above, but for the case where a worker
        // exists from a prior consult but never carries a session_id (which
        // is the normal state for DeepSeek, since the runner returns a
        // synthetic `deepseek-<uuid>` per call but we never resume it).
        if agent == "deepseek" {
            state.ask_count = state.ask_count.saturating_add(1);
            state.last_used_ms = now;
            let out = state.clone();
            persist_worker_registry_if_enabled(&workers);
            return Ok(out);
        }
        return Err(format!("worker_missing_session agent={agent} cwd={cwd}"));
    }
    state.ask_count = state.ask_count.saturating_add(1);
    state.last_used_ms = now;
    let out = state.clone();
    persist_worker_registry_if_enabled(&workers);
    Ok(out)
}

#[instrument(skip_all)]
pub async fn update_worker_session(
    agent: &str,
    cwd: &str,
    session_key: Option<&str>,
    session_id: Option<String>,
) {
    let key = worker_key(agent, cwd, session_key);
    let mut workers = worker_registry_store().lock().await;
    if let Some(state) = workers.get_mut(&key) {
        state.session_id = session_id;
        state.last_used_ms = core_unix_time_ms();
        persist_worker_registry_if_enabled(&workers);
    }
}

#[instrument(skip_all)]
pub async fn dismiss_worker(agent: &str, cwd: &str, session_key: Option<&str>) -> bool {
    let key = worker_key(agent, cwd, session_key);
    let mut workers = worker_registry_store().lock().await;
    let removed = workers.remove(&key).is_some();
    if removed {
        persist_worker_registry_if_enabled(&workers);
    }
    removed
}

#[instrument(skip_all)]
pub fn should_invalidate_cached_session(error_text: &str) -> bool {
    let msg = error_text.to_lowercase();
    msg.contains("invalid session identifier")
        || msg.contains("error resuming session")
        || msg.contains("session not found")
        || msg.contains("unknown session")
}

#[instrument(skip_all)]
pub async fn reset_worker_registry_for_tests() {
    let mut workers = worker_registry_store().lock().await;
    workers.clear();

}

#[cfg(test)]
mod worker_key_tests {
    use super::worker_key;

    /// Cross-session contamination. Keyed on (agent, cwd) alone, two NAMED sessions for one agent
    /// in one directory shared a worker record and therefore a CLI session id, so each resumed
    /// the other's conversation.
    ///
    /// Demonstrated live before the fix: a passphrase given only to session A came back verbatim
    /// from session B, with tools explicitly forbidden so no shared store could explain it. It
    /// affected codex and gemini identically; grok only made it visible.
    #[tokio::test]
    async fn u_wk_01_named_sessions_do_not_share_a_worker() {
        let a = worker_key("grok", "/tmp", Some("alpha"));
        let b = worker_key("grok", "/tmp", Some("beta"));
        assert_ne!(a, b, "two named sessions in one cwd must not collapse onto one worker");

        // One-shot ask_agent keeps the shared record: it has no session of its own to isolate.
        let anon1 = worker_key("grok", "/tmp", None);
        let anon2 = worker_key("grok", "/tmp", None);
        assert_eq!(anon1, anon2);
        assert_ne!(anon1, a, "a named session must not reuse the anonymous worker");

        // Empty or whitespace names are treated as absent, not as a distinct session.
        assert_eq!(worker_key("grok", "/tmp", Some("")), anon1);
        assert_eq!(worker_key("grok", "/tmp", Some("   ")), anon1);

        // Still separated by agent and cwd.
        assert_ne!(a, worker_key("codex", "/tmp", Some("alpha")));
        assert_ne!(a, worker_key("grok", "/other", Some("alpha")));
    }
}
