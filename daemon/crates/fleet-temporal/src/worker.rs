//! The `run_worker` activity's mechanics (build stage B, triumvirate-fleet-BUILD.md).
//!
//! One fleet member's agent run, as a process that can outlive this daemon:
//! - FRESH: write the launch marker, spawn `triumvirate fleet-shim` in its own group with NO
//!   kill_on_drop, wait for its token, clear the marker, then watch it.
//! - ADOPT (decision A): the token verifies Live, so a previous attempt's shim is still running
//!   (the daemon died and Temporal retried). Watch it by polling its identity: a restarted daemon
//!   is not its parent and cannot wait() on it.
//! - FINISHED: a done record from the shim the token names. Return it; nothing runs again.
//! - BLOCK (non-retryable): a Refused token (our process, but not in its group), or a launch
//!   marker whose owner never produced a live token (checked in `launch`).
//!
//! Cancellation arrives through heartbeats; it stops the verified group (SIGTERM, grace,
//! SIGKILL, every result checked) and returns cancelled.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use fleet::{
    shim::{self, DoneRecord},
    worker_token::{self, Verification, WorkerToken},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunWorkerInput {
    pub project_root: String,
    pub fleet_id: String,
    pub task_id: String,
    pub agent: String,
    pub worktree: String,
    /// The agent command line, argv[0] first.
    pub command: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunWorkerOutput {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub branch_head: Option<String>,
    /// Last bytes of the agent's output. Never the whole transcript (2 MB payload limit).
    pub stdout_tail: String,
    pub stderr_tail: String,
    /// True when this attempt found the work already running or finished.
    pub adopted: bool,
}

/// How often the watch loop heartbeats and checks for completion.
pub const POLL: Duration = Duration::from_secs(2);
/// How long a fresh shim may take to write its token.
const TOKEN_WAIT: Duration = Duration::from_secs(15);
const TAIL_BYTES: u64 = 4096;

/// A failure the retry policy must not retry: retrying cannot make it right, and could run the
/// same task twice.
#[derive(Debug)]
pub struct Blocked(pub String);

pub enum Start {
    Finished(DoneRecord),
    Adopt(WorkerToken),
    Fresh,
}

/// Decide what this attempt must do from what is on disk. Pure reads.
pub fn decide(root: &Path, fleet_id: &str, task_id: &str) -> Result<Start, Blocked> {
    let token = worker_token::read_token(root, fleet_id, task_id)
        .map_err(|e| Blocked(format!("unreadable launch token: {e}")))?;
    let done = shim::read_done(root, fleet_id, task_id)
        .map_err(|e| Blocked(format!("unreadable done record: {e}")))?;
    match (token, done) {
        (Some(t), Some(d)) if d.shim_pid == t.pid && d.shim_start_time_us == t.start_time_us => Ok(Start::Finished(d)),
        (Some(t), _) => match t.verify() {
            Verification::Live => Ok(Start::Adopt(t)),
            // The previous shim is gone without a matching completion record: it was killed
            // mid-run. Nothing of ours is running, so a fresh run is safe (at least once).
            Verification::Gone | Verification::Reused(_) => Ok(Start::Fresh),
            Verification::Refused(why) => Err(Blocked(format!("worker cannot be verified: {why}"))),
        },
        // No token. A launch marker here means another attempt may be mid-launch, or one died
        // between claiming and spawning: `launch` sorts that out (its exclusive claim fails, it
        // waits for a live token to adopt, and blocks only if none appears). Blocking here would
        // fail a member while a concurrent attempt is starting it.
        (None, _) => Ok(Start::Fresh),
    }
}

/// The binary to run as the shim: `TRIUMVIRATE_SHIM_BIN`, else this executable.
fn shim_bin() -> std::io::Result<PathBuf> {
    match std::env::var_os("TRIUMVIRATE_SHIM_BIN") {
        Some(p) => Ok(PathBuf::from(p)),
        None => std::env::current_exe(),
    }
}

/// Launch a fresh shim and return it once its token is on disk. If another attempt already owns
/// the launch (its marker exists), wait for THAT shim's token and return it with no child: this
/// attempt adopts it rather than starting the agent a second time.
pub async fn launch(input: &RunWorkerInput) -> Result<(Option<tokio::process::Child>, WorkerToken), Blocked> {
    let root = Path::new(&input.project_root);
    let claimed = worker_token::try_claim_launch(root, &input.fleet_id, &input.task_id)
        .map_err(|e| Blocked(format!("launch marker could not be written: {e}")))?;
    if !claimed {
        let started = Instant::now();
        loop {
            if let Ok(Some(t)) = worker_token::read_token(root, &input.fleet_id, &input.task_id)
                && t.verify() == Verification::Live
            {
                return Ok((None, t));
            }
            if started.elapsed() > TOKEN_WAIT {
                return Err(Blocked(
                    "another attempt holds the launch marker but no live worker token appeared".to_string(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    let bin = shim_bin().map_err(|e| Blocked(format!("cannot locate the shim binary: {e}")))?;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.arg("fleet-shim")
        .arg("--project-root")
        .arg(&input.project_root)
        .args(["--fleet-id", &input.fleet_id, "--task-id", &input.task_id, "--agent", &input.agent])
        .arg("--worktree")
        .arg(&input.worktree)
        .arg("--")
        .args(&input.command)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // The worker must survive this daemon: no kill_on_drop, its own group.
        .kill_on_drop(false)
        .process_group(0);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            worker_token::clear_launch_marker(root, &input.fleet_id, &input.task_id);
            return Err(Blocked(format!("the shim did not start: {e}")));
        }
    };
    let pid = child.id().unwrap_or(0);
    // Our child's OS start time, read now while it certainly exists. The token must carry the same
    // pid AND start time: the pid alone could match a stale token after pid reuse (Codex, review
    // of 3871850). Not "is it Live now": a fast worker can finish before this loop looks, and its
    // token is still ours.
    let child_start = worker_token::proc_info(pid).map(|i| i.start_time_us);
    let started = Instant::now();
    loop {
        if let Ok(Some(t)) = worker_token::read_token(root, &input.fleet_id, &input.task_id)
            && t.pid == pid
            && child_start.is_some_and(|cs| cs == t.start_time_us)
        {
            worker_token::clear_launch_marker(root, &input.fleet_id, &input.task_id);
            return Ok((Some(child), t));
        }
        if let Ok(Some(status)) = child.try_wait() {
            worker_token::clear_launch_marker(root, &input.fleet_id, &input.task_id);
            return Err(Blocked(format!("the shim exited before writing its token ({status})")));
        }
        if started.elapsed() > TOKEN_WAIT {
            // Leave the marker: something may be running that no token names.
            return Err(Blocked(format!("the shim (pid {pid}) wrote no token within {}s", TOKEN_WAIT.as_secs())));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The run has a matching completion record.
pub fn finished(root: &Path, input: &RunWorkerInput, token: &WorkerToken) -> Option<DoneRecord> {
    shim::read_done(root, &input.fleet_id, &input.task_id)
        .ok()
        .flatten()
        .filter(|d| d.shim_pid == token.pid && d.shim_start_time_us == token.start_time_us)
}

fn tail(path: std::io::Result<PathBuf>) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(path) = path else { return String::new() };
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)));
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

pub fn output(input: &RunWorkerInput, done: DoneRecord, adopted: bool) -> RunWorkerOutput {
    let root = Path::new(&input.project_root);
    RunWorkerOutput {
        exit_code: done.exit_code,
        signal: done.signal,
        branch_head: done.branch_head,
        stdout_tail: tail(shim::out_path(root, &input.fleet_id, &input.task_id)),
        stderr_tail: tail(shim::err_path(root, &input.fleet_id, &input.task_id)),
        adopted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_for(pid: u32) -> WorkerToken {
        WorkerToken::for_spawned_child(pid, "fleet-d", "fleet-d-T-001", "stub").expect("token")
    }

    fn spawn_leader(script: &str) -> std::process::Child {
        use std::os::unix::process::CommandExt;
        std::process::Command::new("sh").args(["-c", script]).process_group(0).spawn().expect("spawn")
    }

    /// Each branch of `decide`, from real files and real processes.
    #[test]
    fn decide_reads_the_disk_correctly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let (f, t) = ("fleet-d", "fleet-d-T-001");

        assert!(matches!(decide(root, f, t), Ok(Start::Fresh)), "nothing on disk: fresh");

        worker_token::write_launch_marker(root, f, t).expect("marker");
        assert!(matches!(decide(root, f, t), Ok(Start::Fresh)), "marker, no token: launch decides (claim fails, waits, adopts or blocks)");
        worker_token::clear_launch_marker(root, f, t);

        let mut live = spawn_leader("sleep 30");
        let tok = token_for(live.id());
        worker_token::write_token(root, &tok).expect("token");
        assert!(matches!(decide(root, f, t), Ok(Start::Adopt(_))), "live token: adopt");

        let done = DoneRecord {
            exit_code: Some(0),
            signal: None,
            branch_head: None,
            finished_at_ms: 0,
            shim_pid: tok.pid,
            shim_start_time_us: tok.start_time_us,
        };
        std::fs::write(shim::done_path(root, f, t).unwrap(), serde_json::to_vec(&done).unwrap()).unwrap();
        assert!(matches!(decide(root, f, t), Ok(Start::Finished(_))), "matching done: finished");

        let other = DoneRecord { shim_pid: tok.pid + 1, ..done };
        std::fs::write(shim::done_path(root, f, t).unwrap(), serde_json::to_vec(&other).unwrap()).unwrap();
        assert!(matches!(decide(root, f, t), Ok(Start::Adopt(_))), "a done record from another shim is not ours");

        let _ = live.kill();
        let _ = live.wait();
        assert!(matches!(decide(root, f, t), Ok(Start::Fresh)), "dead shim, no matching done: fresh");
    }

    /// Two attempts racing to launch one member: the second finds the first's marker, waits for
    /// its live token and adopts it, spawning nothing. RED IF it launches a second agent.
    #[tokio::test]
    async fn a_second_attempt_adopts_instead_of_launching_again() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let input = RunWorkerInput {
            project_root: root.display().to_string(),
            fleet_id: "fleet-d".to_string(),
            task_id: "fleet-d-T-001".to_string(),
            agent: "stub".to_string(),
            worktree: root.display().to_string(),
            // Would be visible if launched: the test asserts no child came back.
            command: vec!["sh".to_string(), "-c".to_string(), "exit 0".to_string()],
        };
        assert!(worker_token::try_claim_launch(root, "fleet-d", "fleet-d-T-001").unwrap(), "first attempt claims");
        let mut first = spawn_leader("sleep 30");
        worker_token::write_token(root, &token_for(first.id())).expect("first attempt's token");
        let Ok((child, token)) = launch(&input).await else { panic!("second attempt must adopt") };
        assert!(child.is_none(), "the second attempt must not spawn anything");
        assert_eq!(token.pid, first.id());
        let _ = first.kill();
        let _ = first.wait();
    }
}
