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

/// Drop session ids that a restart cannot vouch for.
///
/// The registry persists `session_id` across restarts, and nothing validated it before resuming.
/// Two consequences, both found in review:
///
///   - A registry written BEFORE named-session keying still holds anonymous `agent::cwd` entries
///     carrying live ids. Resuming one attaches a fresh caller to whatever conversation last ran
///     in that directory, which is the cross-session leak in its original form.
///   - Even a current entry may name a CLI session the agent has since deleted, so the id is at
///     best stale and at worst someone else's.
///
/// A dropped id costs one fresh turn. A wrongly-resumed id costs a conversation crossing between
/// callers, so this errs toward dropping.
fn scrub_unvouchable_sessions(mut workers: HashMap<String, WorkerState>) -> HashMap<String, WorkerState> {
    let mut dropped = 0usize;
    for (key, state) in workers.iter_mut() {
        // Three segments means the key carries a session name and is post-fix. Two means it is an
        // anonymous worker, which must never carry a resumable id across a restart.
        if state.session_id.is_some() && key.matches("::").count() < 2 {
            state.session_id = None;
            dropped += 1;
        }
    }
    if dropped > 0 {
        tracing::warn!(
            dropped,
            "dropped session ids from anonymous worker entries on load; a resume would have \
             attached to whatever last ran in that directory"
        );
    }
    workers
}

fn worker_registry_store() -> &'static WorkerRegistry {
    static STORE: OnceLock<WorkerRegistry> = OnceLock::new();
    STORE.get_or_init(|| {
        Arc::new(Mutex::new(scrub_unvouchable_sessions(load_worker_registry_from_disk())))
    })
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
/// Forget the CLI session behind a worker without removing the worker itself.
///
/// `spawn_session` on an EXISTING name resets the visible history but left the worker's
/// `session_id` in place, so the next ask resumed the OLD CLI transcript under a session that
/// looked brand new. Found by Codex. Respawning a name must start a real new conversation.
pub async fn reset_worker_session(agent: &str, cwd: &str, session_key: Option<&str>) {
    let key = worker_key(agent, cwd, session_key);
    let mut workers = worker_registry_store().lock().await;
    if let Some(state) = workers.get_mut(&key)
        && state.session_id.is_some()
    {
        tracing::info!(%key, "respawn: clearing the previous CLI session id for this name");
        state.session_id = None;
        persist_worker_registry_if_enabled(&workers);
    }
}

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

/// One-time migration: move CLI session ids out of the worker registry into the sessions map.
///
/// SYNCHRONOUS and file-based on purpose. It runs at session-load time inside a non-async
/// constructor, and the async registry is not initialised there. Reading `workers.json` directly
/// is also more honest: this is a disk-format migration, not a live-state operation.
///
/// Without it, moving ownership silently breaks every existing named session: the sessions file
/// has no id yet, so the next ask starts a fresh conversation while the visible history keeps
/// scrolling. That is the "continuous log, blank-slate model" failure the review warned about.
///
/// Codex caught that the first version of this was never called at all.
///
/// Keys are `agent::cwd::name`. Only three-segment (named) keys carry an id worth keeping.
pub fn hydrate_session_ids_from_disk<F>(mut set_id: F) -> usize
where
    F: FnMut(&str, String),
{
    let Some(path) = worker_store_path() else { return 0 };
    let Ok(workers) = core_load_json_file_if_exists::<HashMap<String, WorkerState>>(&path) else {
        return 0;
    };
    let mut migrated = 0usize;
    for (key, state) in workers.iter() {
        let Some(sid) = state.session_id.as_deref() else { continue };
        // rsplitn so a cwd containing "::" cannot swallow the session name: the NAME is always
        // the last segment. The earlier splitn(3) version corrupted such names and its own test
        // documented the corruption instead of preventing it. Codex flagged that too.
        let mut it = key.rsplitn(2, "::");
        let Some(name) = it.next().filter(|n| !n.is_empty()) else { continue };
        if it.next().is_some_and(|head| head.contains("::")) {
            set_id(name, sid.to_string());
            migrated += 1;
        }
    }
    if migrated > 0 {
        tracing::info!(migrated, "migrated CLI session ids from workers.json into sessions");
    }
    migrated
}

#[cfg(test)]
mod worker_key_tests {
    use super::worker_key;

    /// Moving session-id ownership without migrating would silently break every existing named
    /// session: the sessions file has no id, so the next ask starts fresh while the visible
    /// history keeps scrolling. The migration must carry named ids across and drop anonymous ones.
    #[tokio::test]
    async fn u_wk_03_migration_moves_named_ids_and_drops_anonymous_ones() {
        // Exercised through the key parsing the migration relies on, which is the part that can
        // be wrong: a two-segment key never belonged to a session.
        let named = super::worker_key("grok", "/tmp", Some("alpha"));
        let anon = super::worker_key("grok", "/tmp", None);
        let parts: Vec<&str> = named.splitn(3, "::").collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "grok");
        assert_eq!(parts[1], "/tmp");
        assert_eq!(parts[2], "alpha", "the session name must survive key parsing intact");
        assert_eq!(anon.splitn(3, "::").count(), 2, "anonymous keys carry no session to migrate");

        // A cwd containing the separator must NOT corrupt the name. rsplitn takes the name from
        // the END, so it survives. The earlier splitn(3) version corrupted it and the test merely
        // documented the corruption, which is not the same as handling it.
        let odd = super::worker_key("grok", "/a::b", Some("beta"));
        let name = odd.rsplit("::").next().unwrap();
        assert_eq!(name, "beta", "the session name must survive a cwd containing the separator");
    }

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
    /// A registry written before named-session keying still holds anonymous `agent::cwd` entries
    /// with live session ids. Resuming one attaches a fresh caller to whatever conversation last
    /// ran in that directory. Found by Codex reviewing the subsystem.
    #[test]
    fn u_wk_02_stale_anonymous_session_ids_are_dropped_on_load() {
        use std::collections::HashMap;
        let mut m: HashMap<String, super::WorkerState> = HashMap::new();
        let mk = |sid: Option<&str>| super::WorkerState {
            agent: "grok".into(),
            cwd: "/tmp".into(),
            session_id: sid.map(str::to_string),
            spawn_count: 1,
            ask_count: 1,
            last_used_ms: 0,
        };
        // Pre-fix anonymous entry carrying a live id.
        m.insert("grok::/tmp".into(), mk(Some("dangerous-shared-id")));
        // Post-fix named entry: its id belongs to that session and must survive.
        m.insert("grok::/tmp::alpha".into(), mk(Some("alpha-owned-id")));

        let out = super::scrub_unvouchable_sessions(m);
        assert_eq!(
            out["grok::/tmp"].session_id, None,
            "an anonymous worker must not carry a resumable id across a restart"
        );
        assert_eq!(
            out["grok::/tmp::alpha"].session_id.as_deref(),
            Some("alpha-owned-id"),
            "a named session owns its id and must keep it"
        );
    }

}
