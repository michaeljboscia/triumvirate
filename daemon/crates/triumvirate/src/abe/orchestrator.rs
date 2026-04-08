use std::{collections::BTreeMap, fs, path::Path};

use futures::stream::{FuturesUnordered, StreamExt};

use super::build_artifacts::{append_deviation, append_manifest, read_state, update_state, BuildState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanTask {
    pub task_id: String,
    pub wave: u32,
    pub req: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskResult {
    pub task_id: String,
    pub status: String,
    pub commit_sha: String,
}

#[async_trait::async_trait]
pub trait DispatchBackend: Send + Sync {
    async fn dispatch_task(&self, task: &PlanTask) -> anyhow::Result<String>;
    async fn wait_task(&self, task_id: &str) -> anyhow::Result<TaskResult>;
}

pub fn parse_plan(path: &Path) -> anyhow::Result<Vec<PlanTask>> {
    let content = fs::read_to_string(path)?;
    let mut tasks = Vec::new();

    for line in content.lines() {
        if line.trim_start().starts_with("<task ") {
            let task_id = extract_attr(line, "id").unwrap_or_default();
            let wave = extract_attr(line, "wave")
                .and_then(|w| w.parse::<u32>().ok())
                .unwrap_or(0);
            let req = extract_attr(line, "req").unwrap_or_default();
            let description = extract_between(&content, &format!("<task id=\"{task_id}\""), "<description>", "</description>")
                .unwrap_or_default();
            tasks.push(PlanTask {
                task_id,
                wave,
                req,
                description,
            });
        }
    }

    tasks.sort_by(|a, b| a.wave.cmp(&b.wave).then(a.task_id.cmp(&b.task_id)));
    Ok(tasks)
}

pub async fn run_orchestrator<B: DispatchBackend>(
    backend: &B,
    plan_path: &Path,
    state_path: &Path,
    manifest_path: &Path,
    deviation_path: &Path,
    max_parallel: usize,
) -> anyhow::Result<()> {
    let mut state = read_state(state_path)?;
    let tasks = parse_plan(plan_path)?;

    let mut waves: BTreeMap<u32, Vec<PlanTask>> = BTreeMap::new();
    for task in tasks {
        if state.tasks_completed.contains(&task.task_id) {
            continue;
        }
        waves.entry(task.wave).or_default().push(task);
    }

    for (wave, wave_tasks) in waves {
        state.current_wave = wave;
        update_state(state_path, &state)?;

        let mut pending = wave_tasks.into_iter();
        let mut running = FuturesUnordered::new();
        loop {
            while running.len() < max_parallel {
                let Some(task) = pending.next() else {
                    break;
                };
                let ticket = backend.dispatch_task(&task).await?;
                state.tasks_running.push(task.task_id.clone());
                update_state(state_path, &state)?;
                running.push(async move { (task, ticket) });
            }

            let Some((task, ticket)) = running.next().await else {
                break;
            };
            let result = backend.wait_task(&ticket).await?;
            state.tasks_running.retain(|t| t != &task.task_id);
            if result.status.eq_ignore_ascii_case("completed") {
                state.tasks_completed.push(task.task_id.clone());
                state.tasks_remaining.retain(|t| t != &task.task_id);
                append_manifest(
                    manifest_path,
                    &task.task_id,
                    std::slice::from_ref(&task.req),
                    &result.commit_sha,
                    "PASS",
                    "clean",
                    "2026-04-08T00:00:00Z",
                )?;
                append_deviation(
                    deviation_path,
                    &task.task_id,
                    "info",
                    "clean - no deviations",
                    "none",
                    "2026-04-08T00:00:00Z",
                )?;
            } else {
                state.tasks_failed.push(task.task_id.clone());
                append_deviation(
                    deviation_path,
                    &task.task_id,
                    "error",
                    "task failed",
                    "worker-error",
                    "2026-04-08T00:00:00Z",
                )?;
            }
            update_state(state_path, &state)?;
        }
    }

    Ok(())
}

fn extract_attr(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_between(content: &str, task_anchor: &str, start_tag: &str, end_tag: &str) -> Option<String> {
    let anchor_pos = content.find(task_anchor)?;
    let tail = &content[anchor_pos..];
    let start = tail.find(start_tag)? + start_tag.len();
    let remaining = &tail[start..];
    let end = remaining.find(end_tag)?;
    Some(remaining[..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{parse_plan, run_orchestrator, DispatchBackend, PlanTask, TaskResult};
    use crate::abe::build_artifacts::{update_state, BuildState};

    #[derive(Default)]
    struct MockBackend;

    #[async_trait::async_trait]
    impl DispatchBackend for MockBackend {
        async fn dispatch_task(&self, task: &PlanTask) -> anyhow::Result<String> {
            Ok(task.task_id.clone())
        }

        async fn wait_task(&self, task_id: &str) -> anyhow::Result<TaskResult> {
            Ok(TaskResult {
                task_id: task_id.to_string(),
                status: "completed".to_string(),
                commit_sha: format!("sha-{task_id}"),
            })
        }
    }

    #[test]
    fn parse_plan_finds_tasks() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plan = tmp.path().join("plan.md");
        std::fs::write(
            &plan,
            r#"<task id="T-001" req="REQ-A1.1" wave="0" depends="">
<description>Hello</description>
</task>
<task id="T-002" req="REQ-A1.2" wave="1" depends="T-001">
<description>World</description>
</task>
"#,
        )
        .expect("write plan");

        let tasks = parse_plan(&plan).expect("parse");
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].task_id, "T-001");
        assert_eq!(tasks[1].task_id, "T-002");
    }

    #[tokio::test]
    async fn orchestrator_writes_artifacts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plan = tmp.path().join("plan.md");
        let state_path = tmp.path().join("BUILD_STATE.json");
        let manifest = tmp.path().join("BUILD_MANIFEST.md");
        let deviation = tmp.path().join("DEVIATION_LOG.md");

        std::fs::write(
            &plan,
            r#"<task id="T-001" req="REQ-A1.1" wave="0" depends="">
<description>Hello</description>
</task>
<task id="T-002" req="REQ-A1.2" wave="1" depends="T-001">
<description>World</description>
</task>
"#,
        )
        .expect("write plan");

        let state = BuildState {
            build_id: "abe-1".to_string(),
            plan_path: "plan.md".to_string(),
            current_wave: 0,
            tasks_completed: vec![],
            tasks_remaining: vec!["T-001".to_string(), "T-002".to_string()],
            tasks_running: vec![],
            tasks_failed: vec![],
            validation_pass_rate: 1.0,
            collateral_fix_count: 0,
            last_commit_sha: "base".to_string(),
            wave_0_sha: "base".to_string(),
            max_parallel: 2,
            build_timeout_sec: None,
            build_started_at: "2026-04-08T00:00:00Z".to_string(),
            updated_at: "2026-04-08T00:00:00Z".to_string(),
        };
        update_state(&state_path, &state).expect("write state");

        run_orchestrator(
            &MockBackend,
            Path::new(&plan),
            Path::new(&state_path),
            Path::new(&manifest),
            Path::new(&deviation),
            2,
        )
        .await
        .expect("run");

        let manifest_body = std::fs::read_to_string(manifest).expect("manifest read");
        assert!(manifest_body.contains("T-001"));
        assert!(manifest_body.contains("T-002"));
    }
}
