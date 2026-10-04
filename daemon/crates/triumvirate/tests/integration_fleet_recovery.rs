//! Startup fleet recovery, through the real `triumvirate daemon` binary.
//!
//! The scenario from the legacy-fix handoff: a fleet's owner dies while its workers run on,
//! then a daemon starts. Here the owner IS a daemon: daemon #1 is started, named as the fleet's
//! owner, and SIGKILLed; daemon #2 starts and must recover the fleet.
//!
//! This is the proof that startup actually invokes the non-deleting recovery: nothing but
//! `run_daemon` runs in daemon #2, and the worktrees must still exist afterwards.
//!
//! Isolated: its own TRIUMVIRATE_HOME and HOME, a free port, no prewarm, no PostHog, no OTEL.
//! No agent is called. Stubs are `sh` and `sleep`.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use fleet::worker_token::{self, ProcessIdentity, WorkerToken};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port()
}

fn start_daemon(home: &Path, log: &Path) -> Child {
    let out = fs::OpenOptions::new().create(true).append(true).open(log).expect("log");
    Command::new(env!("CARGO_BIN_EXE_triumvirate"))
        .arg("daemon")
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("TRIUMVIRATE_HOME", home.join(".triumvirate"))
        .env("TRIUMVIRATE_DAEMON_BIND_ADDR", format!("127.0.0.1:{}", free_port()))
        .env("TRIUMVIRATE_DAEMON_PREWARM", "0")
        .env("TRIUMVIRATE_FLEET_KILL_GRACE_SECS", "1")
        .stdin(Stdio::null())
        .stdout(out.try_clone().expect("dup"))
        .stderr(out)
        .spawn()
        .expect("start daemon")
}

fn identity(pid: u32) -> ProcessIdentity {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(i) = worker_token::proc_info(pid) {
            return ProcessIdentity { pid, start_time_us: i.start_time_us };
        }
        assert!(Instant::now() < deadline, "no process info for {pid}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A true orphan in its own process group, running in `cwd`.
fn orphan(script: &str, cwd: &Path) -> u32 {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("set -m; sh -c '{script}' >/dev/null 2>&1 </dev/null & echo $!"))
        .current_dir(cwd)
        .output()
        .expect("spawn orphan");
    std::thread::sleep(Duration::from_millis(200));
    String::from_utf8_lossy(&out.stdout).trim().parse().expect("pid")
}

fn alive(pid: u32) -> bool {
    worker_token::proc_info(pid).is_some()
}

struct Cleanup {
    pids: Vec<u32>,
    daemons: Vec<Child>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        for pid in &self.pids {
            let _ = worker_token::signal_group(*pid, 9);
        }
        for d in &mut self.daemons {
            let _ = d.kill();
            let _ = d.wait();
        }
    }
}

/// A ledger with `fleet_id` running and one in-progress task per name. Returns the worktrees.
fn seed(root: &Path, fleet_id: &str, tasks: &[&str]) -> Vec<PathBuf> {
    fs::create_dir_all(root.join(".triumvirate")).expect("mkdir");
    let _ = ledger::LedgerStore::open(root.to_path_buf()).expect("ledger");
    let store = fleet::tasks::FleetTaskStore::new(root.to_path_buf()).expect("tasks");
    store.insert_fleet(fleet_id, "restart test").expect("fleet");
    let conn = rusqlite::Connection::open(root.join(".triumvirate/ledger.db")).expect("db");
    conn.execute("UPDATE fleets SET state = 'running' WHERE fleet_id = ?1", [fleet_id]).expect("state");
    tasks
        .iter()
        .map(|t| {
            let task_id = format!("{fleet_id}-{t}");
            store.insert_task(&task_id, fleet_id, t, &[]).expect("task");
            conn.execute(
                "UPDATE tasks SET state = 'in_progress', assigned_agent = 'codex' WHERE task_id = ?1",
                [&task_id],
            )
            .expect("task state");
            let wt = root.join(".triumvirate/worktrees").join(format!("{fleet_id}-{task_id}-codex"));
            fs::create_dir_all(&wt).expect("worktree");
            wt
        })
        .collect()
}

fn token(pid: u32, root: &Path, fleet_id: &str, task_id: &str, owner: ProcessIdentity) {
    let mut t = WorkerToken::for_spawned_child(pid, fleet_id, task_id, "codex").expect("token");
    t.owner = owner;
    worker_token::write_token(root, &t).expect("write token");
}

