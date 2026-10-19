//! GAUNTLET net suite for T-105, written blind from the spec (no implementation read).
//!
//! Source of truth: T-105 SPEC.md (seat_lock public API), SPEC_v2.md REQ-053, TEST_PLAN.md row 053.
//!
//! SPEC_AMBIGUITY: the spec does not say the error text or code returned by `acquire_seat_lock` on failure
//!   (the signature is `Result<SeatLockGuard, String>`). Error-path tests assert `Err` with a non-empty message only.
//! SPEC_AMBIGUITY: the spec does not say whether `acquire_seat_lock` creates the `locks` directory when it is missing.
//!   Every test pre-creates `$TRIUMVIRATE_HOME/locks`.
//! SPEC_AMBIGUITY: the spec does not define seat-name validation (empty name, path separators, `..`). No test covers it.
//! SPEC_AMBIGUITY: the spec says a symlinked lock path or a symlinked `locks` directory must not point the flock outside
//!   `$TRIUMVIRATE_HOME/locks`, but does not say the call must return `Err`. Symlink tests assert `Err` plus "the outside
//!   target is untouched", the strongest form the spec supports.
//! SPEC_AMBIGUITY: the spec does not state the module path. Tests assume `daemon_core::seat_lock` is publicly reachable.

use daemon_core::seat_lock::{acquire_seat_lock, SeatLockGuard};
use std::io::{BufRead, BufReader};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

const CHILD_HOME_ENV: &str = "GAUNTLET_T105_CHILD_HOME";
const CHILD_SEAT_ENV: &str = "GAUNTLET_T105_CHILD_SEAT";
const CHILD_HELD_MARKER: &str = "GAUNTLET_T105_HELD";
const HANG_GUARD: Duration = Duration::from_secs(10);

/// A fresh TRIUMVIRATE_HOME with `locks/` already created.
fn home_with_locks() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(home.path().join("locks")).expect("create locks dir");
    home
}

/// Kills and reaps a child process when dropped, so a failed assertion cannot leak a sleeping holder.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// REQ-053: the lock lives at $TRIUMVIRATE_HOME/locks/<seat>.
#[test]
fn lock_file_is_created_at_home_locks_seat() {
    let home = home_with_locks();
    let _guard: SeatLockGuard = acquire_seat_lock(home.path(), "kimi").expect("acquire kimi");
    let lock_file: PathBuf = home.path().join("locks").join("kimi");
    assert!(lock_file.is_file(), "expected lock file at {}", lock_file.display());
}

// FEAT-010 / pid.rs pattern: the lock is the fd, never the contents. The file stays zero bytes while held and after release.
#[test]
fn lock_file_is_never_written() {
    let home = home_with_locks();
    let lock_file = home.path().join("locks").join("qwen");
    {
        let _guard = acquire_seat_lock(home.path(), "qwen").expect("acquire qwen");
        assert_eq!(std::fs::metadata(&lock_file).expect("stat while held").len(), 0);
    }
    assert_eq!(std::fs::metadata(&lock_file).expect("stat after release").len(), 0);
}

// REQ-053: one lock per seat, so different seats never block each other.
#[test]
fn different_seats_do_not_block_each_other() {
    let home = home_with_locks();
    let path = home.path().to_path_buf();
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let worker = thread::spawn(move || {
        let kimi = acquire_seat_lock(&path, "kimi");
        let mistral = acquire_seat_lock(&path, "mistral");
        let outcome = match (kimi, mistral) {
            (Ok(_k), Ok(_m)) => Ok(()),
            (Err(e), _) | (_, Err(e)) => Err(e),
        };
        let _ = tx.send(outcome);
    });
    let outcome = rx.recv_timeout(HANG_GUARD).expect("different seats must not block");
    assert_eq!(outcome, Ok(()));
    worker.join().expect("worker join");
}

// REQ-053 / TEST_PLAN 053: a second acquire on the same seat blocks until the first guard drops.
// Ordering is recorded by events, not by sleeps. The waiter can only log "second-acquired" after the holder's drop.
#[test]
fn second_acquire_blocks_until_first_guard_drops() {
    let home = home_with_locks();
    let path = home.path().to_path_buf();
    let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

    let first: SeatLockGuard = acquire_seat_lock(&path, "kimi").expect("first acquire");

    let (tx, rx) = mpsc::channel::<()>();
    let (waiter_path, waiter_log) = (path.clone(), Arc::clone(&log));
    let waiter = thread::spawn(move || {
        let guard = acquire_seat_lock(&waiter_path, "kimi").expect("second acquire");
        waiter_log.lock().expect("log lock").push("second-acquired");
        tx.send(()).expect("signal waiter");
        drop(guard);
    });

    log.lock().expect("log lock").push("first-released");
    drop(first);

    rx.recv_timeout(HANG_GUARD).expect("waiter must acquire after the first guard drops");
    waiter.join().expect("waiter join");
    assert_eq!(*log.lock().expect("log lock"), vec!["first-released", "second-acquired"]);
}

// REQ-053 / concurrency_limit = 1: eight contending threads never hold the seat at the same time.
#[test]
fn concurrency_limit_is_one_under_contention() {
    let home = home_with_locks();
    let path = home.path().to_path_buf();
    let active = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let (p, active, max_seen, completed) =
                (path.clone(), Arc::clone(&active), Arc::clone(&max_seen), Arc::clone(&completed));
            thread::spawn(move || {
                let guard = acquire_seat_lock(&p, "deepseek").expect("contended acquire");
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_seen.fetch_max(now, Ordering::SeqCst);
                active.fetch_sub(1, Ordering::SeqCst);
                completed.fetch_add(1, Ordering::SeqCst);
                drop(guard);
            })
        })
        .collect();
    for h in handles {
        h.join().expect("contender join");
    }
    assert_eq!(max_seen.load(Ordering::SeqCst), 1, "more than one holder at once");
    assert_eq!(completed.load(Ordering::SeqCst), 8, "every contender must acquire and release");
}

