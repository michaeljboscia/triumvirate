use std::{collections::BTreeMap, fs, path::Path};

use chrono::Utc;
use daemon_core::metrics::DaemonMetrics;
use futures::stream::{FuturesUnordered, StreamExt};
use shared_types::{ContractFields, FilePolicy};
use tracing::instrument;

use super::build_artifacts::{append_deviation, append_manifest, read_state, update_state};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanTask {
    pub task_id: String,
    pub wave: u32,
    pub req: String,
    pub description: String,
    pub scope_out: String,
    pub tools: String,
    pub verify: String,
    pub contract_fields: ContractFields,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskResult {
    pub task_id: String,
    pub status: String,
    pub commit_sha: String,
    pub modified_files: Vec<String>,
    pub attempts: u32,
}

#[async_trait::async_trait]
pub trait DispatchBackend: Send + Sync {
    async fn dispatch_task(&self, task: &PlanTask) -> anyhow::Result<String>;
    async fn wait_task(&self, task_id: &str) -> anyhow::Result<TaskResult>;
}

#[instrument(skip_all)]
pub fn parse_plan(path: &Path) -> anyhow::Result<Vec<PlanTask>> {
    let content = fs::read_to_string(path)?;
    let mut tasks = Vec::new();

    let mut cursor = 0usize;
    while let Some(start_rel) = content[cursor..].find("<task ") {
        let start = cursor + start_rel;
        let Some(end_rel) = content[start..].find("</task>") else {
            anyhow::bail!("unclosed <task> block starting at byte {start}");
        };
        let end = start + end_rel + "</task>".len();
        let block = &content[start..end];
        tasks.push(parse_task_block(block)?);
        cursor = end;
    }

    tasks.sort_by(|a, b| a.wave.cmp(&b.wave).then(a.task_id.cmp(&b.task_id)));
    Ok(tasks)
}

fn parse_task_block(block: &str) -> anyhow::Result<PlanTask> {
    let first_line = block.lines().next().unwrap_or_default();
    let task_id = extract_attr(first_line, "id").unwrap_or_default();
    let wave_raw =
        extract_attr(first_line, "wave").ok_or_else(|| anyhow::anyhow!("task block missing wave attribute"))?;
    let wave = wave_raw
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("invalid wave value '{wave_raw}'"))?;
    let req = extract_attr(first_line, "req").unwrap_or_default();
    let description = extract_between(block, "<description>", "</description>").unwrap_or_default();
    let files = extract_between(block, "<files>", "</files>").unwrap_or_default();
    let scope_out = extract_between(block, "<scope_out>", "</scope_out>").unwrap_or_default();
    let tools = extract_between(block, "<tools>", "</tools>").unwrap_or_default();
    let verify = extract_between(block, "<verify>", "</verify>").unwrap_or_default();
    let reality_test = extract_between(block, "<reality_test>", "</reality_test>").unwrap_or_default();
    let done_when = extract_between(block, "<done_when>", "</done_when>").unwrap_or_default();

    if task_id.trim().is_empty() {
        anyhow::bail!("task block missing id attribute");
    }
    if req.trim().is_empty() {
        anyhow::bail!("task {task_id} is missing req attribute");
    }
    if description.trim().is_empty() {
        anyhow::bail!("task {task_id} is missing <description>");
    }
    if scope_out.trim().is_empty() {
        anyhow::bail!("task {task_id} is missing <scope_out>");
    }
    if reality_test.trim().is_empty() {
        anyhow::bail!("task {task_id} is missing <reality_test>");
    }
    if done_when.trim().is_empty() {
        anyhow::bail!("task {task_id} is missing <done_when>");
    }
    if files.trim().is_empty() {
        anyhow::bail!("task {task_id} is missing <files>");
    }
    if tools.trim().is_empty() {
        anyhow::bail!("task {task_id} is missing <tools>");
    }
    if verify.trim().is_empty() {
        anyhow::bail!("task {task_id} is missing <verify>");
    }

    let allowed_files = parse_csv_items(&files);
    let req_ids = parse_csv_items(&req);
    let allowed_commands = parse_tool_commands(&tools);
    let test_command = verify.trim().to_string();

    let contract_fields = ContractFields {
        task_id: task_id.clone(),
        req_ids,
        wave,
        file_policy: FilePolicy::DefaultDeny,
        allowed_files,
        forbidden_files: Vec::new(),
        allowed_commands,
        forbidden_commands: Vec::new(),
        commit_format: format!("^{task_id}:"),
        test_command,
        task_timeout_sec: 1_800,
        done_when,
        reality_test,
    };

    Ok(PlanTask {
        task_id,
        wave,
        req,
        description,
        scope_out,
        tools,
        verify,
        contract_fields,
    })
}

#[instrument(skip_all, fields(status = "running"))]
pub async fn run_orchestrator<B: DispatchBackend>(
    backend: &B,
    plan_path: &Path,
    state_path: &Path,
    manifest_path: &Path,
    deviation_path: &Path,
    max_parallel: usize,
) -> anyhow::Result<()> {
    run_orchestrator_with_metrics(
        backend,
        plan_path,
        state_path,
        manifest_path,
        deviation_path,
        max_parallel,
        None,
    )
    .await
}