fn fleet_state(root: &Path, fleet_id: &str) -> (String, Option<String>) {
    rusqlite::Connection::open(root.join(".triumvirate/ledger.db"))
        .expect("db")
        .query_row(
            "SELECT state, failure_reason FROM fleets WHERE fleet_id = ?1",
            [fleet_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row")
}

fn task_states(root: &Path, fleet_id: &str) -> Vec<String> {
    let conn = rusqlite::Connection::open(root.join(".triumvirate/ledger.db")).expect("db");
    let mut stmt = conn.prepare("SELECT state FROM tasks WHERE fleet_id = ?1 ORDER BY task_id").expect("q");
    stmt.query_map([fleet_id], |r| r.get(0)).expect("rows").map(|r| r.expect("row")).collect()
}

/// RED IF: the restarted daemon does not recover the crashed fleet, recovers it before its
/// orphans are gone, deletes a worktree, or touches a fleet whose owner is alive, an ownerless
/// fleet, or a shell sitting in a worktree.
#[test]
fn a_restarted_daemon_stops_orphans_first_then_fails_the_fleet_and_keeps_worktrees() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    fs::create_dir_all(home.join(".triumvirate")).expect("home");
    let log1 = dir.path().join("daemon-1.log");
    let log = dir.path().join("daemon-2.log");
    let index = home.join(".triumvirate/fleets.json");
    let mut cleanup = Cleanup { pids: Vec::new(), daemons: Vec::new() };

    // Daemon #1: the owner that will crash.
    let first = start_daemon(&home, &log1);
    let owner = identity(first.id());
    cleanup.daemons.push(first);

    // The crashed fleet: two workers (one ignores SIGTERM) and a bystander shell.
    let crashed = dir.path().join("crashed");
    let wts = seed(&crashed, "fleet-crashed", &["T-001", "T-002"]);
    worker_token::write_owner_record(&crashed, "fleet-crashed", &owner).expect("owner record");
    let polite = orphan("sleep 120", &wts[0]);
    let stubborn = orphan("trap \"\" TERM; while :; do sleep 1; done", &wts[1]);
    let bystander = orphan("sleep 120", &wts[0]);
    cleanup.pids.extend([polite, stubborn, bystander]);
    token(polite, &crashed, "fleet-crashed", "fleet-crashed-T-001", owner);
    token(stubborn, &crashed, "fleet-crashed", "fleet-crashed-T-002", owner);

    // A fleet whose owner is alive (this test process): another session's live fleet.
    let live = dir.path().join("live");
    let live_wts = seed(&live, "fleet-live", &["T-001"]);
    let me = ProcessIdentity::current().expect("me");
    worker_token::write_owner_record(&live, "fleet-live", &me).expect("owner record");
    let live_worker = orphan("sleep 120", &live_wts[0]);
    cleanup.pids.push(live_worker);
    token(live_worker, &live, "fleet-live", "fleet-live-T-001", me);

    // A fleet from before owner records.
    let old = dir.path().join("old");
    seed(&old, "fleet-old", &["T-001"]);

    for (id, root) in [("fleet-crashed", &crashed), ("fleet-live", &live), ("fleet-old", &old)] {
        fleet::index::record_fleet_root_in(&index, id, &root.display().to_string()).expect("index");
    }

    // The crash.
    let mut first = cleanup.daemons.pop().expect("first");
    first.kill().expect("SIGKILL daemon #1");
    first.wait().expect("reap daemon #1");
    assert!(!owner.is_alive());

    // Daemon #2. Watch the ledger from the moment it starts: the first time the fleet reads
    // `failed`, both orphans must already be gone.
    cleanup.daemons.push(start_daemon(&home, &log));
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut failed_seen = false;
    let mut reset_seen = false;
    loop {
        if !failed_seen && fleet_state(&crashed, "fleet-crashed").0 == "failed" {
            failed_seen = true;
            assert!(!alive(polite), "the fleet was failed while an orphan was still running");
            assert!(!alive(stubborn), "the fleet was failed while the SIGTERM-ignoring orphan was still running");
        }
        // The task reset is the step that lets a second claimant in, so it must come after the
        // orphans too, not merely the fleet state (Antigravity, review of d434e38).
        if !reset_seen && task_states(&crashed, "fleet-crashed").iter().any(|s| s == "pending") {
            reset_seen = true;
            assert!(!alive(polite) && !alive(stubborn), "a task was reset to pending while an orphan was still running");
        }
        if fs::read_to_string(&log).unwrap_or_default().contains("startup fleet recovery finished") {
            break;
        }
        assert!(Instant::now() < deadline, "daemon #2 never finished startup recovery:\n{}", fs::read_to_string(&log).unwrap_or_default());
        std::thread::sleep(Duration::from_millis(25));
    }

    let (state, reason) = fleet_state(&crashed, "fleet-crashed");
    assert_eq!(state, "failed");
    assert_eq!(reason.as_deref(), Some("crash recovery: stale fleet detected"));
    assert_eq!(task_states(&crashed, "fleet-crashed"), vec!["pending", "pending"]);
    assert!(wts.iter().all(|w| w.is_dir()), "startup must never delete a worktree");
    assert!(alive(bystander), "a shell in the worktree without a token is never signalled");

    assert_eq!(fleet_state(&live, "fleet-live").0, "running", "a live owner's fleet is not recovered");
    assert!(alive(live_worker), "a live owner's worker is never signalled");
    assert_eq!(fleet_state(&old, "fleet-old").0, "running", "an ownerless fleet is left alone");
}
