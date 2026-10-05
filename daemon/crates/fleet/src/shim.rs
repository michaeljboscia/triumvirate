//! `triumvirate fleet-shim`: the process a Temporal `run_worker` activity launches for one fleet
//! worker (design `triumvirate-fleet.md`, build stage A).
//!
//! It exists so a worker can outlive the daemon that launched it and still be found, stopped or
//! adopted afterwards:
//! - It leads its own process group and writes its launch token for ITSELF before spawning the
//!   agent, so a verified group signal reaches it and the agent together.
//! - The agent's stdout and stderr go to FILES beside the token, never to pipes the daemon owns
//!   (a pipe breaks when the daemon dies, and SIGPIPE would kill the worker adoption needs), and
//!   never into the worktree (the agent's own `git add -A` would sweep them up).
//! - When the agent exits, it writes `{task}.done` atomically: the record a retry reads to tell
//!   finished from partial. A shim killed by SIGKILL writes none, so "partial" is what it reads.
//! - It ignores SIGTERM/SIGINT/SIGHUP itself (and resets them to default for the agent), so a
//!   cancel's group SIGTERM ends the agent and the shim still records how it ended.
//!
//! Synchronous on purpose: no runtime, nothing to outlive.

use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};

use crate::worker_token::{self, WorkerToken};

#[derive(Debug, Clone)]
pub struct ShimArgs {
    pub project_root: PathBuf,
    pub fleet_id: String,
    pub task_id: String,
    pub agent: String,
    pub worktree: PathBuf,
    /// The agent command line, argv[0] first.
    pub command: Vec<String>,
}

/// What the shim saw when the agent exited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoneRecord {
    pub exit_code: Option<i32>,
    /// Set when the agent was ended by a signal (a cancel's SIGTERM, for instance).
    pub signal: Option<i32>,
    /// `git rev-parse HEAD` in the worktree after the agent exited, when it resolves.
    pub branch_head: Option<String>,
    pub finished_at_ms: u64,
    /// The shim that wrote this, so a record cannot be mistaken for another run's.
    pub shim_pid: u32,
    pub shim_start_time_us: u64,
}

fn sidecar(project_root: &Path, fleet_id: &str, task_id: &str, ext: &str) -> io::Result<PathBuf> {
    let token = worker_token::token_path(project_root, fleet_id, task_id)?;
    Ok(token.with_extension(ext))
}

pub fn out_path(project_root: &Path, fleet_id: &str, task_id: &str) -> io::Result<PathBuf> {
    sidecar(project_root, fleet_id, task_id, "out")
}
pub fn err_path(project_root: &Path, fleet_id: &str, task_id: &str) -> io::Result<PathBuf> {
    sidecar(project_root, fleet_id, task_id, "err")
}
pub fn done_path(project_root: &Path, fleet_id: &str, task_id: &str) -> io::Result<PathBuf> {
    sidecar(project_root, fleet_id, task_id, "done")
}

