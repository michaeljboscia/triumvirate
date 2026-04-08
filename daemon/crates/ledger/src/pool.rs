use std::{
    collections::{HashMap, VecDeque, hash_map::Entry},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_ACTIVE_POOLS: usize = 10;
const IDLE_TTL_MS: u64 = 15 * 60 * 1000;

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolStats {
    pub active_pools: usize,
    pub queued_projects: usize,
}

#[derive(Default)]
struct PoolState {
    active: HashMap<PathBuf, u64>,
    queued: VecDeque<PathBuf>,
}

fn state() -> &'static Mutex<PoolState> {
    static STATE: OnceLock<Mutex<PoolState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(PoolState::default()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn register_activity(project_root: &Path) {
    register_activity_at(project_root, now_ms());
}

pub(crate) fn register_activity_at(project_root: &Path, now: u64) {
    reap_idle_at(now);
    let mut guard = match state().lock() {
        Ok(v) => v,
        Err(_) => return,
    };
    let project = project_root.to_path_buf();
    if let Entry::Occupied(mut entry) = guard.active.entry(project.clone()) {
        entry.insert(now);
        return;
    }
    if guard.active.len() < MAX_ACTIVE_POOLS {
        guard.active.insert(project, now);
        return;
    }
    if !guard.queued.iter().any(|p| p == &project) {
        guard.queued.push_back(project);
    }
}

pub(crate) fn reap_idle() {
    reap_idle_at(now_ms());
}

pub(crate) fn reap_idle_at(now: u64) {
    let mut guard = match state().lock() {
        Ok(v) => v,
        Err(_) => return,
    };

    guard
        .active
        .retain(|_, last_active| now.saturating_sub(*last_active) <= IDLE_TTL_MS);

    while guard.active.len() < MAX_ACTIVE_POOLS {
        let Some(next) = guard.queued.pop_front() else {
            break;
        };
        if guard.active.contains_key(&next) {
            continue;
        }
        guard.active.insert(next, now);
    }
}

#[cfg(test)]
pub(crate) fn pool_stats() -> PoolStats {
    let guard = match state().lock() {
        Ok(v) => v,
        Err(_) => {
            return PoolStats {
                active_pools: 0,
                queued_projects: 0,
            };
        }
    };
    PoolStats {
        active_pools: guard.active.len(),
        queued_projects: guard.queued.len(),
    }
}

#[cfg(test)]
pub(crate) fn reset_pool_state_for_tests() {
    let mut guard = state().lock().expect("pool state mutex poisoned");
    guard.active.clear();
    guard.queued.clear();
}
