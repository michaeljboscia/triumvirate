use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use tracing::instrument;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BuildState {
    pub build_id: String,
    pub plan_path: String,
    pub current_wave: u32,
    pub tasks_completed: Vec<String>,
    pub tasks_remaining: Vec<String>,
    pub tasks_running: Vec<String>,
    pub tasks_failed: Vec<String>,
    pub validation_pass_rate: f64,
    pub collateral_fix_count: u32,
    pub last_commit_sha: String,
    pub wave_0_sha: String,
    pub max_parallel: u32,
    pub build_timeout_sec: Option<u64>,
    pub build_started_at: String,
    pub updated_at: String,
}

#[instrument(skip_all)]
pub fn update_state(path: &Path, state: &BuildState) -> anyhow::Result<()> {
    let payload = serde_json::to_vec_pretty(state)?;
    fs::write(path, payload)?;
    Ok(())
}

#[instrument(skip_all)]
pub fn read_state(path: &Path) -> anyhow::Result<BuildState> {
    let raw = fs::read(path)?;
    Ok(serde_json::from_slice(&raw)?)
}

#[allow(clippy::too_many_arguments)]
#[instrument(skip_all, fields(task_id = %task_id, wave = wave, status = %validation))]
pub fn append_manifest(
    path: &Path,
    task_id: &str,
    req_ids: &[String],
    commit_sha: &str,
    wave: u32,
    files_modified: &[String],
    attempts: u32,
    validation: &str,
    gemini_review: &str,
    timestamp: &str,
) -> anyhow::Result<()> {
    if !path.exists() {
        fs::write(
            path,
            b"## BUILD_MANIFEST\n\n| task_id | req_ids | wave | files_modified | attempts | commit_sha | validation | gemini_review | timestamp |\n|---|---|---|---|---|---|---|---|---|\n",
        )?;
    }
    let row = format!(
        "| {task_id} | {} | {wave} | {} | {attempts} | {commit_sha} | {validation} | {gemini_review} | {timestamp} |\n",
        req_ids.join(","),
        files_modified.join(",")
    );
    let mut existing = fs::read_to_string(path)?;
    existing.push_str(&row);
    fs::write(path, existing)?;
    Ok(())
}

#[instrument(skip_all, fields(task_id = %task_id, status = %severity))]
pub fn append_deviation(
    path: &Path,
    task_id: &str,
    severity: &str,
    summary: &str,
    classification: &str,
    timestamp: &str,
) -> anyhow::Result<()> {
    if !path.exists() {
        fs::write(path, b"## DEVIATION_LOG\n\n")?;
    }
    let entry = format!(
        "### {timestamp} - {task_id}\n- severity: {severity}\n- class: {classification}\n- summary: {summary}\n\n"
    );
    let mut existing = fs::read_to_string(path)?;
    existing.push_str(&entry);
    fs::write(path, existing)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{append_deviation, append_manifest, read_state, update_state, BuildState};

    #[test]
    fn writers_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let state_path = tmp.path().join("BUILD_STATE.json");
        let manifest_path = tmp.path().join("BUILD_MANIFEST.md");
        let deviation_path = tmp.path().join("DEVIATION_LOG.md");

        let state = BuildState {
            build_id: "abe-1".to_string(),
            plan_path: "docs/abe/IMPLEMENTATION_PLAN.md".to_string(),
            current_wave: 1,
            tasks_completed: vec!["T-001".to_string()],
            tasks_remaining: vec!["T-002".to_string()],
            tasks_running: vec![],
            tasks_failed: vec![],
            validation_pass_rate: 1.0,
            collateral_fix_count: 0,
            last_commit_sha: "abc".to_string(),
            wave_0_sha: "abc".to_string(),
            max_parallel: 2,
            build_timeout_sec: None,
            build_started_at: "2026-04-07T00:00:00Z".to_string(),
            updated_at: "2026-04-07T00:00:01Z".to_string(),
        };

        update_state(&state_path, &state).expect("update state");
        let read_back = read_state(&state_path).expect("read state");
        assert_eq!(read_back, state);

        append_manifest(
            &manifest_path,
            "T-001",
            &["REQ-A1.1".to_string()],
            "abc",
            1,
            &["src/lib.rs".to_string()],
            1,
            "PASS",
            "clean",
            "2026-04-07T00:00:02Z",
        )
        .expect("append manifest");
        let manifest = std::fs::read_to_string(&manifest_path).expect("manifest read");
        assert!(manifest.contains("T-001"));

        append_deviation(
            &deviation_path,
            "T-001",
            "info",
            "clean - no deviations",
            "none",
            "2026-04-07T00:00:03Z",
        )
        .expect("append deviation");
        let deviation = std::fs::read_to_string(&deviation_path).expect("deviation read");
        assert!(deviation.contains("clean - no deviations"));
    }
}
