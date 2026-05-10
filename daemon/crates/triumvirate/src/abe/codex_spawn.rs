use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use anyhow::Context;
use daemon_core::metrics::DaemonMetrics;
use tokio::{
    io::{self, AsyncRead, AsyncWriteExt},
    process::Child,
    process::Command,
    sync::Mutex,
};
use tracing::instrument;

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub cmd: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub envs: HashMap<String, String>,
    pub output_log_dir: Option<PathBuf>,
}

#[instrument(skip_all)]
pub async fn spawn_background(spec: SpawnSpec) -> anyhow::Result<Arc<Mutex<Child>>> {
    let mut command = Command::new(&spec.cmd);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Backstop: if the parent task is dropped (panic, cancel, abnormal
        // teardown) before the monitor reaps the child, tokio sends SIGKILL
        // and reaps automatically. Prevents zombie codex processes when the
        // main monitor task aborts before its select! resolves.
        .kill_on_drop(true);
    for (k, v) in spec.envs {
        command.env(k, v);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn {}", spec.cmd))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    spawn_output_drain(
        stdout,
        spec.output_log_dir
            .as_ref()
            .map(|dir| dir.join("stdout.log")),
        "stdout",
    );
    spawn_output_drain(
        stderr,
        spec.output_log_dir
            .as_ref()
            .map(|dir| dir.join("stderr.log")),
        "stderr",
    );
    Ok(Arc::new(Mutex::new(child)))
}

fn spawn_output_drain<R>(stream: Option<R>, log_path: Option<PathBuf>, stream_name: &'static str)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let Some(stream) = stream else {
        return;
    };
    tokio::spawn(async move {
        if let Err(err) = drain_output(stream, log_path, stream_name).await {
            tracing::warn!(
                stream = stream_name,
                error = %err,
                "failed to drain codex worker output"
            );
        }
    });
}

async fn drain_output<R>(
    mut stream: R,
    log_path: Option<PathBuf>,
    stream_name: &'static str,
) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin,
{
    if let Some(path) = log_path {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create output log directory {}", parent.display()))?;
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .with_context(|| format!("failed to open codex worker output log {}", path.display()))?;
        io::copy(&mut stream, &mut file).await?;
        file.flush().await?;
    } else {
        let mut sink = io::sink();
        let bytes = io::copy(&mut stream, &mut sink).await?;
        tracing::debug!(stream = stream_name, bytes, "drained codex worker output");
    }
    Ok(())
}

#[instrument(skip_all, fields(status = "timeout"))]
pub async fn enforce_timeout(child: Arc<Mutex<Child>>, timeout_sec: u64, cwd: &Path) -> anyhow::Result<bool> {
    enforce_timeout_with_metrics(child, timeout_sec, cwd, None).await
}

#[instrument(skip_all, fields(timeout_sec))]
pub async fn enforce_timeout_with_metrics(
    child: Arc<Mutex<Child>>,
    timeout_sec: u64,
    cwd: &Path,
    metrics: Option<&DaemonMetrics>,
) -> anyhow::Result<bool> {
    tokio::time::sleep(std::time::Duration::from_secs(timeout_sec)).await;

    let mut child = child.lock().await;
    if child
        .try_wait()
        .context("failed to check child process status before timeout enforcement")?
        .is_some()
    {
        return Ok(false);
    }

    if let Some(pid) = child.id() {
        // SIGTERM first to give the worker a chance to flush state and exit cleanly.
        let _ = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        tracing::warn!(
            task_id = "unknown",
            timeout_sec,
            signal = "SIGTERM",
            "abe_timeout_triggered"
        );
        if let Some(metrics) = metrics {
            metrics.abe_timeout_total.inc();
        }
    }
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    // After SIGTERM grace: if the child has exited on its own (e.g. flushed
    // state and quit cleanly in response to SIGTERM, or finished naturally
    // milliseconds after the deadline fired), report this as a non-timeout
    // so the dispatcher can pick up the natural exit via wait() and route
    // through the normal completion path instead of mark_timeout. Closes
    // the false-timeout race observed when codex finishes inside the
    // SIGTERM grace window.
    if child
        .try_wait()
        .context("failed to check child process status before SIGKILL escalation")?
        .is_some()
    {
        cleanup_git_locks(cwd);
        return Ok(false);
    }

    let _ = child
        .kill()
        .await
        .context("failed to send SIGKILL to timed-out child process");
    tracing::warn!(
        task_id = "unknown",
        timeout_sec,
        signal = "SIGKILL",
        "abe_timeout_triggered"
    );
    if let Some(metrics) = metrics {
        metrics.abe_timeout_total.inc();
    }

    cleanup_git_locks(cwd);
    Ok(true)
}

fn cleanup_git_locks(worktree_path: &Path) {
    let git_dir = resolve_git_dir(worktree_path).unwrap_or_else(|err| {
        tracing::warn!(
            worktree_path = %worktree_path.display(),
            error = %err,
            "failed_to_resolve_git_dir_for_cleanup"
        );
        worktree_path.join(".git")
    });
    let _ = std::fs::remove_file(git_dir.join("index.lock"));
    let _ = std::fs::remove_file(worktree_path.join(".git").join("index.lock"));
    let _ = std::fs::remove_file(worktree_path.join(".git/index.lock"));
}

fn resolve_git_dir(worktree_path: &Path) -> anyhow::Result<PathBuf> {
    let dot_git = worktree_path.join(".git");
    if dot_git.is_file() {
        let content = std::fs::read_to_string(&dot_git)
            .with_context(|| format!("failed to read git pointer file {}", dot_git.display()))?;
        if let Some(gitdir) = content.lines().find_map(|line| line.strip_prefix("gitdir:")) {
            let raw = gitdir.trim();
            let parsed = PathBuf::from(raw);
            if parsed.is_absolute() {
                return Ok(parsed);
            }
            return Ok(worktree_path.join(parsed));
        }
    }
    Ok(dot_git)
}

#[instrument(skip_all)]
pub fn resolve_commit_outputs(worktree_path: &Path, starting_sha: &str) -> (String, Vec<String>) {
    let head = std::process::Command::new("git")
        .arg("-C")
        .arg(worktree_path)
        .arg("rev-parse")
        .arg("HEAD")
        .output();
    let commit_sha = head
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    if commit_sha == starting_sha {
        return ("".to_string(), Vec::new());
    }

    let files = std::process::Command::new("git")
        .arg("-C")
        .arg(worktree_path)
        .arg("show")
        .arg("--name-only")
        .arg("--pretty=format:")
        .arg("HEAD")
        .output();
    let modified_files = files
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|body| {
            body.lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| line.trim().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    (commit_sha, modified_files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_background_drains_stdout_and_stderr_to_logs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let log_dir = tmp.path().join(".triumvirate").join("logs");
        let child = spawn_background(SpawnSpec {
            cmd: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                "printf stdout-drained; printf stderr-drained >&2".to_string(),
            ],
            cwd: tmp.path().display().to_string(),
            envs: HashMap::new(),
            output_log_dir: Some(log_dir.clone()),
        })
        .await
        .expect("spawn");

        {
            let mut locked = child.lock().await;
            let status = locked.wait().await.expect("wait");
            assert!(status.success());
        }

        for _ in 0..20 {
            let stdout = std::fs::read_to_string(log_dir.join("stdout.log")).unwrap_or_default();
            let stderr = std::fs::read_to_string(log_dir.join("stderr.log")).unwrap_or_default();
            if stdout.contains("stdout-drained") && stderr.contains("stderr-drained") {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        panic!("expected stdout/stderr drain logs to contain child output");
    }
}
