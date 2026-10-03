//! Launch tokens: the only way anything outside a worker's own wait loop may find and signal it.
//!
//! A fleet worker's pid lives in `FLEET_CHILDREN`, which is memory. After a restart that map is
//! empty, and the obvious substitutes are all unsafe: matching by working directory kills an
//! operator's shell sitting in the worktree (unanimous across four review seats, 2026-10-03), and
//! a bare pid can be reused by an unrelated process once the worker exits.
//!
//! So each worker gets `.triumvirate/worker.json` in its worktree, written right after spawn: pid,
//! process group, the OS-reported start time, and the fleet, task and agent. Every later signal
//! (stall, cancel, startup recovery) re-reads the OS immediately before signalling and refuses
//! loudly on any mismatch. A pid and a start time together name one process, ever.
//!
//! The token also names its OWNER: the process that spawned the worker and waits on it. Fleets
//! run inside whichever `triumvirate mcp` process the session started, not inside the daemon, so
//! "the daemon restarted" does not mean "this worker is orphaned". Recovery may only touch a
//! fleet whose owner is gone.

use std::{
    fs,
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

/// Where the token lives, relative to the worktree.
pub const TOKEN_RELATIVE_PATH: &str = ".triumvirate/worker.json";

/// The fleet-level owner record, under the project root, one file per fleet. It exists before any
/// worker launches, so a fleet that died while still `spawning` (no tokens yet) is recoverable.
pub fn owner_record_path(project_root: &Path, fleet_id: &str) -> PathBuf {
    project_root
        .join(".triumvirate")
        .join("fleet-owners")
        .join(format!("{fleet_id}.json"))
}

/// What the OS says about one pid right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcInfo {
    /// Microseconds since the Unix epoch.
    pub start_time_us: u64,
    pub pgid: u32,
}

/// One process, named so it cannot be confused with a later process that reuses the pid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_time_us: u64,
}

impl ProcessIdentity {
    /// This process.
    pub fn current() -> Option<Self> {
        let pid = std::process::id();
        proc_info(pid).map(|i| Self { pid, start_time_us: i.start_time_us })
    }

    /// True only when this exact process (pid AND start time) is still running.
    pub fn is_alive(&self) -> bool {
        matches!(proc_info(self.pid), Some(i) if i.start_time_us == self.start_time_us)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerToken {
    pub pid: u32,
    pub pgid: u32,
    pub start_time_us: u64,
    pub fleet_id: String,
    pub task_id: String,
    pub agent: String,
    pub owner: ProcessIdentity,
}

/// The result of re-reading the OS for a token's process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    /// The same process, still in the same group. Safe to signal.
    Live,
    /// No process with that pid (or only a zombie). Nothing to signal.
    Gone,
    /// A process holds the pid but it is not ours. Signal NOTHING.
    Mismatch(String),
}

/// What a terminate attempt did. `Err` from `terminate` means it may still be running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminateOutcome {
    AlreadyGone,
    /// Exited within the grace period after SIGTERM.
    Terminated,
    /// Ignored SIGTERM for the grace period and was SIGKILLed.
    Killed,
}

impl WorkerToken {
    /// Build the token for a just-spawned child, reading its start time and group from the OS.
    /// `None` when the child is already gone or the OS will not say; the caller logs that.
    pub fn for_spawned_child(pid: u32, fleet_id: &str, task_id: &str, agent: &str) -> Option<Self> {
        let info = proc_info(pid)?;
        Some(Self {
            pid,
            pgid: info.pgid,
            start_time_us: info.start_time_us,
            fleet_id: fleet_id.to_string(),
            task_id: task_id.to_string(),
            agent: agent.to_string(),
            owner: ProcessIdentity::current()?,
        })
    }

    pub fn verify(&self) -> Verification {
        match proc_info(self.pid) {
            None => Verification::Gone,
            Some(i) if i.start_time_us != self.start_time_us => Verification::Mismatch(format!(
                "pid {} start time is {} but the token says {}: the pid was reused",
                self.pid, i.start_time_us, self.start_time_us
            )),
            Some(i) if i.pgid != self.pgid => Verification::Mismatch(format!(
                "pid {} is in process group {} but the token says {}",
                self.pid, i.pgid, self.pgid
            )),
            Some(_) => Verification::Live,
        }
    }

    /// Verify, then signal the worker's whole process group. `Ok(false)` means it was already
    /// gone. A mismatch is an `Err` and nothing was signalled.
    pub fn signal_verified(&self, sig: i32) -> Result<bool, String> {
        match self.verify() {
            Verification::Gone => Ok(false),
            Verification::Mismatch(why) => {
                tracing::error!(
                    fleet_id = %self.fleet_id,
                    task_id = %self.task_id,
                    pid = self.pid,
                    reason = %why,
                    "launch token does not match the running process; refusing to signal"
                );
                Err(format!("refused to signal {}: {why}", self.pid))
            }
            Verification::Live => signal_group(self.pgid, sig).map(|_| true),
        }
    }

