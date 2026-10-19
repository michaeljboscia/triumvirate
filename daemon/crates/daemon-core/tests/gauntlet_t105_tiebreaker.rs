// GAUNTLET tiebreaker suite for T-105, written blind from the spec (no implementation read).
// SPEC_AMBIGUITY: The contract requires symlink rejection but does not specify the exact error text.
// SPEC_AMBIGUITY: The public API has no nonblocking probe, so blocking is observed through channel ordering with a bounded receive.

use daemon_core::seat_lock::acquire_seat_lock;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

const CHILD_MODE_ENV: &str = "TRIUMVIRATE_T105_CHILD_MODE";
const CHILD_HOME_ENV: &str = "TRIUMVIRATE_T105_CHILD_HOME";
const CHILD_SEAT_ENV: &str = "TRIUMVIRATE_T105_CHILD_SEAT";
const CHILD_READY_LINE: &str = "T105_CHILD_LOCKED";

#[test]
fn creates_an_empty_lock_file_at_the_contract_path() {
    // REQ-053: acquire creates $TRIUMVIRATE_HOME/locks/<seat> without writing lock contents.
    let home = TempDir::new().expect("create isolated TRIUMVIRATE_HOME");
    let lock_path = home.path().join("locks").join("kimi");

    let guard = acquire_seat_lock(home.path(), "kimi").expect("acquire kimi seat lock");

    assert!(lock_path.is_file());
    assert_eq!(
        fs::read(&lock_path).expect("read created seat lock file"),
        Vec::<u8>::new()
    );

    drop(guard);
}

