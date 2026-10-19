// GAUNTLET filter suite for T-105, written blind from the spec (no implementation read).
//
// SPEC_AMBIGUITY: the spec does not say whether acquire_seat_lock creates the `locks` directory itself when it is missing; tests pre-create it.
// SPEC_AMBIGUITY: the spec gives no error text or code for a failed acquire (symlinked lock file, symlinked locks dir); tests assert is_err only.
// SPEC_AMBIGUITY: the spec does not constrain the seat name (empty, path separators, ".."); no test covers it.
// SPEC_AMBIGUITY: the spec does not say what happens when $TRIUMVIRATE_HOME does not exist; no test covers it.

use daemon_core::seat_lock::{acquire_seat_lock, SeatLockGuard};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn home_with_locks() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(home.path().join("locks")).expect("mkdir locks");
    home
}

fn lock_path(home: &Path, seat: &str) -> PathBuf {
    home.join("locks").join(seat)
}

// Independent probe of the flock: a python3 child tries a non-blocking exclusive flock on the path.
// Exit 0 = it got the lock (nobody holds it), exit 3 = lock held elsewhere.
fn probe_is_held(path: &Path) -> bool {
    let code = "import fcntl,sys\nf=open(sys.argv[1],'a')\ntry:\n fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)\nexcept OSError:\n sys.exit(3)\nsys.exit(0)\n";
    let st = Command::new("python3")
        .arg("-c")
        .arg(code)
        .arg(path)
        .status()
        .expect("python3 available");
    match st.code() {
        Some(0) => false,
        Some(3) => true,
        other => panic!("probe failed unexpectedly: {other:?}"),
    }
}

// Child process that takes a blocking exclusive flock, prints READY, then sleeps.
fn spawn_holder(path: &Path) -> Child {
    let code = "import fcntl,sys,time\nf=open(sys.argv[1],'a')\nfcntl.flock(f,fcntl.LOCK_EX)\nprint('READY',flush=True)\ntime.sleep(600)\n";
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(code)
        .arg(path)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn python3");
    let out = child.stdout.take().expect("stdout");
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(out), &mut line).expect("read READY");
    assert_eq!(line.trim(), "READY");
    child
}

// REQ-053: acquire returns a guard and creates the lock file at $TRIUMVIRATE_HOME/locks/<seat>.
#[test]
fn test_should_create_lock_file_at_locks_seat_path() {
    let home = home_with_locks();
    let guard = acquire_seat_lock(home.path(), "kimi").expect("acquire");
    assert!(lock_path(home.path(), "kimi").is_file());
    drop(guard);
}

// REQ-053: the lock is a real kernel flock visible to other processes while the guard lives.
#[test]
fn test_should_hold_flock_visible_to_other_process_while_guard_alive() {
    let home = home_with_locks();
    let guard = acquire_seat_lock(home.path(), "kimi").expect("acquire");
    assert!(probe_is_held(&lock_path(home.path(), "kimi")));
    drop(guard);
}

// REQ-053: dropping the guard releases the flock.
#[test]
fn test_should_release_flock_when_guard_dropped() {
    let home = home_with_locks();
    let guard = acquire_seat_lock(home.path(), "kimi").expect("acquire");
    assert!(probe_is_held(&lock_path(home.path(), "kimi")));
    drop(guard);
    assert!(!probe_is_held(&lock_path(home.path(), "kimi")));
}

// REQ-053: release also happens at the end of a scope (RAII).
#[test]
fn test_should_release_flock_at_end_of_scope() {
    let home = home_with_locks();
    {
        let _g: SeatLockGuard = acquire_seat_lock(home.path(), "qwen").expect("acquire");
        assert!(probe_is_held(&lock_path(home.path(), "qwen")));
    }
    assert!(!probe_is_held(&lock_path(home.path(), "qwen")));
}

// REQ-053: the same seat can be acquired again after the previous guard dropped (repeatable).
#[test]
fn test_should_reacquire_same_seat_after_drop_many_times() {
    let home = home_with_locks();
    for _ in 0..25 {
        let g = acquire_seat_lock(home.path(), "mistral").expect("acquire");
        assert!(probe_is_held(&lock_path(home.path(), "mistral")));
        drop(g);
        assert!(!probe_is_held(&lock_path(home.path(), "mistral")));
    }
}

