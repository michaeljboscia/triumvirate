//! `triumvirate fleet-shim`, through the real binary (build stage A, triumvirate-fleet-BUILD.md).
//!
//! The shim calls setpgid and changes signal dispositions, so it is only ever run as its own
//! process here, never inside the test harness.

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use fleet::{shim, worker_token};

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    worktree: PathBuf,
}

const FLEET: &str = "fleet-shimtest";
const TASK: &str = "fleet-shimtest-T-001";

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("project");
    let worktree = dir.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    Fixture { root, worktree, _dir: dir }
}

fn start(fx: &Fixture, script: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_triumvirate"))
        .args(["fleet-shim", "--project-root"])
        .arg(&fx.root)
        .args(["--fleet-id", FLEET, "--task-id", TASK, "--agent", "stub", "--worktree"])
        .arg(&fx.worktree)
        .args(["--", "sh", "-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn fleet-shim")
}

fn wait_token(root: &Path) -> worker_token::WorkerToken {
    let started = Instant::now();
    loop {
        if let Ok(Some(t)) = worker_token::read_token(root, FLEET, TASK) {
            return t;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the shim never wrote its token");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// RED IF: output lands in the worktree, the done record misses the exit code, or the token and
/// the done record name different processes.
#[test]
fn a_stub_runs_and_everything_lands_outside_the_worktree() {
    let fx = fixture();
    let status = start(&fx, "echo out-line; echo err-line >&2; exit 7").wait().expect("wait");
    assert_eq!(status.code(), Some(7), "the shim exits with the agent's code");

    let out = std::fs::read_to_string(shim::out_path(&fx.root, FLEET, TASK).unwrap()).unwrap();
    let err = std::fs::read_to_string(shim::err_path(&fx.root, FLEET, TASK).unwrap()).unwrap();
    assert_eq!(out.trim(), "out-line");
    assert_eq!(err.trim(), "err-line");
    let done = shim::read_done(&fx.root, FLEET, TASK).unwrap().expect("done record");
    assert_eq!(done.exit_code, Some(7));
    assert_eq!(done.signal, None);
    let token = worker_token::read_token(&fx.root, FLEET, TASK).unwrap().expect("token");
    assert_eq!(token.pid, done.shim_pid, "token and done record name the same shim");
    assert_eq!(token.start_time_us, done.shim_start_time_us);
    assert_eq!(token.pgid, token.pid, "the shim leads its own group");
    let in_worktree: Vec<_> = std::fs::read_dir(&fx.worktree).unwrap().flatten().map(|e| e.path()).collect();
    assert!(in_worktree.is_empty(), "nothing may be written into the worktree: {in_worktree:?}");
}

/// A cancel's group SIGTERM ends the agent; the shim survives it and records the signal.
/// RED IF the shim dies with the agent (no record) or the agent inherits SIG_IGN and runs on.
#[test]
fn a_group_sigterm_ends_the_agent_and_the_shim_records_it() {
    let fx = fixture();
    let mut child = start(&fx, "sleep 30");
    let token = wait_token(&fx.root);
    std::thread::sleep(Duration::from_millis(300)); // let the agent start
    let started = Instant::now();
    worker_token::signal_group(token.pgid, libc::SIGTERM).expect("signal");
    let status = child.wait().expect("wait");
    assert!(started.elapsed() < Duration::from_secs(5), "the agent ignored SIGTERM");
    assert_eq!(status.code(), Some(128 + libc::SIGTERM), "the shim reports the agent's signal");
    let done = shim::read_done(&fx.root, FLEET, TASK).unwrap().expect("the shim must record a cancelled run");
    assert_eq!(done.signal, Some(libc::SIGTERM));
}

/// SIGKILL takes the shim too, so no done record: that is how a retry reads "partial".
#[test]
fn a_sigkilled_shim_leaves_no_done_record() {
    let fx = fixture();
    let mut child = start(&fx, "sleep 30");
    let token = wait_token(&fx.root);
    worker_token::signal_group(token.pgid, libc::SIGKILL).expect("signal");
    let _ = child.wait();
    assert!(shim::read_done(&fx.root, FLEET, TASK).unwrap().is_none());
}

/// A done record left by an earlier attempt must not be read as this run's.
#[test]
fn a_stale_done_record_is_cleared_before_the_agent_runs() {
    let fx = fixture();
    assert_eq!(start(&fx, "exit 0").wait().expect("wait").code(), Some(0));
    let mut child = start(&fx, "sleep 30");
    let _ = wait_token(&fx.root);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        shim::read_done(&fx.root, FLEET, TASK).unwrap().is_none(),
        "the previous attempt's done record survived into a running attempt"
    );
    let t = worker_token::read_token(&fx.root, FLEET, TASK).unwrap().unwrap();
    worker_token::signal_group(t.pgid, libc::SIGKILL).expect("cleanup");
    let _ = child.wait();
}
