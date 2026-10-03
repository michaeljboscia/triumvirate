//! `{triumvirate_home}/fleets.json`: fleet id to the project root its ledger lives under (D-016).
//!
//! The in-memory fleet map is gone after a restart and it was the only record of which repo's
//! ledger a fleet lives in. Restart recovery and cancel both depend on this file, so a spawn whose
//! index write fails now fails (it used to be best effort).

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub fn fleet_index_path() -> Option<PathBuf> {
    daemon_core::triumvirate_home_dir().ok().map(|h| h.join("fleets.json"))
}

fn index_lock() -> &'static std::sync::Mutex<()> {
    static L: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    L.get_or_init(|| std::sync::Mutex::new(()))
}

fn read_map(index: &Path) -> BTreeMap<String, String> {
    fs::read(index)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn record_fleet_root_in(index: &Path, fleet_id: &str, project_root: &str) -> std::io::Result<()> {
    let _guard = index_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut map = read_map(index);
    map.insert(fleet_id.to_string(), project_root.to_string());
    if let Some(dir) = index.parent() {
        fs::create_dir_all(dir)?;
    }
    // Write-then-rename, so a crash mid-write leaves the old index rather than a torn one.
    let tmp = index.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&map).map_err(std::io::Error::other)?)?;
    fs::rename(&tmp, index)
}

pub fn lookup_fleet_root_in(index: &Path, fleet_id: &str) -> Option<String> {
    let _guard = index_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    read_map(index).get(fleet_id).cloned()
}

/// Every distinct project root the index names, for startup recovery.
pub fn project_roots_in(index: &Path) -> Vec<PathBuf> {
    let _guard = index_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut roots: Vec<PathBuf> = read_map(index).into_values().map(PathBuf::from).collect();
    roots.sort();
    roots.dedup();
    roots
}