// REQ-053: locks are per seat; holding one seat does not lock another.
#[test]
fn test_should_not_lock_other_seat_while_one_is_held() {
    let home = home_with_locks();
    let _a = acquire_seat_lock(home.path(), "kimi").expect("acquire kimi");
    let b = acquire_seat_lock(home.path(), "qwen").expect("acquire qwen without blocking");
    assert!(probe_is_held(&lock_path(home.path(), "kimi")));
    assert!(probe_is_held(&lock_path(home.path(), "qwen")));
    drop(b);
    assert!(probe_is_held(&lock_path(home.path(), "kimi")));
    assert!(!probe_is_held(&lock_path(home.path(), "qwen")));
}

// REQ-053 reality test: the second acquirer of the same seat blocks until the first guard drops (ordering, not sleep).
#[test]
fn test_should_block_second_acquirer_until_first_guard_drops() {
    let home = home_with_locks();
    let home_path = home.path().to_path_buf();
    let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    let first = acquire_seat_lock(&home_path, "kimi").expect("first");

    let (started_tx, started_rx) = mpsc::channel();
    let log2 = Arc::clone(&log);
    let hp = home_path.clone();
    let t = thread::spawn(move || {
        started_tx.send(()).unwrap();
        let g = acquire_seat_lock(&hp, "kimi").expect("second");
        log2.lock().unwrap().push("second_acquired");
        drop(g);
    });
    started_rx.recv().unwrap();
    // Give the second thread ample chance to (wrongly) get through; ordering is asserted via the log, not the delay.
    thread::sleep(Duration::from_millis(150));
    log.lock().unwrap().push("first_released");
    drop(first);
    t.join().unwrap();
    assert_eq!(*log.lock().unwrap(), vec!["first_released", "second_acquired"]);
}

// REQ-053: the second acquirer blocks rather than failing (it does not return while the seat is held).
#[test]
fn test_should_not_return_while_seat_is_held_by_another_guard() {
    let home = home_with_locks();
    let hp = home.path().to_path_buf();
    let first = acquire_seat_lock(&hp, "kimi").expect("first");
    let (tx, rx) = mpsc::channel();
    let hp2 = hp.clone();
    let t = thread::spawn(move || {
        let r = acquire_seat_lock(&hp2, "kimi");
        tx.send(r.is_ok()).unwrap();
    });
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "second call returned while seat was held");
    drop(first);
    assert!(rx.recv_timeout(Duration::from_secs(10)).expect("second call completes after release"));
    t.join().unwrap();
}

