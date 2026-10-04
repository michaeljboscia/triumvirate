//! Launch tokens: the only way anything outside a worker's own wait loop may find and signal it.
//!
//! A fleet worker's pid lives in `FLEET_CHILDREN`, which is memory. After a restart that map is
//! empty, and the obvious substitutes are all unsafe: matching by working directory kills an
//! operator's shell sitting in the worktree (unanimous across four review seats, 2026-10-03), and
//! a bare pid can be reused by an unrelated process once the worker exits.
//!
//! So each worker gets a token at `{project_root}/.triumvirate/fleet-workers/{fleet_id}/{task_id}.json`,
//! written right after spawn: pid, process group, the OS-reported start time, and the fleet, task
//! and agent. NOT in the worktree: the worker writes there, so a worker (or a `git clean -fdx` it
//! runs) could rewrite its token to aim a kill elsewhere, or delete it and hide from recovery
//! (Codex, review of 57dfd2d). Codex's workspace-write sandbox cannot reach the project's own
//! `.triumvirate`. Every later signal
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
    /// A process holds the pid but started at a different time: the pid was reused, so OUR
    /// worker is gone. Signal NOTHING; the holder is a stranger.
    Reused(String),
    /// Signal nothing, and do NOT conclude the worker is gone: the token is malformed, or the
    /// pid and start time still match (it IS our worker) but it left the recorded process group
    /// (a CLI that calls setsid). Recovery must block on this (Codex, review of ee7984c).
    Refused(String),
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
    /// Also `None` when the child does not lead its own process group: every signal goes to the
    /// group, and a group the worker merely joined is somebody else's (the spawner's).
    pub fn for_spawned_child(pid: u32, fleet_id: &str, task_id: &str, agent: &str) -> Option<Self> {
        let info = proc_info(pid)?;
        if info.pgid != pid {
            return None;
        }
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
        if self.pgid != self.pid {
            return Verification::Refused(format!(
                "token names group {} for pid {}: a worker always leads its own group",
                self.pgid, self.pid
            ));
        }
        match proc_info(self.pid) {
            None => Verification::Gone,
            Some(i) if i.start_time_us != self.start_time_us => Verification::Reused(format!(
                "pid {} start time is {} but the token says {}: the pid was reused",
                self.pid, i.start_time_us, self.start_time_us
            )),
            Some(i) if i.pgid != self.pgid => Verification::Refused(format!(
                "pid {} is still our worker but moved to process group {} (token says {})",
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
            Verification::Reused(why) | Verification::Refused(why) => {
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
        //
        // Residual risk, accepted (Codex, review of ee7984c): between the last poll and this
        // kill (at most one 50 ms poll), the group could empty AND its id be handed to a new
        // group. That needs the pid space to wrap inside the window. Closing it fully needs a
        // pidfd, which macOS does not have.
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

fn fleet_token_dir(project_root: &Path, fleet_id: &str) -> io::Result<PathBuf> {
    check_id(fleet_id)?;
    Ok(project_root.join(".triumvirate").join("fleet-workers").join(fleet_id))
}

/// Ids become path components. Generated internally, but a separator or `..` would escape the
/// token directory, so refuse them rather than trust that.
fn check_id(id: &str) -> io::Result<()> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id == "." || id == ".." {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unsafe id for a token path: {id:?}")));
    }
    Ok(())
}

/// Send `sig` to one process, checking the result. ESRCH (already gone) is not an error.
pub fn signal_pid(pid: u32, sig: i32) -> Result<(), String> {
    if pid <= 1 {
        return Err(format!("refusing to signal pid {pid}"));
    }
    // SAFETY: a plain kill(2) on a positive pid.
    let rc = unsafe { libc::kill(pid as i32, sig) };
    if rc == 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(format!("kill({pid}, {sig}) failed: {err}"))
}

pub fn token_path(project_root: &Path, fleet_id: &str, task_id: &str) -> io::Result<PathBuf> {
    check_id(task_id)?;
    Ok(fleet_token_dir(project_root, fleet_id)?.join(format!("{task_id}.json")))
}

/// Write-then-rename, so a reader never sees a torn token.
pub fn write_token(project_root: &Path, token: &WorkerToken) -> io::Result<()> {
    write_json_atomically(&token_path(project_root, &token.fleet_id, &token.task_id)?, token)
}

/// `Ok(None)` when there is no token (the worker never launched).
pub fn read_token(project_root: &Path, fleet_id: &str, task_id: &str) -> io::Result<Option<WorkerToken>> {
    read_json(&token_path(project_root, fleet_id, task_id)?)
}

pub fn write_owner_record(project_root: &Path, fleet_id: &str, owner: &ProcessIdentity) -> io::Result<()> {
    write_json_atomically(&owner_record_path(project_root, fleet_id), owner)
}

pub fn read_owner_record(project_root: &Path, fleet_id: &str) -> io::Result<Option<ProcessIdentity>> {
    read_json(&owner_record_path(project_root, fleet_id))
}

/// Written immediately BEFORE a worker is spawned and cleared once its token is on disk (or the
/// spawn failed). A marker with no token means a worker may be running that no token names: the
/// owner died between spawn and the token write. Recovery must not reset that task. Without the
/// marker, "no token" could not tell "never launched" from "launched, untracked" (Codex, review
/// of ee7984c).
pub fn launch_marker_path(project_root: &Path, fleet_id: &str, task_id: &str) -> io::Result<PathBuf> {
    check_id(task_id)?;
    Ok(fleet_token_dir(project_root, fleet_id)?.join(format!("{task_id}.launching")))
}

pub fn write_launch_marker(project_root: &Path, fleet_id: &str, task_id: &str) -> io::Result<()> {
    let path = launch_marker_path(project_root, fleet_id, task_id)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, b"")
}

/// Create the launch marker only if it does not exist: `Ok(true)` when this caller now owns the
/// launch, `Ok(false)` when another attempt already does. Exclusive create (O_EXCL) closes the
/// window between deciding to launch and launching, in which two attempts could both decide
/// "fresh" and both spawn the agent (Codex, review of 3871850).
///
/// The marker records its claimant (pid plus start time). A claimant that died after claiming and
/// before its worker's token existed cannot be mid-launch, so its marker is stale: it is removed
/// and the claim retried once. Without that, every later attempt blocked on it for good (Codex,
/// confirmation pass on d867c01). A live claimant's marker is never touched.
pub fn try_claim_launch(project_root: &Path, fleet_id: &str, task_id: &str) -> io::Result<bool> {
    let path = launch_marker_path(project_root, fleet_id, task_id)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let me = ProcessIdentity::current()
        .ok_or_else(|| io::Error::other("cannot read this process's identity for the launch marker"))?;
    for _ in 0..2 {
        match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(&serde_json::to_vec(&me).map_err(io::Error::other)?)?;
                f.sync_all()?;
                return Ok(true);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let claimant: Option<ProcessIdentity> =
                    fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
                match claimant {
                    Some(c) if !c.is_alive() && read_token(project_root, fleet_id, task_id)?.is_none() => {
                        tracing::warn!(fleet_id, task_id, claimant_pid = c.pid, "stale launch marker from a dead claimant; reclaiming");
                        fs::remove_file(&path)?;
                    }
                    // A live claimant, or a marker with no identity (written by the legacy
                    // engine or an older binary): not ours to reclaim.
                    _ => return Ok(false),
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(false)
}

pub fn clear_launch_marker(project_root: &Path, fleet_id: &str, task_id: &str) {
    if let Ok(path) = launch_marker_path(project_root, fleet_id, task_id) {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => tracing::error!(path = %path.display(), error = %e, "could not clear the launch marker; recovery will treat this task as untracked"),
        }
    }
}

/// Marks a fleet as run by the Temporal engine. Its tokens have no legacy owner record because
/// Temporal owns them (the activity and its heartbeat), so the legacy startup recovery must leave
/// the fleet alone: to it, a live Temporal fleet would look exactly like a crashed legacy one.
pub fn mark_temporal_engine(project_root: &Path, fleet_id: &str) -> io::Result<()> {
    let dir = fleet_token_dir(project_root, fleet_id)?;
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("ENGINE"), b"temporal\n")
}

pub fn is_temporal_engine(project_root: &Path, fleet_id: &str) -> bool {
    fleet_token_dir(project_root, fleet_id)
        .ok()
        .and_then(|d| fs::read_to_string(d.join("ENGINE")).ok())
        .is_some_and(|s| s.trim() == "temporal")
}

pub fn has_launch_marker(project_root: &Path, fleet_id: &str, task_id: &str) -> bool {
    launch_marker_path(project_root, fleet_id, task_id).is_ok_and(|p| p.exists())
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

/// Every worker token of `fleet_id`, with the file it came from. Unreadable tokens, and tokens
/// whose contents do not match their file name, are returned as errors so the caller can refuse
/// to call the fleet clean.
pub fn fleet_tokens(project_root: &Path, fleet_id: &str) -> Vec<(PathBuf, io::Result<WorkerToken>)> {
    let Ok(dir) = fleet_token_dir(project_root, fleet_id) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue; // a temp file mid-rename
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
        match read_json::<WorkerToken>(&path) {
            Ok(Some(t)) if t.fleet_id == fleet_id && t.task_id == stem => out.push((path, Ok(t))),
            Ok(Some(t)) => out.push((
                path,
                Err(io::Error::other(format!(
                    "token names fleet {} task {}, not the file it is in",
                    t.fleet_id, t.task_id
                ))),
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

    /// A marker whose claimant died before any token existed is reclaimed; a live claimant's is
    /// not. RED IF a dead claimant blocks the launch for good, or a live one loses its claim.
    #[test]
    fn a_dead_claimants_launch_marker_is_reclaimed_and_a_live_ones_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let marker = launch_marker_path(root, "f", "f-T-001").unwrap();
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();

        let dead = test_support::dead_owner();
        std::fs::write(&marker, serde_json::to_vec(&dead).unwrap()).unwrap();
        assert!(try_claim_launch(root, "f", "f-T-001").unwrap(), "a dead claimant's marker is reclaimed");
        let now: ProcessIdentity = serde_json::from_slice(&std::fs::read(&marker).unwrap()).unwrap();
        assert_eq!(now, ProcessIdentity::current().unwrap(), "the marker now names this claimant");

        // This process is alive, so a second claim must lose.
        assert!(!try_claim_launch(root, "f", "f-T-001").unwrap(), "a live claimant keeps its claim");
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
        assert!(matches!(t.verify(), Verification::Reused(_)));
        let err = t.terminate(Duration::from_millis(200)).unwrap_err();
        assert!(err.contains("refused"), "{err}");
        assert!(!reaped(&mut s, Duration::from_millis(300)), "the process must still be running");
    }

    /// A tampered token aims the signal at the test runner's own group. RED IF the refusal is
    /// removed: the SIGKILL then lands on this test binary and the whole run dies, which is a loud
    /// failure rather than a failed assertion (Antigravity, review of d434e38).
    #[test]
    fn a_wrong_pgid_is_refused() {
        let mut s = stub("sleep 30");
        let mut t = WorkerToken::for_spawned_child(s.0.id(), "f", "t", "codex").expect("token");
        // SAFETY: getpgrp has no preconditions.
        t.pgid = unsafe { libc::getpgrp() } as u32;
        assert!(t.signal_verified(libc::SIGKILL).is_err());
        assert!(!reaped(&mut s, Duration::from_millis(300)));
    }

    /// A child that did not get its own group gets no token, so nothing can ever signal the
    /// group it shares with its spawner.
    #[test]
    fn a_child_in_the_spawners_group_gets_no_token() {
        let mut child = Command::new("sleep").arg("30").spawn().expect("spawn");
        assert!(WorkerToken::for_spawned_child(child.id(), "f", "t", "codex").is_none());
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn sigterm_stops_a_cooperative_worker() {
        let s = stub("sleep 30");
        let t = WorkerToken::for_spawned_child(s.0.id(), "f", "t", "codex").expect("token");
        // Reap concurrently, the way a real owner (or launchd for an orphan) would.
        let pid = s.0.id();
        let reaper = std::thread::spawn(move || {
            let mut status = 0;
            // SAFETY: waitpid on our own child.
            unsafe { libc::waitpid(pid as i32, &mut status, 0) };
        });
        let started = Instant::now();
        assert_eq!(t.terminate(Duration::from_secs(3)), Ok(TerminateOutcome::Terminated));
        // RED IF terminate sleeps out the whole grace instead of polling: a worker that stops
        // on SIGTERM must not hold recovery for the full grace (Antigravity, review of d434e38).
        assert!(started.elapsed() < Duration::from_secs(1), "took {:?}", started.elapsed());
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
        assert_eq!(read_token(dir.path(), "fleet-1", "fleet-1-T-001").expect("read"), Some(t.clone()));
        assert_eq!(read_token(dir.path(), "fleet-1", "fleet-1-T-002").expect("read"), None);
        assert_eq!(fleet_tokens(dir.path(), "fleet-1").len(), 1);
        assert!(
            token_path(dir.path(), "fleet-1", "fleet-1-T-001").expect("path").starts_with(dir.path().join(".triumvirate/fleet-workers")),
            "tokens live in the project's .triumvirate, never in a worktree"
        );
    }

    /// RED IF: an id can steer a token path out of the token directory.
    #[test]
    fn ids_that_would_escape_the_token_directory_are_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        for bad in ["../x", "a/b", "..", "", "a\\b", "..\\x"] {
            assert!(token_path(dir.path(), "fleet-1", bad).is_err(), "{bad:?}");
            assert!(token_path(dir.path(), bad, "t").is_err(), "{bad:?}");
        }
    }

    /// A token copied into another task's file is not trusted.
    #[test]
    fn a_token_in_the_wrong_file_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let s = stub("sleep 30");
        let t = WorkerToken::for_spawned_child(s.0.id(), "fleet-1", "fleet-1-T-001", "grok").expect("token");
        let wrong = token_path(dir.path(), "fleet-1", "fleet-1-T-009").expect("path");
        fs::create_dir_all(wrong.parent().expect("dir")).expect("mkdir");
        fs::write(&wrong, serde_json::to_vec(&t).expect("json")).expect("write");
        let tokens = fleet_tokens(dir.path(), "fleet-1");
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].1.is_err());
    }
}

/// Real-process fixtures for the fleet tests: orphans like the ones a crashed owner leaves.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::process::{Command, Stdio};

    /// A true orphan: started through a shell that exits at once, so it is reparented to
    /// launchd (which reaps it, as it would a real orphan) and leads its own process group.
    /// `cwd` is where it runs. Returns its pid.
    pub fn orphan(script: &str, cwd: &Path) -> u32 {
        let out = Command::new("sh")
            .arg("-c")
            .arg(format!("set -m; sh -c '{script}' >/dev/null 2>&1 </dev/null & echo $!"))
            .current_dir(cwd)
            .stderr(Stdio::null())
            .output()
            .expect("spawn orphan");
        let pid: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().expect("orphan pid");
        // Let the trap (if any) install before anyone signals it.
        std::thread::sleep(Duration::from_millis(200));
        pid
    }

    /// The identity of a process that has exited: a stand-in for an owner that crashed.
    pub fn dead_owner() -> ProcessIdentity {
        let mut child = Command::new("sleep").arg("30").spawn().expect("spawn owner");
        let id = ProcessIdentity {
            pid: child.id(),
            start_time_us: proc_info(child.id()).expect("owner info").start_time_us,
        };
        child.kill().expect("kill owner");
        child.wait().expect("reap owner");
        assert!(!id.is_alive());
        id
    }

    /// A token for `pid` owned by `owner`, written under `project_root`.
    pub fn token_for(pid: u32, project_root: &Path, fleet_id: &str, task_id: &str, owner: ProcessIdentity) -> WorkerToken {
        let mut t = WorkerToken::for_spawned_child(pid, fleet_id, task_id, "codex").expect("token");
        t.owner = owner;
        write_token(project_root, &t).expect("write token");
        t
    }

    pub fn alive(pid: u32) -> bool {
        proc_info(pid).is_some()
    }

    /// Wait until `pid` is gone (an orphan is reaped by launchd shortly after it dies).
    pub fn gone_within(pid: u32, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if !alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// Kill a leftover fixture process regardless of what the test did.
    pub struct Reap(pub Vec<u32>);
    impl Drop for Reap {
        fn drop(&mut self) {
            for pid in &self.0 {
                // SAFETY: plain kill on a fixture pid this test started.
                unsafe { libc::kill(-(*pid as i32), libc::SIGKILL) };
                unsafe { libc::kill(*pid as i32, libc::SIGKILL) };
            }
        }
    }
}
