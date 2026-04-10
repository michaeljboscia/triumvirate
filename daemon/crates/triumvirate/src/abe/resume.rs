use std::path::Path;

use super::build_artifacts::{read_state, BuildState};
use shared_types::TaskStatus;
use tracing::instrument;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeResult {
    pub first_incomplete_task: Option<String>,
    pub recovered_running_tasks: Vec<String>,
}

#[instrument(skip_all)]
pub fn resume_from_state<F>(state_path: &Path, mut status_lookup: F) -> anyhow::Result<(BuildState, ResumeResult)>
where
    F: FnMut(&str) -> Option<TaskStatus>,
{
    let mut state = read_state(state_path)?;
    let mut recovered = Vec::new();

    for task in state.tasks_running.clone() {
        match status_lookup(&task) {
            Some(TaskStatus::Completed) => {
                state.tasks_running.retain(|t| t != &task);
                if !state.tasks_completed.contains(&task) {
                    state.tasks_completed.push(task.clone());
                }
                state.tasks_remaining.retain(|t| t != &task);
                recovered.push(task);
            }
            Some(TaskStatus::Working) => {}
            _ => {
                state.tasks_running.retain(|t| t != &task);
                if !state.tasks_failed.contains(&task) {
                    state.tasks_failed.push(task.clone());
                }
            }
        }
    }

    let first_incomplete_task = state
        .tasks_remaining
        .iter()
        .find(|task| !state.tasks_completed.contains(task))
        .cloned();

    Ok((
        state,
        ResumeResult {
            first_incomplete_task,
            recovered_running_tasks: recovered,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::resume_from_state;
    use crate::abe::build_artifacts::{update_state, BuildState};
    use shared_types::TaskStatus;

    #[test]
    fn resume_reconciles_running_tasks() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let state_path = tmp.path().join("BUILD_STATE.json");
        let state = BuildState {
            build_id: "abe-1".to_string(),
            plan_path: "docs/abe/IMPLEMENTATION_PLAN.md".to_string(),
            current_wave: 2,
            tasks_completed: vec!["T-001".to_string()],
            tasks_remaining: vec!["T-002".to_string(), "T-003".to_string()],
            tasks_running: vec!["T-002".to_string()],
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
        update_state(&state_path, &state).expect("write state");

        let (new_state, resume) = resume_from_state(&state_path, |task| {
            if task == "T-002" {
                Some(TaskStatus::Completed)
            } else {
                None
            }
        })
        .expect("resume");

        assert!(new_state.tasks_completed.contains(&"T-002".to_string()));
        assert_eq!(resume.first_incomplete_task.as_deref(), Some("T-003"));
    }
}