// REQ-053: mutual exclusion across many contenders (a non-atomic read-modify-write stays consistent).
#[test]
fn test_should_serialize_many_threads_on_one_seat() {
    let home = home_with_locks();
    let hp = home.path().to_path_buf();
    let counter = Arc::new(Mutex::new(0u32));
    let inside = Arc::new(Mutex::new(0u32));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let hp = hp.clone();
        let counter = Arc::clone(&counter);
        let inside = Arc::clone(&inside);
        handles.push(thread::spawn(move || {
            for _ in 0..5 {
                let g = acquire_seat_lock(&hp, "kimi").expect("acquire");
                {
                    let mut n = inside.lock().unwrap();
                    *n += 1;
                    assert_eq!(*n, 1, "two holders inside the critical section");
                }
                let v = *counter.lock().unwrap();
                thread::yield_now();
                *counter.lock().unwrap() = v + 1;
                *inside.lock().unwrap() -= 1;
                drop(g);
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(*counter.lock().unwrap(), 40);
}

// REQ-053 reality test: a child process holding the lock that is killed releases it; a waiter then acquires.
#[test]
fn test_should_release_lock_when_holding_child_process_is_killed() {
    let home = home_with_locks();
    let path = lock_path(home.path(), "kimi");
    fs::write(&path, b"").unwrap();
    let mut child = spawn_holder(&path);
    assert!(probe_is_held(&path), "child should hold the lock");

    let hp = home.path().to_path_buf();
    let (tx, rx) = mpsc::channel();
    let t = thread::spawn(move || {
        let g = acquire_seat_lock(&hp, "kimi").expect("acquire after child death");
        tx.send(()).unwrap();
        drop(g);
    });
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "acquired while child still held the lock");
    child.kill().expect("kill child");
    child.wait().expect("reap child");
    rx.recv_timeout(Duration::from_secs(10)).expect("lock released by process death");
    t.join().unwrap();
}

// REQ-053: a guard held by this process excludes a different process, and admits it after drop.
#[test]
fn test_should_exclude_other_process_while_guard_held_then_admit_it() {
    let home = home_with_locks();
    let path = lock_path(home.path(), "qwen");
    let g = acquire_seat_lock(home.path(), "qwen").expect("acquire");
    assert!(probe_is_held(&path));
    drop(g);
    assert!(!probe_is_held(&path));
}

// Spec doc: the lock never writes into the file (the lock is the fd, not the contents).
#[test]
fn test_should_leave_lock_file_empty_after_acquire() {
    let home = home_with_locks();
    let g = acquire_seat_lock(home.path(), "kimi").expect("acquire");
    assert_eq!(fs::metadata(lock_path(home.path(), "kimi")).unwrap().len(), 0);
    drop(g);
    assert_eq!(fs::metadata(lock_path(home.path(), "kimi")).unwrap().len(), 0);
}

// Spec doc: open with create and no truncate; pre-existing content is preserved.
#[test]
fn test_should_not_truncate_existing_lock_file() {
    let home = home_with_locks();
    let path = lock_path(home.path(), "kimi");
    fs::write(&path, b"keep-me").unwrap();
    let g = acquire_seat_lock(home.path(), "kimi").expect("acquire");
    drop(g);
    assert_eq!(fs::read(&path).unwrap(), b"keep-me");
}

// Spec doc: an existing lock file is reused (same inode) across acquires.
#[test]
fn test_should_reuse_existing_lock_file_across_acquires() {
    use std::os::unix::fs::MetadataExt;
    let home = home_with_locks();
    let g = acquire_seat_lock(home.path(), "kimi").expect("first");
    let ino1 = fs::metadata(lock_path(home.path(), "kimi")).unwrap().ino();
    drop(g);
    let g = acquire_seat_lock(home.path(), "kimi").expect("second");
    let ino2 = fs::metadata(lock_path(home.path(), "kimi")).unwrap().ino();
    drop(g);
    assert_eq!(ino1, ino2);
}

// Spec doc: acquiring a seat whose lock file pre-exists (left by a dead holder) succeeds immediately.
#[test]
fn test_should_acquire_when_stale_lock_file_exists_with_no_holder() {
    let home = home_with_locks();
    fs::write(lock_path(home.path(), "mistral"), b"").unwrap();
    let g = acquire_seat_lock(home.path(), "mistral").expect("acquire");
    assert!(probe_is_held(&lock_path(home.path(), "mistral")));
    drop(g);
}

// Spec doc: a planted symlink at locks/<seat> must not be followed; the target outside locks is not locked or modified.
#[test]
fn test_should_refuse_symlinked_lock_file_and_leave_target_untouched() {
    let home = home_with_locks();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("victim");
    fs::write(&target, b"secret").unwrap();
    std::os::unix::fs::symlink(&target, lock_path(home.path(), "kimi")).unwrap();
    let r = acquire_seat_lock(home.path(), "kimi");
    assert!(r.is_err(), "symlinked lock file must be refused");
    drop(r);
    assert_eq!(fs::read(&target).unwrap(), b"secret");
    assert!(!probe_is_held(&target));
}

// Spec doc: a dangling symlink at locks/<seat> must not cause the target to be created.
#[test]
fn test_should_refuse_dangling_symlink_and_not_create_target() {
    let home = home_with_locks();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("not-yet");
    std::os::unix::fs::symlink(&target, lock_path(home.path(), "kimi")).unwrap();
    let r = acquire_seat_lock(home.path(), "kimi");
    assert!(r.is_err(), "dangling symlink must be refused");
    assert!(!target.exists(), "target outside locks must not be created");
}

// Spec doc: a `locks` directory that is itself a symlink must not redirect the lock outside $TRIUMVIRATE_HOME/locks.
#[test]
fn test_should_refuse_symlinked_locks_directory() {
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), home.path().join("locks")).unwrap();
    let r = acquire_seat_lock(home.path(), "kimi");
    assert!(r.is_err(), "symlinked locks dir must be refused");
    assert!(!outside.path().join("kimi").exists(), "no lock file may be created through the link");
}

// REQ-053 error path: a refused acquire returns a non-empty String message (Result<_, String>).
#[test]
fn test_should_return_non_empty_error_string_on_refusal() {
    let home = home_with_locks();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("victim");
    fs::write(&target, b"x").unwrap();
    std::os::unix::fs::symlink(&target, lock_path(home.path(), "kimi")).unwrap();
    let err: String = match acquire_seat_lock(home.path(), "kimi") {
        Ok(_) => panic!("expected refusal"),
        Err(e) => e,
    };
    assert!(!err.trim().is_empty());
}

// REQ-053: different homes are independent (a lock in one TRIUMVIRATE_HOME does not block another).
#[test]
fn test_should_not_block_across_distinct_triumvirate_homes() {
    let h1 = home_with_locks();
    let h2 = home_with_locks();
    let _a = acquire_seat_lock(h1.path(), "kimi").expect("home1");
    let b = acquire_seat_lock(h2.path(), "kimi").expect("home2 must not block");
    assert!(probe_is_held(&lock_path(h2.path(), "kimi")));
    drop(b);
}

// ===== Round 2 (survivor feedback): seat-name validation and flock success =====

fn tree_listing(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            out.push(p.strip_prefix(root).unwrap().to_string_lossy().into_owned());
            if p.is_dir() {
                walk(&p, root, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn assert_invalid_seat(seat: &str) {
    let home = home_with_locks();
    let before = tree_listing(home.path());
    let err = match acquire_seat_lock(home.path(), seat) {
        Ok(_) => panic!("seat {seat:?} must be rejected"),
        Err(e) => e,
    };
    assert!(err.starts_with("invalid_seat_name:"), "seat {seat:?} gave error {err:?}");
    assert_eq!(tree_listing(home.path()), before, "seat {seat:?} must create nothing under home");
}

// Spec sharpening: empty seat is invalid.
#[test]
fn test_should_reject_empty_seat_name() {
    assert_invalid_seat("");
}

// Spec sharpening: uppercase is invalid (whole-name and mixed).
#[test]
fn test_should_reject_uppercase_seat_names() {
    assert_invalid_seat("Kimi");
    assert_invalid_seat("KIMI");
    assert_invalid_seat("kImi");
    assert_invalid_seat("kimI");
}

// Spec sharpening: '/' is invalid (traversal and nesting).
#[test]
fn test_should_reject_slash_in_seat_name() {
    assert_invalid_seat("a/b");
    assert_invalid_seat("/kimi");
    assert_invalid_seat("kimi/");
    assert_invalid_seat("/etc/passwd");
}

// Spec sharpening: '.' and '..' are invalid.
#[test]
fn test_should_reject_dot_names() {
    assert_invalid_seat(".");
    assert_invalid_seat("..");
    assert_invalid_seat("../kimi");
    assert_invalid_seat("ki.mi");
    assert_invalid_seat(".kimi");
    assert_invalid_seat("kimi.");
}

// Spec sharpening: '_' is invalid.
#[test]
fn test_should_reject_underscore_in_seat_name() {
    assert_invalid_seat("a_b");
    assert_invalid_seat("_kimi");
    assert_invalid_seat("kimi_");
}

// Spec sharpening: space and other whitespace are invalid.
#[test]
fn test_should_reject_whitespace_in_seat_name() {
    assert_invalid_seat(" ");
    assert_invalid_seat("a b");
    assert_invalid_seat("kimi ");
    assert_invalid_seat(" kimi");
    assert_invalid_seat("a\tb");
    assert_invalid_seat("a\nb");
}

// Spec sharpening: non-ASCII is invalid, including letters and digits outside ASCII.
#[test]
fn test_should_reject_non_ascii_seat_names() {
    assert_invalid_seat("kim\u{00ef}");
    assert_invalid_seat("\u{043a}imi");
    assert_invalid_seat("kimi\u{1F600}");
    assert_invalid_seat("\u{0663}");
}

// Spec sharpening: other punctuation and NUL are invalid.
#[test]
fn test_should_reject_other_punctuation_and_nul() {
    for s in ["a:b", "a*b", "a\\b", "a$b", "a@b", "a+b", "a=b", "a;b", "ki\0mi", "kimi~"] {
        assert_invalid_seat(s);
    }
}

// Spec sharpening: one invalid character among valid ones rejects the whole name.
#[test]
fn test_should_reject_seat_with_single_invalid_char_among_valid() {
    assert_invalid_seat("kimi-2/x");
    assert_invalid_seat("9-a_");
    assert_invalid_seat("-A-");
}

// Spec sharpening: a rejected name creates nothing even when `locks` is absent.
#[test]
fn test_should_create_nothing_when_rejecting_in_empty_home() {
    let home = tempfile::tempdir().unwrap();
    let err = match acquire_seat_lock(home.path(), "Bad/Seat") {
        Ok(_) => panic!("must be rejected"),
        Err(e) => e,
    };
    assert!(err.starts_with("invalid_seat_name:"), "{err:?}");
    assert_eq!(tree_listing(home.path()), Vec::<String>::new());
}

// Spec sharpening: a traversal name must not create a file outside locks.
#[test]
fn test_should_not_create_file_outside_locks_for_traversal_name() {
    let home = home_with_locks();
    let r = acquire_seat_lock(home.path(), "../escaped");
    assert!(r.is_err());
    assert!(!home.path().join("escaped").exists());
}

// Spec sharpening: lowercase letters, digits and '-' are valid.
#[test]
fn test_should_accept_lowercase_digits_and_hyphen_seat_names() {
    for s in ["kimi", "mistral", "qwen", "a", "z", "0", "9", "seat-1", "a-b-c", "-", "--", "-a", "a-", "0123456789", "abcdefghijklmnopqrstuvwxyz", "k2-0"] {
        let home = home_with_locks();
        let g = match acquire_seat_lock(home.path(), s) {
            Ok(g) => g,
            Err(e) => panic!("seat {s:?} must be accepted, got {e:?}"),
        };
        assert!(lock_path(home.path(), s).is_file(), "seat {s:?}");
        drop(g);
    }
}

// Spec sharpening: boundary characters of the allowed ranges are accepted, their ASCII neighbors rejected.
#[test]
fn test_should_enforce_range_boundaries_of_allowed_characters() {
    for s in ["a", "z", "0", "9", "-"] {
        let home = home_with_locks();
        assert!(acquire_seat_lock(home.path(), s).is_ok(), "{s:?} must be accepted");
    }
    for s in ["`", "{", "/", ":", "@", "[", "A", "Z", ",", "."] {
        assert_invalid_seat(s);
    }
}

// Spec sharpening: a successful acquire holds an exclusive flock (second acquirer blocks until drop).
#[test]
fn test_should_block_second_thread_after_successful_acquire_until_drop() {
    let home = home_with_locks();
    let hp = home.path().to_path_buf();
    let first = acquire_seat_lock(&hp, "seat-7").expect("first");
    let (tx, rx) = mpsc::channel();
    let hp2 = hp.clone();
    let t = thread::spawn(move || {
        let g = acquire_seat_lock(&hp2, "seat-7");
        tx.send(g.is_ok()).unwrap();
    });
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "second acquirer got through a held lock");
    drop(first);
    assert!(rx.recv_timeout(Duration::from_secs(10)).expect("second acquirer proceeds after drop"));
    t.join().unwrap();
}

// Spec sharpening: an Ok guard really holds the flock (result not inverted), none after drop.
#[test]
fn test_should_hold_exclusive_flock_after_ok_and_none_after_drop() {
    let home = home_with_locks();
    let path = lock_path(home.path(), "seat-8");
    fs::write(&path, b"").unwrap();
    assert!(!probe_is_held(&path));
    let g = acquire_seat_lock(home.path(), "seat-8").expect("acquire");
    assert!(probe_is_held(&path), "Ok guard must really hold the flock");
    drop(g);
    assert!(!probe_is_held(&path), "flock must be gone after drop");
}

// Spec sharpening: acquire waits for a foreign holder, then really holds the lock itself.
#[test]
fn test_should_wait_for_foreign_holder_then_really_hold_lock() {
    let home = home_with_locks();
    let path = lock_path(home.path(), "seat-9");
    fs::write(&path, b"").unwrap();
    let mut child = spawn_holder(&path);
    let hp = home.path().to_path_buf();
    let (tx, rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let t = thread::spawn(move || {
        let g = acquire_seat_lock(&hp, "seat-9").expect("acquire after foreign release");
        tx.send(()).unwrap();
        release_rx.recv().unwrap();
        drop(g);
    });
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "acquire returned while a foreign process held the lock");
    child.kill().unwrap();
    child.wait().unwrap();
    rx.recv_timeout(Duration::from_secs(10)).expect("acquire completes after foreign release");
    assert!(probe_is_held(&path), "guard returned Ok but does not hold the flock");
    release_tx.send(()).unwrap();
    t.join().unwrap();
    assert!(!probe_is_held(&path));
}
