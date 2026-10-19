//! One process per seat, machine-wide (Seven Primes FEAT-010, REQ-053): an advisory flock on
//! `$TRIUMVIRATE_HOME/locks/<seat>`, released when the guard drops or the holding process dies.
//!
//! The fd pattern is pid.rs's (`PidFile::acquire`): open with create and no truncate, then `flock` on the raw fd, and
//! keep the `File` alive inside the guard. Two differences, both deliberate: this lock BLOCKS (a second call for the
//! same seat waits its turn instead of failing, because the second call is a legitimate queued request), and it never
//! writes into the file (the lock is the fd, not the contents, so there is nothing to go stale).
//!
//! The kernel drops a flock when the last fd on its open file description closes, which includes the holder being
//! killed. That is what makes a crashed dispatch release the seat without any cleanup code.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Holds the seat's lock for as long as it lives. Dropping it closes the fd, which releases the flock.
#[derive(Debug)]
pub struct SeatLockGuard {
    _file: File,
}

/// `$TRIUMVIRATE_HOME/locks/<seat>`.
fn lock_path(triumvirate_home: &Path, seat: &str) -> Result<PathBuf, String> {
    // A seat key becomes a file name, so it must not be able to name anything outside locks/.
    let ok = !seat.is_empty()
        && seat
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        return Err(format!(
            "invalid_seat_name: {seat:?} (lowercase ascii, digits and '-' only)"
        ));
    }
    Ok(triumvirate_home.join("locks").join(seat))
}

/// Open (creating if needed) the lock file without following symlinks, so a planted `locks/<seat>` link or a
/// `locks` directory that is itself a link cannot point the flock at a file outside `$TRIUMVIRATE_HOME/locks`.
fn open_lock_file(path: &Path) -> Result<File, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("seat_lock: cannot create {}: {e}", parent.display()))?;
        let meta = std::fs::symlink_metadata(parent)
            .map_err(|e| format!("seat_lock: cannot stat {}: {e}", parent.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!("seat_lock: {} is a symlink", parent.display()));
        }
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        // O_NOFOLLOW: opening a symlink fails (ELOOP) instead of opening its target.
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("seat_lock: cannot open {}: {e}", path.display()))
}