#[test]
fn a_second_acquisition_waits_until_the_first_guard_drops() {
    // REQ-053 reality test: two threads never overlap on one seat and the queued acquisition proceeds after guard drop.
    let home = TempDir::new().expect("create isolated TRIUMVIRATE_HOME");
    let home_path = home.path().to_path_buf();
    let first_guard =
        acquire_seat_lock(&home_path, "mistral").expect("acquire first mistral seat lock");

    let (attempted_tx, attempted_rx) = mpsc::channel();
    let (acquired_tx, acquired_rx) = mpsc::channel();

    let waiter = thread::spawn(move || {
        attempted_tx
            .send(())
            .expect("announce second acquisition attempt");
        let result = acquire_seat_lock(&home_path, "mistral");
        acquired_tx
            .send(result)
            .expect("report second acquisition result");
    });

    attempted_rx
        .recv()
        .expect("observe second acquisition attempt");

    // Orchestrator compile fix (GAUNTLET triage): the Result holds a guard, which has no PartialEq; same assertion.
    assert!(matches!(
        acquired_rx.recv_timeout(Duration::from_millis(250)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));

    drop(first_guard);

    let second_guard = acquired_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("second acquisition did not proceed after first guard dropped")
        .expect("second acquisition returned an error");

    drop(second_guard);
    waiter.join().expect("join second acquisition thread");
}

#[test]
fn dropping_a_guard_allows_reacquisition_without_rewriting_the_file() {
    // REQ-053 done condition: RAII guard release permits later exclusive acquisition of the same seat.
    let home = TempDir::new().expect("create isolated TRIUMVIRATE_HOME");
    let lock_path = home.path().join("locks").join("qwen");

    let first_guard = acquire_seat_lock(home.path(), "qwen").expect("acquire first qwen seat lock");
    drop(first_guard);

    let second_guard =
        acquire_seat_lock(home.path(), "qwen").expect("reacquire qwen seat lock after drop");

    assert_eq!(
        fs::read(&lock_path).expect("read reacquired seat lock file"),
        Vec::<u8>::new()
    );

    drop(second_guard);
}

#[cfg(unix)]
#[test]
fn rejects_a_lock_file_symlink_without_changing_its_target() {
    // REQ-053 security clause: locks/<seat> must not follow a planted file symlink outside the lock directory.
    use std::os::unix::fs::symlink;

    let home = TempDir::new().expect("create isolated TRIUMVIRATE_HOME");
    let outside = TempDir::new().expect("create isolated outside directory");
    let outside_file = outside.path().join("outside-lock-target");
    let sentinel = b"outside sentinel";

    fs::create_dir(home.path().join("locks")).expect("create locks directory");
    fs::write(&outside_file, sentinel).expect("create outside target");
    symlink(&outside_file, home.path().join("locks").join("kimi"))
        .expect("plant lock-file symlink");

    match acquire_seat_lock(home.path(), "kimi") {
        Ok(guard) => {
            drop(guard);
            panic!("acquisition followed a planted locks/<seat> symlink");
        }
        Err(error) => assert!(!error.is_empty()),
    }

    assert_eq!(
        fs::read(&outside_file).expect("read outside target"),
        sentinel
    );
}

#[cfg(unix)]
#[test]
fn rejects_a_symlinked_locks_directory_without_creating_an_external_lock() {
    // REQ-053 security clause: a symlinked locks directory must not redirect the flock outside TRIUMVIRATE_HOME.
    use std::os::unix::fs::symlink;

    let home = TempDir::new().expect("create isolated TRIUMVIRATE_HOME");
    let outside = TempDir::new().expect("create isolated outside directory");
    let external_lock = outside.path().join("qwen");

    symlink(outside.path(), home.path().join("locks")).expect("plant locks-directory symlink");

    match acquire_seat_lock(home.path(), "qwen") {
        Ok(guard) => {
            drop(guard);
            panic!("acquisition followed a planted locks directory symlink");
        }
        Err(error) => assert!(!error.is_empty()),
    }

    assert!(!external_lock.exists());
}

#[test]
fn killing_the_holding_process_releases_the_lock() {
    // REQ-053 reality test: process death closes the holding fd and releases the seat lock.
    let home = TempDir::new().expect("create isolated TRIUMVIRATE_HOME");
    let executable = env::current_exe().expect("resolve integration-test executable");

    let mut child = Command::new(executable)
        .arg("--ignored")
        .arg("--exact")
        .arg("child_process_holds_seat_lock_until_killed")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(CHILD_MODE_ENV, "hold-seat-lock")
        .env(CHILD_HOME_ENV, home.path())
        .env(CHILD_SEAT_ENV, "kimi")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn child lock holder");

    let stdout = child.stdout.take().expect("capture child stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();

    loop {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .expect("read child readiness output");
        assert_ne!(bytes, 0, "child exited before reporting lock acquisition");
        // Orchestrator fix (GAUNTLET triage): libtest prints "test <name> ... " with no newline before the child's output,
        // so the ready marker shares a line; an exact match never fired and the parent waited forever.
        if line.contains(CHILD_READY_LINE) {
            break;
        }
    }

    let home_path = home.path().to_path_buf();
    let (attempted_tx, attempted_rx) = mpsc::channel();
    let (acquired_tx, acquired_rx) = mpsc::channel();

    let waiter = thread::spawn(move || {
        attempted_tx
            .send(())
            .expect("announce parent acquisition attempt");
        let result = acquire_seat_lock(&home_path, "kimi");
        acquired_tx
            .send(result)
            .expect("report parent acquisition result");
    });

    attempted_rx
        .recv()
        .expect("observe parent acquisition attempt");

    // Orchestrator compile fix (GAUNTLET triage): the Result holds a guard, which has no PartialEq; same assertion.
    assert!(matches!(
        acquired_rx.recv_timeout(Duration::from_millis(250)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));

    child.kill().expect("kill child lock holder");
    let status = child.wait().expect("reap child lock holder");
    assert!(!status.success());

    let guard = acquired_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("lock remained blocked after holding process died")
        .expect("parent acquisition returned an error after process death");

    drop(guard);
    waiter.join().expect("join parent acquisition thread");
}

#[test]
#[ignore]
fn child_process_holds_seat_lock_until_killed() {
    // REQ-053 helper: hold the fd in a distinct process for the process-death reality test.
    if env::var(CHILD_MODE_ENV).as_deref() != Ok("hold-seat-lock") {
        return;
    }

    let home = env::var_os(CHILD_HOME_ENV).expect("child home environment variable");
    let seat = env::var(CHILD_SEAT_ENV).expect("child seat environment variable");
    let _guard =
        acquire_seat_lock(std::path::Path::new(&home), &seat).expect("child acquires seat lock");

    println!("{CHILD_READY_LINE}");
    std::io::stdout()
        .flush()
        .expect("flush child readiness output");

    loop {
        thread::park();
    }
}