// REQ-053 / FEAT-010 release on drop: the same thread can take the seat again after its guard drops.
#[test]
fn guard_drop_releases_seat_for_reacquire() {
    let home = home_with_locks();
    let path = home.path().to_path_buf();
    let (tx, rx) = mpsc::channel::<()>();
    let worker = thread::spawn(move || {
        let first = acquire_seat_lock(&path, "grok").expect("first acquire");
        drop(first);
        let _second = acquire_seat_lock(&path, "grok").expect("reacquire after drop");
        let _ = tx.send(());
    });
    rx.recv_timeout(HANG_GUARD).expect("reacquire after drop must not block");
    worker.join().expect("worker join");
}

// REQ-053: the lock is keyed by $TRIUMVIRATE_HOME, so two homes never contend for the same seat.
#[test]
fn same_seat_in_different_homes_does_not_block() {
    let home_a = home_with_locks();
    let home_b = home_with_locks();
    let (a, b) = (home_a.path().to_path_buf(), home_b.path().to_path_buf());
    let (tx, rx) = mpsc::channel::<()>();
    let worker = thread::spawn(move || {
        let _ga = acquire_seat_lock(&a, "kimi").expect("home a acquire");
        let _gb = acquire_seat_lock(&b, "kimi").expect("home b acquire");
        let _ = tx.send(());
    });
    rx.recv_timeout(HANG_GUARD).expect("separate homes must not block each other");
    worker.join().expect("worker join");
}

// REQ-053 / FEAT-010 kernel release: a holder process that is killed releases the seat with no cleanup code.
// The holder is this test binary re-run with --exact on the child test below. It only runs when the env vars are set.
#[test]
fn gauntlet_t105_child_holder() {
    let (Ok(home), Ok(seat)) = (std::env::var(CHILD_HOME_ENV), std::env::var(CHILD_SEAT_ENV)) else {
        return;
    };
    let _guard = acquire_seat_lock(Path::new(&home), &seat).expect("child acquire");
    println!("{CHILD_HELD_MARKER}");
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

#[test]
fn killed_holder_process_releases_seat() {
    let home = home_with_locks();
    let path = home.path().to_path_buf();

    let exe = std::env::current_exe().expect("current test exe");
    let child = Command::new(exe)
        .args(["--exact", "gauntlet_t105_child_holder", "--nocapture", "--test-threads=1"])
        .env(CHILD_HOME_ENV, &path)
        .env(CHILD_SEAT_ENV, "kimi")
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn holder child");
    let mut child = KillOnDrop(child);

    // Wait until the child reports that it holds the lock (event-driven, not a sleep).
    let stdout = child.0.stdout.take().expect("child stdout");
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read child stdout");
        assert!(n > 0, "child exited before it held the lock");
        if line.contains(CHILD_HELD_MARKER) {
            break;
        }
    }

    // Start a waiter while the child holds the seat.
    let (tx, rx) = mpsc::channel::<()>();
    let waiter_path = path.clone();
    let waiter = thread::spawn(move || {
        let _g = acquire_seat_lock(&waiter_path, "kimi").expect("waiter acquire");
        let _ = tx.send(());
    });

    // Negative probe: while the live child holds the seat, the waiter must not get it.
    assert!(
        rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "waiter acquired the seat while the child still holds it"
    );

    // Kill the holder. The kernel must release the flock, so the waiter gets the seat.
    child.0.kill().expect("kill holder");
    child.0.wait().expect("reap holder");
    rx.recv_timeout(HANG_GUARD).expect("seat must be released when the holder is killed");
    waiter.join().expect("waiter join");
}

// REQ-053 / spec: symlinked lock file must not point the flock at a file outside $TRIUMVIRATE_HOME/locks.
#[test]
fn symlinked_seat_file_is_refused_and_outside_target_untouched() {
    let home = home_with_locks();
    let outside = tempfile::tempdir().expect("outside tempdir");
    let target = outside.path().join("target");
    std::fs::write(&target, "outside-original").expect("write outside target");
    symlink(&target, home.path().join("locks").join("kimi")).expect("plant symlink");

    let err = acquire_seat_lock(home.path(), "kimi").err().expect("symlinked seat file must be refused");
    assert!(!err.is_empty(), "refusal must carry a message");
    assert_eq!(std::fs::read_to_string(&target).expect("read outside target"), "outside-original");
}

// REQ-053 / spec: a locks directory that is itself a link must not send the lock outside $TRIUMVIRATE_HOME/locks.
#[test]
fn symlinked_locks_directory_is_refused_and_outside_dir_untouched() {
    let home = tempfile::tempdir().expect("home tempdir");
    let outside = tempfile::tempdir().expect("outside tempdir");
    symlink(outside.path(), home.path().join("locks")).expect("plant locks symlink");

    let err = acquire_seat_lock(home.path(), "kimi").err().expect("symlinked locks dir must be refused");
    assert!(!err.is_empty(), "refusal must carry a message");
    let entries = std::fs::read_dir(outside.path()).expect("read outside dir").count();
    assert_eq!(entries, 0, "no lock file may be created outside the locks directory");
}