pub async fn run_orchestrator_with_metrics<B: DispatchBackend>(
    backend: &B,
    plan_path: &Path,
    state_path: &Path,
    manifest_path: &Path,
    deviation_path: &Path,
    max_parallel: usize,
    metrics: Option<&DaemonMetrics>,
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
                state.tasks_running.push(task.task_id.clone());
                update_state(state_path, &state)?;
                running.push(async move {
                    let started = std::time::Instant::now();
                    let ticket = backend.dispatch_task(&task).await?;
                    let result = backend.wait_task(&ticket).await?;
                    Ok::<_, anyhow::Error>((task, result, started.elapsed()))
                });
            }

            let Some(outcome) = running.next().await else {
                break;
            };
            let (task, result, elapsed) = outcome?;
            if let Some(metrics) = metrics {
                metrics
                    .abe_task_duration_seconds
                    .with_label_values(&[&task.wave.to_string()])
                    .observe(elapsed.as_secs_f64());
            }
            state.tasks_running.retain(|t| t != &task.task_id);
            if result.status.eq_ignore_ascii_case("completed") {
                state.tasks_completed.push(task.task_id.clone());
                state.tasks_remaining.retain(|t| t != &task.task_id);
                let timestamp = Utc::now().to_rfc3339();
                append_manifest(
                    manifest_path,
                    &task.task_id,
                    std::slice::from_ref(&task.req),
                    &result.commit_sha,
                    task.wave,
                    &result.modified_files,
                    result.attempts,
                    "PASS",
                    "clean",
                    &timestamp,
                )?;
                append_deviation(
                    deviation_path,
                    &task.task_id,
                    "info",
                    "clean - no deviations",
                    "none",
                    &timestamp,
                )?;
            } else {
                state.tasks_failed.push(task.task_id.clone());
                let timestamp = Utc::now().to_rfc3339();
                append_deviation(
                    deviation_path,
                    &task.task_id,
                    "error",
                    "task failed",
                    "worker-error",
                    &timestamp,
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

fn extract_between(content: &str, start_tag: &str, end_tag: &str) -> Option<String> {
    let start = content.find(start_tag)? + start_tag.len();
    let remaining = &content[start..];
    let end = remaining.find(end_tag)?;
    Some(remaining[..end].trim().to_string())
}

fn parse_csv_items(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn parse_tool_commands(raw: &str) -> Vec<Vec<String>> {
    parse_csv_items(raw)
        .into_iter()
        .map(|cmd| cmd.split_whitespace().map(ToString::to_string).collect::<Vec<_>>())
        .filter(|parts| !parts.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Mutex;

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
                modified_files: vec!["src/lib.rs".to_string()],
                attempts: 1,
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
<files>mcp-server/src/abe/types.ts</files>
<scope_out>none</scope_out>
<tools>npx tsc --noEmit</tools>
<verify>npx tsc --noEmit</verify>
<reality_test>real</reality_test>
<done_when>done</done_when>
</task>
<task id="T-002" req="REQ-A1.2" wave="1" depends="T-001">
<description>World</description>
<files>mcp-server/src/abe/contract-schema.ts</files>
<scope_out>none</scope_out>
<tools>npm test</tools>
<verify>npm test</verify>
<reality_test>real</reality_test>
<done_when>done</done_when>
</task>
"#,
        )
        .expect("write plan");

        let tasks = parse_plan(&plan).expect("parse");
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].task_id, "T-001");
        assert_eq!(tasks[1].task_id, "T-002");
        assert_eq!(tasks[0].contract_fields.allowed_files, vec!["mcp-server/src/abe/types.ts"]);
        assert_eq!(
            tasks[0].contract_fields.allowed_commands,
            vec![vec!["npx".to_string(), "tsc".to_string(), "--noEmit".to_string()]]
        );
    }

    #[test]
    fn parse_plan_requires_reality_test() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plan = tmp.path().join("plan.md");
        std::fs::write(
            &plan,
            r#"<task id="T-001" req="REQ-A1.1" wave="0" depends="">
<description>Hello</description>
<files>a.rs</files>
<scope_out>x</scope_out>
<tools>cargo test</tools>
<verify>cargo test</verify>
<done_when>done</done_when>
</task>"#,
        )
        .expect("write plan");

        let err = parse_plan(&plan).expect_err("missing reality_test should fail");
        assert!(err.to_string().contains("missing <reality_test>"));
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
<files>mcp-server/src/abe/types.ts</files>
<scope_out>none</scope_out>
<tools>npx tsc --noEmit</tools>
<verify>npx tsc --noEmit</verify>
<reality_test>real</reality_test>
<done_when>done</done_when>
</task>
<task id="T-002" req="REQ-A1.2" wave="1" depends="T-001">
<description>World</description>
<files>mcp-server/src/abe/contract-schema.ts</files>
<scope_out>none</scope_out>
<tools>npm test</tools>
<verify>npm test</verify>
<reality_test>real</reality_test>
<done_when>done</done_when>
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

    struct FailOnceBackend {
        attempts: Mutex<HashMap<String, u32>>,
    }

    impl Default for FailOnceBackend {
        fn default() -> Self {
            Self {
                attempts: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl DispatchBackend for FailOnceBackend {
        async fn dispatch_task(&self, task: &PlanTask) -> anyhow::Result<String> {
            Ok(task.task_id.clone())
        }

        async fn wait_task(&self, task_id: &str) -> anyhow::Result<TaskResult> {
            let mut attempts = self
                .attempts
                .lock()
                .expect("attempt mutex poisoned");
            let count = attempts.entry(task_id.to_string()).or_insert(0);
            *count += 1;
            let n = *count;

            if task_id == "T-003" && n == 1 {
                return Ok(TaskResult {
                    task_id: task_id.to_string(),
                    status: "failed".to_string(),
                    commit_sha: String::new(),
                    modified_files: Vec::new(),
                    attempts: n,
                });
            }

            Ok(TaskResult {
                task_id: task_id.to_string(),
                status: "completed".to_string(),
                commit_sha: format!("sha-{task_id}-{n}"),
                modified_files: vec!["src/lib.rs".to_string()],
                attempts: n,
            })
        }
    }

    #[tokio::test]
    async fn orchestrator_recovers_after_failed_attempt_on_resume() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plan = tmp.path().join("plan.md");
        let state_path = tmp.path().join("BUILD_STATE.json");
        let manifest = tmp.path().join("BUILD_MANIFEST.md");
        let deviation = tmp.path().join("DEVIATION_LOG.md");

        std::fs::write(
            &plan,
            r#"<task id="T-001" req="REQ-A1.1" wave="0" depends="">
<description>First</description>
<files>a.rs</files>
<scope_out>none</scope_out>
<tools>cargo test</tools>
<verify>true</verify>
<reality_test>real</reality_test>
<done_when>done</done_when>
</task>
<task id="T-002" req="REQ-A1.2" wave="0" depends="">
<description>Second</description>
<files>b.rs</files>
<scope_out>none</scope_out>
<tools>cargo test</tools>
<verify>true</verify>
<reality_test>real</reality_test>
<done_when>done</done_when>
</task>
<task id="T-003" req="REQ-A1.3" wave="1" depends="T-002">
<description>Third</description>
<files>c.rs</files>
<scope_out>none</scope_out>
<tools>cargo test</tools>
<verify>true</verify>
<reality_test>real</reality_test>
<done_when>done</done_when>
</task>
<task id="T-004" req="REQ-A1.4" wave="1" depends="T-003">
<description>Fourth</description>
<files>d.rs</files>
<scope_out>none</scope_out>
<tools>cargo test</tools>
<verify>true</verify>
<reality_test>real</reality_test>
<done_when>done</done_when>
</task>
"#,
        )
        .expect("write plan");

        let state = BuildState {
            build_id: "abe-recovery".to_string(),
            plan_path: "plan.md".to_string(),
            current_wave: 0,
            tasks_completed: vec![],
            tasks_remaining: vec![
                "T-001".to_string(),
                "T-002".to_string(),
                "T-003".to_string(),
                "T-004".to_string(),
            ],
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
        update_state(&state_path, &state).expect("write initial state");

        let backend = FailOnceBackend::default();

        run_orchestrator(
            &backend,
            Path::new(&plan),
            Path::new(&state_path),
            Path::new(&manifest),
            Path::new(&deviation),
            2,
        )
        .await
        .expect("first run");

        let state_after_first = crate::abe::build_artifacts::read_state(&state_path)
            .expect("read state first");
        assert!(state_after_first.tasks_failed.contains(&"T-003".to_string()));
        assert!(
            state_after_first.tasks_remaining.contains(&"T-003".to_string()),
            "failed task should remain for recovery run"
        );

        run_orchestrator(
            &backend,
            Path::new(&plan),
            Path::new(&state_path),
            Path::new(&manifest),
            Path::new(&deviation),
            2,
        )
        .await
        .expect("second run");

        let final_state =
            crate::abe::build_artifacts::read_state(&state_path).expect("read final state");
        assert!(final_state.tasks_remaining.is_empty());
        assert!(final_state.tasks_completed.contains(&"T-003".to_string()));

        let manifest_body = std::fs::read_to_string(&manifest).expect("manifest read");
        assert!(manifest_body.contains("T-001"));
        assert!(manifest_body.contains("T-002"));
        assert!(manifest_body.contains("T-003"));
        assert!(manifest_body.contains("T-004"));

        let deviation_body = std::fs::read_to_string(&deviation).expect("deviation read");
        assert!(deviation_body.contains("task failed"));
        assert!(deviation_body.contains("clean - no deviations"));
    }
}