    /// SIGTERM the group, wait up to `grace`, then SIGKILL whatever is left of the group, and
    /// wait for it to go. Every signal is checked. Blocking: call from a blocking context.
    pub fn terminate(&self, grace: Duration) -> Result<TerminateOutcome, String> {
        if !self.signal_verified(libc::SIGTERM)? {
            return Ok(TerminateOutcome::AlreadyGone);
        }
        if wait_until(grace, || !self.group_exists()) {
            return Ok(TerminateOutcome::Terminated);
        }
        // The leader may have exited while a child that ignores TERM keeps the group alive. A
        // process group id cannot be reused while the group has a member, so the group we are
        // about to kill is still the one we verified at SIGTERM. If the leader is still here,
        // verify it again anyway: that is the case the token exists for.
        if self.verify() != Verification::Gone {
            self.signal_verified(libc::SIGKILL)?;
        } else {
            signal_group(self.pgid, libc::SIGKILL)?;
        }
        tracing::warn!(
            fleet_id = %self.fleet_id,
            task_id = %self.task_id,
            pid = self.pid,
            grace_ms = grace.as_millis() as u64,
            "worker ignored SIGTERM for the grace period; escalated to SIGKILL"
        );
        if wait_until(Duration::from_secs(5), || !self.group_exists()) {
            Ok(TerminateOutcome::Killed)
        } else {
            Err(format!(
                "process group {} still exists 5s after SIGKILL",
                self.pgid
            ))
        }
    }

