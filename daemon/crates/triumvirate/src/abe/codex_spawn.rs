use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use anyhow::Context;
use tokio::{process::Child, process::Command, sync::Mutex};
use tracing::instrument;

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub cmd: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub envs: HashMap<String, String>,
}

#[instrument(skip_all)]
pub async fn spawn_background(spec: SpawnSpec) -> anyhow::Result<Arc<Mutex<Child>>> {
    let mut command = Command::new(&spec.cmd);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in spec.envs {
        command.env(k, v);
    }
    let child = command
        .spawn()
        .with_context(|| format!("failed to spawn {}", spec.cmd))?;
    Ok(Arc::new(Mutex::new(child)))
}

#[instrument(skip_all, fields(status = "timeout"))]
pub async fn enforce_timeout(child: Arc<Mutex<Child>>, timeout_sec: u64, cwd: &Path) -> anyhow::Result<bool> {
    tokio::time::sleep(std::time::Duration::from_secs(timeout_sec)).await;

    let mut child = child.lock().await;
    if child.try_wait()?.is_some() {
        return Ok(false);
    }

    if let Some(pid) = child.id() {
        // SIGTERM first to give the worker a chance to flush state and exit cleanly.
        let _ = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    }
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    if child.try_wait()?.is_none() {
        let _ = child.kill().await;
    }

    cleanup_git_locks(cwd);
    Ok(true)
}

fn cleanup_git_locks(worktree_path: &Path) {
    let git_dir = resolve_git_dir(worktree_path);
    let _ = std::fs::remove_file(git_dir.join("index.lock"));
    let _ = std::fs::remove_file(worktree_path.join(".git").join("index.lock"));
    let _ = std::fs::remove_file(worktree_path.join(".git/index.lock"));
}

fn resolve_git_dir(worktree_path: &Path) -> PathBuf {
    let dot_git = worktree_path.join(".git");
    if dot_git.is_file() {
        let content = std::fs::read_to_string(&dot_git).unwrap_or_default();
        if let Some(gitdir) = content.lines().find_map(|line| line.strip_prefix("gitdir:")) {
            let raw = gitdir.trim();
            let parsed = PathBuf::from(raw);
            if parsed.is_absolute() {
                return parsed;
            }
            return worktree_path.join(parsed);
        }
    }
    dot_git
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
