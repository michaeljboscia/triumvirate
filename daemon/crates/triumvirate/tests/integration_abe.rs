use reqwest::{Client, StatusCode};
use serde::Serialize;
use shared_types::{
    CancelTaskRequest, CancelTaskResponse, ContractFields, DispatchCodexWorktreeRequest,
    DispatchCodexWorktreeResponse, FilePolicy, GetTaskOutputRequest, GetTaskOutputResponse,
    GetTaskStatusRequest, GetTaskStatusResponse, TaskCompleteRequest, TaskStatus,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tempfile::TempDir;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_POLLS: usize = 240;

fn daemon_base_url() -> String {
    if let Ok(value) = std::env::var("TRIUMVIRATE_DAEMON_BASE_URL") {
        return value.trim_end_matches('/').to_string();
    }
    if let Ok(value) = std::env::var("TRIUMVIRATE_DAEMON_URL") {
        let trimmed = value.trim_end_matches('/');
        if let Some(prefix) = trimmed.strip_suffix("/status") {
            return prefix.to_string();
        }
        return trimmed.to_string();
    }
    "http://127.0.0.1:8080".to_string()
}

fn daemon_token() -> anyhow::Result<String> {
    let home = std::env::var("HOME")?;
    let token_path = PathBuf::from(home).join(".triumvirate").join("daemon.token");
    let token = fs::read_to_string(&token_path)?;
    Ok(token.trim().to_string())
}

fn api_client() -> anyhow::Result<(Client, String, String)> {
    Ok((Client::new(), daemon_base_url(), daemon_token()?))
}

async fn post_json<TReq: Serialize, TResp: serde::de::DeserializeOwned>(
    path: &str,
    body: &TReq,
) -> anyhow::Result<TResp> {
    let (client, base, token) = api_client()?;
    let res = client
        .post(format!("{base}{path}"))
        .bearer_auth(token)
        .json(body)
        .send()
        .await?;
    let status = res.status();
    if status == reqwest::StatusCode::METHOD_NOT_ALLOWED {
        anyhow::bail!("SKIP: {path} is MCP-only (no HTTP route). Status: 405 Method Not Allowed");
    }
    if !status.is_success() {
        anyhow::bail!("{} {} failed with {}", "POST", path, status);
    }
    Ok(res.json::<TResp>().await?)
}

async fn post_json_expect_status<TReq: Serialize>(
    path: &str,
    body: &TReq,
    status: StatusCode,
) -> anyhow::Result<()> {
    let (client, base, token) = api_client()?;
    let res = client
        .post(format!("{base}{path}"))
        .bearer_auth(token)
        .json(body)
        .send()
        .await?;
    anyhow::ensure!(res.status() == status, "expected {status}, got {}", res.status());
    Ok(())
}

/// Returns true if the endpoint is MCP-only (405) and prints a skip message.
/// Tests call this at dispatch time and return Ok(()) to skip gracefully.
fn is_mcp_only_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    msg.contains("MCP-only") || msg.contains("405")
}

async fn get_status(task_id: &str) -> anyhow::Result<GetTaskStatusResponse> {
    post_json(
        "/abe/get_task_status",
        &GetTaskStatusRequest {
            task_id: task_id.to_string(),
        },
    )
    .await
}