    /// Whether any member of the worker's group is still running (zombies excluded for the
    /// leader; other members are reaped by their own parents).
    fn group_exists(&self) -> bool {
        if self.verify() == Verification::Live {
            return true;
        }
        // SAFETY: kill with signal 0 only checks for existence and permission.
        let rc = unsafe { libc::kill(-(self.pgid as i32), 0) };
        rc == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Send `sig` to process group `pgid`, checking the result. ESRCH (the group is already empty)
/// is not an error.
pub fn signal_group(pgid: u32, sig: i32) -> Result<(), String> {
    if pgid <= 1 {
        return Err(format!("refusing to signal process group {pgid}"));
    }
    // SAFETY: a plain kill(2) on a negative pid (a process group), arguments are plain integers.
    let rc = unsafe { libc::kill(-(pgid as i32), sig) };
    if rc == 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    tracing::error!(pgid, sig, error = %err, "kill failed");
    Err(format!("kill(-{pgid}, {sig}) failed: {err}"))
}

pub fn token_path(worktree: &Path) -> PathBuf {
    worktree.join(TOKEN_RELATIVE_PATH)
}

/// Write-then-rename, so a reader never sees a torn token.
pub fn write_token(worktree: &Path, token: &WorkerToken) -> io::Result<()> {
    write_json_atomically(&token_path(worktree), token)
}

/// `Ok(None)` when there is no token (the worker never launched, or launched before tokens).
pub fn read_token(worktree: &Path) -> io::Result<Option<WorkerToken>> {
    read_json(&token_path(worktree))
}

pub fn write_owner_record(project_root: &Path, fleet_id: &str, owner: &ProcessIdentity) -> io::Result<()> {
    write_json_atomically(&owner_record_path(project_root, fleet_id), owner)
}

pub fn read_owner_record(project_root: &Path, fleet_id: &str) -> io::Result<Option<ProcessIdentity>> {
    read_json(&owner_record_path(project_root, fleet_id))
}

fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(value).map_err(io::Error::other)?)?;
    fs::rename(&tmp, path)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Every worker token under `{project_root}/.triumvirate/worktrees/{fleet_id}-*`, with the
/// worktree it came from. Unreadable tokens are returned as errors so the caller can refuse to
/// call the fleet clean.
pub fn fleet_tokens(project_root: &Path, fleet_id: &str) -> Vec<(PathBuf, io::Result<WorkerToken>)> {
    let base = project_root.join(".triumvirate").join("worktrees");
    let Ok(entries) = fs::read_dir(&base) else {
        return Vec::new();
    };
    let prefix = format!("{fleet_id}-");
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let matches = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(&prefix));
        if !matches || !path.is_dir() {
            continue;
        }
        match read_token(&path) {
            Ok(Some(t)) if t.fleet_id == fleet_id => out.push((path, Ok(t))),
            // A token for a different fleet in this fleet's directory is not ours to act on.
            Ok(Some(t)) => out.push((
                path,
                Err(io::Error::other(format!("token names fleet {}, not {fleet_id}", t.fleet_id))),
            )),
            Ok(None) => {}
            Err(e) => out.push((path, Err(e))),
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// What the OS reports for `pid`, or `None` when there is no such process or it is a zombie.
#[cfg(target_os = "macos")]
pub fn proc_info(pid: u32) -> Option<ProcInfo> {
    const SZOMB: u32 = 5;
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: proc_pidinfo writes at most `size` bytes into `info`, which is that size.
    let n = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if n != size || info.pbi_status == SZOMB {
        return None;
    }
    Some(ProcInfo {
        start_time_us: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
        pgid: info.pbi_pgid,
    })
}

/// Linux: field 22 of /proc/PID/stat is the start time in clock ticks since boot. That is not
/// epoch time, but it is stable for the life of the process, which is all the token needs.
#[cfg(target_os = "linux")]
pub fn proc_info(pid: u32) -> Option<ProcInfo> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is in parentheses and may contain spaces; parse after the last ')'.
    let rest = &stat[stat.rfind(')')? + 2..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // rest starts at field 3 (state).
    if fields.first() == Some(&"Z") {
        return None;
    }
    Some(ProcInfo {
        pgid: fields.get(2)?.parse().ok()?,
        start_time_us: fields.get(19)?.parse().ok()?,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn proc_info(_pid: u32) -> Option<ProcInfo> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    /// A real process in its own group, a child of this test, killed and reaped on drop.
    struct Stub(std::process::Child);
    impl Drop for Stub {
        fn drop(&mut self) {
            let _ = signal_group(self.0.id(), libc::SIGKILL);
            let _ = self.0.wait();
        }
    }
    fn stub(script: &str) -> Stub {
        use std::os::unix::process::CommandExt;
        Stub(
            Command::new("sh")
                .arg("-c")
                .arg(script)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .expect("spawn stub"),
        )
    }

    fn reaped(s: &mut Stub, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = s.0.try_wait() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn a_live_worker_verifies() {
        let s = stub("sleep 30");
        let t = WorkerToken::for_spawned_child(s.0.id(), "f", "t", "codex").expect("token");
        assert_eq!(t.pgid, s.0.id(), "the stub leads its own group");
        assert_eq!(t.verify(), Verification::Live);
    }

    /// RED IF: a token whose start time does not match is acted on. Nothing may be signalled.
    #[test]
    fn a_wrong_start_time_is_refused_and_nothing_is_signalled() {
        let mut s = stub("sleep 30");
        let mut t = WorkerToken::for_spawned_child(s.0.id(), "f", "t", "codex").expect("token");
        t.start_time_us += 1;
        assert!(matches!(t.verify(), Verification::Mismatch(_)));
        let err = t.terminate(Duration::from_millis(200)).unwrap_err();
        assert!(err.contains("refused"), "{err}");
        assert!(!reaped(&mut s, Duration::from_millis(300)), "the process must still be running");
    }

    #[test]
    fn a_wrong_pgid_is_refused() {
        let mut s = stub("sleep 30");
        let mut t = WorkerToken::for_spawned_child(s.0.id(), "f", "t", "codex").expect("token");
        t.pgid = std::process::id();
        assert!(t.signal_verified(libc::SIGKILL).is_err());
        assert!(!reaped(&mut s, Duration::from_millis(300)));
    }

    #[test]
    fn sigterm_stops_a_cooperative_worker() {
        let mut s = stub("sleep 30");
        let t = WorkerToken::for_spawned_child(s.0.id(), "f", "t", "codex").expect("token");
        // Reap concurrently, the way a real owner (or launchd for an orphan) would.
        let pid = s.0.id();
        let reaper = std::thread::spawn(move || {
            let mut status = 0;
            // SAFETY: waitpid on our own child.
            unsafe { libc::waitpid(pid as i32, &mut status, 0) };
        });
        assert_eq!(t.terminate(Duration::from_secs(3)), Ok(TerminateOutcome::Terminated));
        reaper.join().expect("reaper");
        // Already reaped by the thread above; Stub's drop would wait on a pid we no longer own.
        std::mem::forget(s);
    }

    /// The escalation: a worker that ignores SIGTERM is SIGKILLed after the grace period.
    #[test]
    fn a_worker_that_ignores_sigterm_is_killed_after_the_grace() {
        let s = stub("trap '' TERM; while :; do sleep 1; done");
        std::thread::sleep(Duration::from_millis(200)); // let the trap install
        let t = WorkerToken::for_spawned_child(s.0.id(), "f", "t", "codex").expect("token");
        let pid = s.0.id();
        let reaper = std::thread::spawn(move || {
            let mut status = 0;
            // SAFETY: waitpid on our own child.
            unsafe { libc::waitpid(pid as i32, &mut status, 0) };
        });
        let started = Instant::now();
        assert_eq!(t.terminate(Duration::from_millis(500)), Ok(TerminateOutcome::Killed));
        assert!(started.elapsed() >= Duration::from_millis(500), "the grace was honoured");
        reaper.join().expect("reaper");
        std::mem::forget(s);
    }

    #[test]
    fn tokens_round_trip_atomically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let s = stub("sleep 30");
        let t = WorkerToken::for_spawned_child(s.0.id(), "fleet-1", "fleet-1-T-001", "grok").expect("token");
        write_token(dir.path(), &t).expect("write");
        assert_eq!(read_token(dir.path()).expect("read"), Some(t));
        assert_eq!(read_token(&dir.path().join("none")).expect("read"), None);
    }
}