pub fn read_done(project_root: &Path, fleet_id: &str, task_id: &str) -> io::Result<Option<DoneRecord>> {
    match std::fs::read(done_path(project_root, fleet_id, task_id)?) {
        Ok(b) => serde_json::from_slice(&b).map(Some).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The last `max_bytes` of an agent's output for a failure record (D-044): cut on a char
/// boundary, trimmed, without a leading U+FFFD left by an earlier byte-level cut, None if empty.
/// Takes text the worker already holds; it reads no file.
pub fn bounded_tail(text: &str, max_bytes: usize) -> Option<String> {
    let mut start = text.len().saturating_sub(max_bytes);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let tail = text[start..].trim().trim_start_matches('\u{FFFD}').trim_start();
    (!tail.is_empty()).then(|| tail.to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn set_signal(sig: i32, handler: libc::sighandler_t) {
    // SAFETY: installing SIG_IGN or SIG_DFL for a plain signal number.
    unsafe {
        libc::signal(sig, handler);
    }
}

/// Run the shim. Returns the exit code the shim process should exit with (the agent's code, or
/// 128 + signal). Errors are setup failures, before any agent ran.
pub fn run(args: &ShimArgs) -> anyhow::Result<i32> {
    use std::os::unix::process::{CommandExt, ExitStatusExt};

    if args.command.is_empty() {
        anyhow::bail!("fleet-shim: no agent command given");
    }
    // Lead our own group. The activity spawns us with process_group(0) already; this makes the
    // shim correct when launched any other way, and the token requires it.
    // SAFETY: setpgid on ourselves.
    unsafe {
        libc::setpgid(0, 0);
    }
    let me = std::process::id();
    let token = WorkerToken::for_spawned_child(me, &args.fleet_id, &args.task_id, &args.agent)
        .ok_or_else(|| anyhow::anyhow!("fleet-shim: cannot read our own process identity, or we do not lead our group"))?;
    // Survive a group SIGTERM ourselves so the agent's end gets recorded. Installed BEFORE the
    // token exists: once the token is on disk a cancel may signal the group, and a shim killed in
    // that window would leave no record (Codex, review of 3871850). The agent gets the default
    // dispositions back before exec (SIG_IGN is inherited across exec otherwise).
    for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        set_signal(sig, libc::SIG_IGN);
    }
    // Stale outputs from an earlier attempt must not be read as this run's: the done record and
    // a half-written one a SIGKILLed shim left behind.
    let done = done_path(&args.project_root, &args.fleet_id, &args.task_id)?;
    for p in [done.clone(), done.with_extension("done.tmp")] {
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => anyhow::bail!("fleet-shim: cannot clear stale {}: {e}", p.display()),
        }
    }
    worker_token::write_token(&args.project_root, &token)?;

    let out = File::create(out_path(&args.project_root, &args.fleet_id, &args.task_id)?)?;
    let err = File::create(err_path(&args.project_root, &args.fleet_id, &args.task_id)?)?;

    let mut cmd = Command::new(&args.command[0]);
    cmd.args(&args.command[1..])
        .current_dir(&args.worktree)
        .env("TRIUMVIRATE_PROJECT_ROOT", &args.project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err));
    // SAFETY: pre_exec runs in the child after fork; signal() is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
                libc::signal(sig, libc::SIG_DFL);
            }
            Ok(())
        });
    }
    let status = cmd.spawn()?.wait()?;

    let branch_head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&args.worktree)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    let done = DoneRecord {
        exit_code: status.code(),
        signal: status.signal(),
        branch_head,
        finished_at_ms: now_ms(),
        shim_pid: me,
        shim_start_time_us: token.start_time_us,
    };
    // Durable, not only atomic: the record is how a retry knows the agent finished, so a power
    // loss must not erase it and rerun finished work (Codex, review of 3871850).
    let path = done_path(&args.project_root, &args.fleet_id, &args.task_id)?;
    let tmp = path.with_extension("done.tmp");
    {
        use std::io::Write;
        let mut f = File::create(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(&done)?)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    if let Some(dir) = path.parent() {
        File::open(dir)?.sync_all()?;
    }
    Ok(status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Behavior (token, outputs, done record, SIGTERM and SIGKILL) is tested through the real
    /// binary in crates/triumvirate/tests/integration_fleet_shim.rs: the shim calls setpgid and
    /// changes signal dispositions, which must never run inside the test harness process.
    #[test]
    fn sidecars_sit_beside_the_token_outside_any_worktree() {
        let root = Path::new("/p");
        let t = worker_token::token_path(root, "f", "f-T-001").unwrap();
        assert_eq!(t, Path::new("/p/.triumvirate/fleet-workers/f/f-T-001.json"));
        assert_eq!(out_path(root, "f", "f-T-001").unwrap(), Path::new("/p/.triumvirate/fleet-workers/f/f-T-001.out"));
        assert_eq!(err_path(root, "f", "f-T-001").unwrap(), Path::new("/p/.triumvirate/fleet-workers/f/f-T-001.err"));
        assert_eq!(done_path(root, "f", "f-T-001").unwrap(), Path::new("/p/.triumvirate/fleet-workers/f/f-T-001.done"));
        assert!(done_path(root, "f", "../x").is_err());
    }

    #[test]
    fn bounded_tail_cuts_on_a_char_boundary_and_drops_a_split_char() {
        assert_eq!(bounded_tail("", 10), None);
        assert_eq!(bounded_tail("  \n ", 10), None);
        assert_eq!(bounded_tail("hello world!", 5).as_deref(), Some("orld!"));
        assert_eq!(bounded_tail("hello world!", 50).as_deref(), Some("hello world!"));
        // "é" is 2 bytes; a 3-byte budget lands inside it and must move forward, not panic.
        assert_eq!(bounded_tail("aéz!", 3).as_deref(), Some("z!"));
        // A tail that an upstream lossy cut started mid-char.
        assert_eq!(bounded_tail("\u{FFFD}rest of the error", 100).as_deref(), Some("rest of the error"));
    }
}