/// Block until this process holds the seat's lock.
#[allow(clippy::useless_conversion)] // libc constants have different widths on different targets (as in pid.rs)
pub fn acquire_seat_lock(triumvirate_home: &Path, seat: &str) -> Result<SeatLockGuard, String> {
    let path = lock_path(triumvirate_home, seat)?;
    let file = open_lock_file(&path)?;
    loop {
        // SAFETY: the fd belongs to `file`, which outlives this call and moves into the guard.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc == 0 {
            tracing::debug!(seat, path = %path.display(), "acquired seat lock");
            return Ok(SeatLockGuard { _file: file });
        }
        let err = std::io::Error::last_os_error();
        // A signal interrupted the wait; the lock was not taken, so wait again.
        if err.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        return Err(format!("seat_lock: flock {} failed: {err}", path.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// A non-blocking probe on a fresh fd: true when someone else holds the seat right now.
    fn held_elsewhere(home: &Path, seat: &str) -> bool {
        let file = open_lock_file(&lock_path(home, seat).unwrap()).unwrap();
        // SAFETY: probe fd owned by `file`, closed (and the probe lock released) when it drops.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        rc != 0
    }

    /// The second acquirer of one seat waits for the first guard to drop. Ordering is MEASURED from an event log
    /// written by both threads, not inferred from sleeps: B's "acquired" must come after A's "released".
    /// RED IF: the lock stops being exclusive, or stops blocking (a non-blocking lock would fail B, not order it).
    #[test]
    fn second_acquirer_blocks_until_the_first_guard_drops() {
        let home = tempfile::tempdir().unwrap();
        let log: Arc<Mutex<Vec<&'static str>>> = Arc::default();
        let guard_a = acquire_seat_lock(home.path(), "kimi").unwrap();
        log.lock().unwrap().push("a acquired");
        assert!(
            held_elsewhere(home.path(), "kimi"),
            "a held lock must be visible to another fd"
        );

        let (acquired_tx, acquired_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (home_b, log_b) = (home.path().to_path_buf(), Arc::clone(&log));
        let b = std::thread::spawn(move || {
            let guard = acquire_seat_lock(&home_b, "kimi").unwrap();
            log_b.lock().unwrap().push("b acquired");
            acquired_tx.send(()).unwrap();
            // Hold the seat until the main thread has checked that B really owns the flock.
            release_rx.recv().unwrap();
            drop(guard);
        });
        // While A holds the seat, B must not get through. A correct lock always times out here; this direction
        // cannot flake. (It cannot prove B reached flock; the ownership check below is what a fake guard fails.)
        assert!(
            acquired_rx
                .recv_timeout(Duration::from_millis(300))
                .is_err(),
            "B must not acquire while A holds the seat"
        );
        assert_eq!(*log.lock().unwrap(), ["a acquired"]);
        log.lock().unwrap().push("a released");
        drop(guard_a);
        acquired_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("B acquires once A releases");
        // Deterministic ownership check: with B's guard alive, another fd must see the seat held. A guard returned
        // without taking the flock (or a lock that is not exclusive) fails here regardless of timing.
        assert!(
            held_elsewhere(home.path(), "kimi"),
            "B's guard must actually hold the flock"
        );
        release_tx.send(()).unwrap();
        b.join().unwrap();
        assert_eq!(
            *log.lock().unwrap(),
            ["a acquired", "a released", "b acquired"]
        );
        assert!(
            !held_elsewhere(home.path(), "kimi"),
            "no holder left after both guards dropped"
        );
    }

    /// A planted symlink cannot redirect the lock outside locks/: neither `locks/<seat>` as a link to an outside file,
    /// nor `locks` itself as a link to an outside directory.
    /// RED IF: the lock file is opened through a symlink.
    #[test]
    fn symlinks_cannot_redirect_the_lock() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("victim");
        std::fs::write(&target, "x").unwrap();
        std::fs::create_dir_all(home.path().join("locks")).unwrap();
        std::os::unix::fs::symlink(&target, home.path().join("locks/kimi")).unwrap();
        let err = acquire_seat_lock(home.path(), "kimi").unwrap_err();
        assert!(err.starts_with("seat_lock: cannot open"), "{err}");

        let home2 = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), home2.path().join("locks")).unwrap();
        let err = acquire_seat_lock(home2.path(), "qwen").unwrap_err();
        assert!(err.ends_with("is a symlink"), "{err}");
        assert!(
            !outside.path().join("qwen").exists(),
            "nothing created outside"
        );
    }

    /// Different seats never wait on each other.
    #[test]
    fn different_seats_do_not_contend() {
        let home = tempfile::tempdir().unwrap();
        let _k = acquire_seat_lock(home.path(), "kimi").unwrap();
        let _q = acquire_seat_lock(home.path(), "qwen").unwrap();
        assert!(home.path().join("locks/kimi").exists() && home.path().join("locks/qwen").exists());
    }

    /// A process that holds the seat and is KILLED releases it: the kernel closes its fd. The holder is a python3
    /// child using fcntl.flock (macOS ships no flock CLI), which takes the same advisory lock as libc::flock.
    /// RED IF: the lock stops being a kernel flock on the fd (for example a pid file or a lock that needs cleanup).
    #[test]
    fn a_killed_holder_releases_the_seat() {
        let home = tempfile::tempdir().unwrap();
        let path = lock_path(home.path(), "mistral").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let script = "import fcntl,sys,time\nf=open(sys.argv[1],'a')\nfcntl.flock(f,fcntl.LOCK_EX)\nprint('locked',flush=True)\ntime.sleep(60)\n";
        let mut child = std::process::Command::new("python3")
            .args(["-I", "-c", script])
            .arg(&path)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect(
                "python3 must be on PATH: this test proves kill-release and must not be skipped",
            );
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(line.trim(), "locked");
        assert!(
            held_elsewhere(home.path(), "mistral"),
            "the child holds the seat"
        );

        child.kill().unwrap();
        child.wait().unwrap();
        // Acquire on a thread so a regression fails the test instead of hanging it.
        let (tx, rx) = mpsc::channel();
        let home_p = home.path().to_path_buf();
        std::thread::spawn(move || {
            tx.send(acquire_seat_lock(&home_p, "mistral").map(|_| ()))
                .unwrap()
        });
        let got = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("lock still held 10 s after the holder died");
        assert!(got.is_ok(), "{got:?}");
    }

    /// RED IF: a seat name can address a file outside locks/.
    #[test]
    fn seat_names_cannot_escape_the_locks_dir() {
        let home = tempfile::tempdir().unwrap();
        for bad in ["", "../x", "a/b", "Kimi", "kimi\0"] {
            let err = acquire_seat_lock(home.path(), bad).unwrap_err();
            assert!(err.starts_with("invalid_seat_name:"), "{bad:?}: {err}");
        }
    }
}