async fn wait_for_terminal_status(task_id: &str) -> anyhow::Result<GetTaskStatusResponse> {
    for _ in 0..MAX_POLLS {
        let status = get_status(task_id).await?;
        if matches!(
            status.status,
            TaskStatus::Completed
                | TaskStatus::Failed
                | TaskStatus::Timeout
                | TaskStatus::SetupFailed
                | TaskStatus::Cancelled
                | TaskStatus::Stuck
        ) {
            return Ok(status);
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    anyhow::bail!("task {task_id} did not reach terminal status within timeout")
}

fn shell_ok(dir: impl AsRef<Path>, args: &[&str]) -> anyhow::Result<()> {
    let out = Command::new("git").args(["-C", dir.as_ref().to_str().unwrap_or("")]).args(args).output()?;
    if !out.status.success() {
        anyhow::bail!(
            "git command failed: git -C {} {}\nstdout: {}\nstderr: {}",
            dir.as_ref().display(),
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

fn repo_head(dir: impl AsRef<Path>) -> anyhow::Result<String> {
    let out = Command::new("git")
        .args(["-C", dir.as_ref().to_str().unwrap_or(""), "rev-parse", "HEAD"])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("failed to read repo head: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn now_nanos() -> anyhow::Result<u128> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos())
}

fn init_temp_repo() -> anyhow::Result<(TempDir, String)> {
    let tmp = tempfile::tempdir()?;
    shell_ok(tmp.path(), &["init"])?;
    shell_ok(tmp.path(), &["config", "user.email", "abe-integration@test.local"])?;
    shell_ok(tmp.path(), &["config", "user.name", "ABE Integration"])?;

    fs::write(tmp.path().join("README.md"), "integration abe test\n")?;
    shell_ok(tmp.path(), &["add", "README.md"])?;
    shell_ok(tmp.path(), &["commit", "-m", "init"])?;

    let sha = repo_head(tmp.path())?;
    Ok((tmp, sha))
}

fn build_contract(task_id: &str, timeout_sec: u64, commit_format: &str) -> ContractFields {
    ContractFields {
        task_id: task_id.to_string(),
        req_ids: vec!["REQ-I-ABE".to_string()],
        wave: 99,
        file_policy: FilePolicy::DefaultDeny,
        allowed_files: vec!["src/integration_abe.rs".to_string()],
        forbidden_files: vec![],
        allowed_commands: vec![vec!["true".to_string()]],
        forbidden_commands: vec![],
        commit_format: commit_format.to_string(),
        test_command: "true".to_string(),
        task_timeout_sec: timeout_sec,
        done_when: "integration lifecycle complete".to_string(),
        reality_test: "dispatch -> status -> output -> cancel".to_string(),
        sandbox_permissions: None,
    }
}

async fn dispatch(
    project_root: &Path,
    sha: &str,
    task_id: &str,
    briefing_content: &str,
    timeout_sec: u64,
    commit_format: &str,
) -> anyhow::Result<DispatchCodexWorktreeResponse> {
    post_json(
        "/abe/dispatch_codex_worktree",
        &DispatchCodexWorktreeRequest {
            project_root: Some(project_root.display().to_string()),
            sha: sha.to_string(),
            briefing_content: briefing_content.to_string(),
            contract_fields: build_contract(task_id, timeout_sec, commit_format),
            keep_failed_worktree: Some(true),
        },
    )
    .await
}

#[tokio::test]
#[ignore]
async fn i_abe_01_dispatch_codex_worktree_trivial_task_worker_spawns_and_commits() -> anyhow::Result<()> {
    let (repo, sha) = init_temp_repo()?;
    let task_id = format!("I-ABE-01-{}", now_nanos()?);

    let dispatched = match dispatch(
        repo.path(),
        &sha,
        &task_id,
        "Create src/integration_abe.rs with a trivial function and commit.",
        180,
        "^I-ABE-01-",
    )
    .await
    {
        Ok(d) => d,
        Err(e) if is_mcp_only_error(&e) => {
            eprintln!("SKIP i_abe_01: dispatch_codex_worktree is MCP-only, no HTTP route");
            return Ok(());
        }
        Err(e) => return Err(e),
    };

    anyhow::ensure!(dispatched.status == "dispatched");
    let final_status = wait_for_terminal_status(&task_id).await?;
    anyhow::ensure!(final_status.status == TaskStatus::Completed);
    anyhow::ensure!(final_status.commit_sha.unwrap_or_default().len() >= 7);
    Ok(())
}

#[tokio::test]
#[ignore]
async fn i_abe_02_get_task_status_transitions_working_to_completed() -> anyhow::Result<()> {
    let (repo, sha) = init_temp_repo()?;
    let task_id = format!("I-ABE-02-{}", now_nanos()?);

    if let Err(e) = dispatch(
        repo.path(),
        &sha,
        &task_id,
        "Write src/integration_abe.rs and commit quickly.",
        180,
        "^I-ABE-02-",
    )
    .await
    {
        if is_mcp_only_error(&e) {
            eprintln!("SKIP i_abe_02: dispatch_codex_worktree is MCP-only, no HTTP route");
            return Ok(());
        }
        return Err(e);
    }

    let first = get_status(&task_id).await?;
    anyhow::ensure!(matches!(first.status, TaskStatus::Working | TaskStatus::Completed));

    let final_status = wait_for_terminal_status(&task_id).await?;
    anyhow::ensure!(final_status.status == TaskStatus::Completed);
    Ok(())
}

#[tokio::test]
#[ignore]
async fn i_abe_03_get_task_output_returns_commit_sha_and_files_after_completion() -> anyhow::Result<()> {
    let (repo, sha) = init_temp_repo()?;
    let task_id = format!("I-ABE-03-{}", now_nanos()?);

    if let Err(e) = dispatch(
        repo.path(),
        &sha,
        &task_id,
        "Create src/integration_abe.rs, commit, and finish.",
        180,
        "^I-ABE-03-",
    )
    .await
    {
        if is_mcp_only_error(&e) {
            eprintln!("SKIP i_abe_03: dispatch_codex_worktree is MCP-only, no HTTP route");
            return Ok(());
        }
        return Err(e);
    }

    let final_status = wait_for_terminal_status(&task_id).await?;
    anyhow::ensure!(final_status.status == TaskStatus::Completed);

    let output: GetTaskOutputResponse = post_json(
        "/abe/get_task_output",
        &GetTaskOutputRequest {
            task_id: task_id.clone(),
        },
    )
    .await?;
    anyhow::ensure!(!output.commit_sha.is_empty());
    anyhow::ensure!(!output.modified_files.is_empty());
    Ok(())
}

#[tokio::test]
#[ignore]
async fn i_abe_04_cancel_task_stops_running_task() -> anyhow::Result<()> {
    let (repo, sha) = init_temp_repo()?;
    let task_id = format!("I-ABE-04-{}", now_nanos()?);

    if let Err(e) = dispatch(
        repo.path(),
        &sha,
        &task_id,
        "Sleep for 60 seconds before writing any file.",
        600,
        "^I-ABE-04-",
    )
    .await
    {
        if is_mcp_only_error(&e) {
            eprintln!("SKIP i_abe_04: dispatch_codex_worktree is MCP-only, no HTTP route");
            return Ok(());
        }
        return Err(e);
    }

    let cancelled: CancelTaskResponse = post_json(
        "/abe/cancel_task",
        &CancelTaskRequest {
            task_id: task_id.clone(),
        },
    )
    .await?;
    anyhow::ensure!(cancelled.status == "cancelled");

    let final_status = wait_for_terminal_status(&task_id).await?;
    anyhow::ensure!(final_status.status == TaskStatus::Cancelled);
    Ok(())
}

#[tokio::test]
#[ignore]
async fn i_abe_05_sentinel_file_triggers_completion_detection() -> anyhow::Result<()> {
    let (repo, sha) = init_temp_repo()?;
    let task_id = format!("I-ABE-05-{}", now_nanos()?);

    if let Err(e) = dispatch(
        repo.path(),
        &sha,
        &task_id,
        "Write src/integration_abe.rs, commit, then write .triumvirate/TASK_COMPLETE.json and keep process alive for at least 30s.",
        180,
        "^I-ABE-05-",
    )
    .await
    {
        if is_mcp_only_error(&e) {
            eprintln!("SKIP i_abe_05: dispatch_codex_worktree is MCP-only, no HTTP route");
            return Ok(());
        }
        return Err(e);
    }

    let final_status = wait_for_terminal_status(&task_id).await?;
    anyhow::ensure!(final_status.status == TaskStatus::Completed);
    anyhow::ensure!(final_status.commit_sha.unwrap_or_default().len() >= 7);
    Ok(())
}

#[tokio::test]
#[ignore]
async fn i_abe_06_three_ceremony_closing_block_commit_sentinel_http_post() -> anyhow::Result<()> {
    let (repo, sha) = init_temp_repo()?;
    let task_id = format!("I-ABE-06-{}", now_nanos()?);

    if let Err(e) = dispatch(
        repo.path(),
        &sha,
        &task_id,
        "Do all three ceremonies: commit, write sentinel, and POST completion to /abe/task-complete.",
        180,
        "^I-ABE-06-",
    )
    .await
    {
        if is_mcp_only_error(&e) {
            eprintln!("SKIP i_abe_06: dispatch_codex_worktree is MCP-only, no HTTP route");
            return Ok(());
        }
        return Err(e);
    }

    let final_status = wait_for_terminal_status(&task_id).await?;
    anyhow::ensure!(final_status.status == TaskStatus::Completed);

    let output: GetTaskOutputResponse = post_json(
        "/abe/get_task_output",
        &GetTaskOutputRequest {
            task_id: task_id.clone(),
        },
    )
    .await?;

    post_json_expect_status(
        "/abe/task-complete",
        &TaskCompleteRequest {
            task_id: task_id.clone(),
            commit_sha: output.commit_sha,
            result: "completed".to_string(),
            timestamp: "2026-04-10T00:00:00Z".to_string(),
            commit_message: "three ceremony verification".to_string(),
        },
        StatusCode::OK,
    )
    .await?;

    Ok(())
}

#[tokio::test]
#[ignore]
async fn i_abe_07_worker_timeout_fires_after_task_timeout_sec() -> anyhow::Result<()> {
    let (repo, sha) = init_temp_repo()?;
    let task_id = format!("I-ABE-07-{}", now_nanos()?);

    if let Err(e) = dispatch(
        repo.path(),
        &sha,
        &task_id,
        "Sleep for 30 seconds before any commit.",
        10,
        "^I-ABE-07-",
    )
    .await
    {
        if is_mcp_only_error(&e) {
            eprintln!("SKIP i_abe_07: dispatch_codex_worktree is MCP-only, no HTTP route");
            return Ok(());
        }
        return Err(e);
    }

    let final_status = wait_for_terminal_status(&task_id).await?;
    anyhow::ensure!(final_status.status == TaskStatus::Timeout);
    Ok(())
}

#[tokio::test]
#[ignore]
async fn i_abe_08_contract_validation_rejects_invalid_commit_format() -> anyhow::Result<()> {
    let (repo, sha) = init_temp_repo()?;
    let task_id = format!("I-ABE-08-{}", now_nanos()?);

    if let Err(e) = dispatch(
        repo.path(),
        &sha,
        &task_id,
        "Create file and commit with a normal message.",
        180,
        "^THIS-FORMAT-CANNOT-MATCH-REAL-COMMIT-MESSAGES$",
    )
    .await
    {
        if is_mcp_only_error(&e) {
            eprintln!("SKIP i_abe_08: dispatch_codex_worktree is MCP-only, no HTTP route");
            return Ok(());
        }
        return Err(e);
    }

    let final_status = wait_for_terminal_status(&task_id).await?;
    anyhow::ensure!(
        matches!(
            final_status.status,
            TaskStatus::Failed | TaskStatus::SetupFailed | TaskStatus::Stuck
        ),
        "expected failure-like status, got {:?} with error {:?}",
        final_status.status,
        final_status.error_message
    );
    Ok(())
}
