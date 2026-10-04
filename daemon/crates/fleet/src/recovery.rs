use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use ledger::LedgerStore;
use rusqlite::Connection;
use shared_types::RawEvent;

use crate::worker_token::{self, TerminateOutcome, Verification};

/// The reason every recovered fleet carries. A test asserts it; tooling may match on it.
pub const RECOVERY_REASON: &str = "crash recovery: stale fleet detected";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryResult {
    pub failed_fleets: Vec<String>,
    pub cleaned_worktrees: usize,
}

/// Which non-terminal fleets recovery may touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryOptions {
    /// A fleet with no owner record was spawned by a binary older than owner records. Its owner
    /// cannot be checked, so it may belong to a live process. Startup leaves it alone; only an
    /// explicit operator call includes it.
    pub include_ownerless: bool,
    /// SIGTERM to SIGKILL grace for each orphan.
    pub grace: Duration,
}

/// What recovery did in one project, fleet by fleet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaleFleetReport {
    /// Marked failed, tasks reset to pending. Their worktrees are untouched.
    pub recovered: Vec<String>,
    /// Owner process still running: a live fleet, not a crashed one.
    pub live_owner: Vec<String>,
    /// No owner record and `include_ownerless` was off.
    pub ownerless: Vec<String>,
    /// An orphan could not be proven stopped, so the fleet was left as it was (resetting its
    /// tasks would let a second claimant run a task the orphan is still running).
    pub blocked: Vec<(String, String)>,
    /// Orphaned workers stopped, across all fleets.
    pub orphans_stopped: usize,
}

/// Recover crashed fleets WITHOUT deleting anything. For each fleet left `spawning`, `running`,
/// `merging` or `recovery_required` whose owner process is gone:
///
/// 1. stop every orphaned worker its launch token can prove is still that worker (SIGTERM, grace,
///    SIGKILL, every result checked; a mismatched token is refused and nothing is signalled);
/// 2. only once every orphan is gone, mark the fleet failed and reset its claimed or in-progress
///    tasks to pending.
///
/// Order matters: resetting first lets a second claimant run the same task while the orphan keeps
/// spending (Codex, Grok, Gemini, review of 2026-10-03). Blocking (it sleeps through the grace):
/// call it from a blocking context.
pub fn recover_stale_fleets(project_root: &Path, opts: RecoveryOptions) -> anyhow::Result<StaleFleetReport> {
    if !project_root.is_absolute() {
        anyhow::bail!("project_root must be absolute");
    }
    let db_path = project_root.join(".triumvirate").join("ledger.db");
    // Never CREATE a ledger: the index outlives the repos it names (deleted temp dirs, moved
    // repos), and recovery must not leave empty databases behind in them.
    let conn = Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;

    let candidates: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT fleet_id FROM fleets
             WHERE state IN ('spawning', 'running', 'merging', 'recovery_required')",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<_, _>>()?
    };

    let mut report = StaleFleetReport::default();
    for fleet_id in candidates {
        let tokens = worker_token::fleet_tokens(project_root, &fleet_id);
        let owner = worker_token::read_owner_record(project_root, &fleet_id)?;
        let token_owner_alive = tokens
            .iter()
            .any(|(_, t)| t.as_ref().is_ok_and(|t| t.owner.is_alive()));
        if owner.is_some_and(|o| o.is_alive()) || token_owner_alive {
            report.live_owner.push(fleet_id);
            continue;
        }
        if owner.is_none() && !tokens.iter().any(|(_, t)| t.is_ok()) && !opts.include_ownerless {
            tracing::warn!(fleet_id = %fleet_id, "stale-looking fleet has no owner record (spawned by an older binary); leaving it alone");
            report.ownerless.push(fleet_id);
            continue;
        }

        // A task in flight with no token: never launched, or the owner died in the instant between
        // spawn and the token write. Tokens live outside the worktree, so a worker cannot have
        // deleted its own. Neither case can be proven here; say so instead of staying silent.
        let tokened: std::collections::BTreeSet<&str> = tokens
            .iter()
            .filter_map(|(_, t)| t.as_ref().ok().map(|t| t.task_id.as_str()))
            .collect();
        let untracked: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT task_id FROM tasks WHERE fleet_id = ?1 AND state IN ('claimed', 'in_progress')",
            )?;
            let rows = stmt.query_map([&fleet_id], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|t| !tokened.contains(t.as_str()))
                .collect()
        };
        // With no token, the launch marker decides: present means a spawn began and no token
        // ever named the process, so a worker may be running; resetting its task would let a
        // second claimant run it too. Absent means it was never launched.
        let mut problems: Vec<String> = untracked
            .iter()
            .filter(|t| worker_token::has_launch_marker(project_root, &fleet_id, t))
            .map(|t| {
                format!(
                    "{t}: launched but no launch token was written; a worker may still be running. \
                     Once you have confirmed none is, fleet_cancel takes the fleet out of recovery"
                )
            })
            .collect();
        let never_launched = untracked.len() - problems.len();
        if never_launched > 0 {
            tracing::info!(fleet_id = %fleet_id, never_launched, "in-flight tasks that were never launched");
        }

        for (path, token) in &tokens {
            let token = match token {
                Ok(t) => t,
                Err(e) => {
                    problems.push(format!("unreadable launch token in {}: {e}", path.display()));
                    continue;
                }
            };
            match token.verify() {
                Verification::Reused(why) => {
                    // The pid now belongs to someone else, so OUR worker is gone. Never signalled.
                    tracing::warn!(fleet_id = %fleet_id, task_id = %token.task_id, reason = %why, "launch token no longer matches; the worker is gone and the pid is not ours");
                    continue;
                }
                Verification::Refused(why) => {
                    // Possibly still our worker (it moved groups) or a malformed token: not proof
                    // that it is gone, so the fleet is not reset.
                    problems.push(format!("{}: {why}", token.task_id));
                    continue;
                }
                Verification::Live | Verification::Gone => {}
            }
            match token.terminate(opts.grace) {
                Ok(TerminateOutcome::AlreadyGone) => {}
                Ok(outcome) => {
                    tracing::warn!(fleet_id = %fleet_id, task_id = %token.task_id, pid = token.pid, ?outcome, "stopped an orphaned fleet worker");
                    report.orphans_stopped += 1;
                }
                Err(e) => problems.push(format!("{}: {e}", token.task_id)),
            }
        }
        if !problems.is_empty() {
            let why = problems.join("; ");
            tracing::error!(fleet_id = %fleet_id, problems = %why, "orphan not proven stopped; fleet left as it was");
            report.blocked.push((fleet_id, why));
            continue;
        }

        conn.execute(
            "UPDATE fleets
             SET state = 'failed',
                 completed_at = datetime('now'),
                 failure_reason = ?2
             WHERE fleet_id = ?1",
            rusqlite::params![fleet_id, RECOVERY_REASON],
        )?;
        conn.execute(
            "UPDATE tasks
             SET state = 'pending',
                 assigned_agent = NULL
             WHERE fleet_id = ?1
               AND state IN ('claimed', 'in_progress')",
            [&fleet_id],
        )?;
        report.recovered.push(fleet_id);
    }

    if !report.recovered.is_empty() {
        let store = LedgerStore::open(project_root.to_path_buf())?;
        for (idx, fleet_id) in report.recovered.iter().enumerate() {
            store.ingest_event(RawEvent {
                session_id: fleet_id.clone(),
                event_type: "fleet_recovery".to_string(),
                sequence: (idx + 1) as i64,
                timestamp: "2030-01-01T00:00:00Z".to_string(),
                payload_json: serde_json::json!({
                    "fleet_id": fleet_id,
                    "reason": RECOVERY_REASON
                })
                .to_string(),
            })?;
        }
    }
    Ok(report)
}

/// Summary of one startup pass over every project in the restart index.
#[derive(Debug, Clone, Default)]
pub struct StartupRecoverySummary {
    pub projects_scanned: usize,
    /// Index entries whose ledger no longer exists (deleted temp dirs, moved repos). Skipped.
    pub projects_missing: usize,
    pub recovered: Vec<String>,
    pub orphans_stopped: usize,
    pub live_owner: usize,
    pub ownerless: usize,
    pub blocked: Vec<(String, String)>,
    pub errors: Vec<(PathBuf, String)>,
}

/// The startup entry point: the non-deleting recovery over every project the index names.
/// Ownerless fleets are left alone. Blocking.
pub fn recover_fleets_at_startup(index: &Path, grace: Duration) -> StartupRecoverySummary {
    let mut summary = StartupRecoverySummary::default();
    for root in crate::index::project_roots_in(index) {
        if !root.join(".triumvirate").join("ledger.db").is_file() {
            summary.projects_missing += 1;
            continue;
        }
        summary.projects_scanned += 1;
        let opts = RecoveryOptions { include_ownerless: false, grace };
        match recover_stale_fleets(&root, opts) {
            Ok(r) => {
                summary.recovered.extend(r.recovered);
                summary.orphans_stopped += r.orphans_stopped;
                summary.live_owner += r.live_owner.len();
                summary.ownerless += r.ownerless.len();
                summary.blocked.extend(r.blocked);
            }
            Err(e) => summary.errors.push((root, e.to_string())),
        }
    }
    summary
}

/// The explicit operator path, unchanged in what it promises: recover every stale fleet whose
/// owner is gone (ownerless ones included) and DELETE their worktree directories. Startup never
/// calls this; deleting a worktree can destroy uncommitted work. Blocking.
pub fn recover_crashed_fleets(project_root: PathBuf) -> anyhow::Result<RecoveryResult> {
    let report = recover_stale_fleets(
        &project_root,
        RecoveryOptions { include_ownerless: true, grace: crate::orchestrator::fleet_kill_grace() },
    )?;
    let mut cleaned_worktrees = 0usize;
    let worktree_base = project_root.join(".triumvirate").join("worktrees");
    for fleet_id in &report.recovered {
        if worktree_base.exists() {
            for entry in fs::read_dir(&worktree_base)? {
                let entry = entry?;
                let path = entry.path();
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                if name.starts_with(&format!("{fleet_id}-")) && path.is_dir() {
                    fs::remove_dir_all(&path)?;
                    cleaned_worktrees += 1;
                }
            }
        }
    }
    Ok(RecoveryResult {
        failed_fleets: report.recovered,
        cleaned_worktrees,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ledger::LedgerStore;

    use super::recover_crashed_fleets;

    #[test]
    fn recovery_marks_failed_cleans_worktrees_and_logs_event() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool")).expect("spool");
        let _store = LedgerStore::open(project_root.clone()).expect("open ledger");
        let conn = rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))
            .expect("open sqlite");
        conn.execute(
            "INSERT INTO fleets (fleet_id, task_description, agent_composition, source_project_root, state)
             VALUES ('fleet-1', 'test', '{\"codex\":1}', ?1, 'running')",
            [project_root.display().to_string()],
        )
        .expect("insert fleet");
        conn.execute(
            "INSERT INTO tasks (fleet_id, task_id, title, assigned_agent, state, depends_on)
             VALUES ('fleet-1', 'T-001', 'task one', 'codex', 'claimed', '[]')",
            [],
        )
        .expect("insert claimed task");
        conn.execute(
            "INSERT INTO tasks (fleet_id, task_id, title, assigned_agent, state, depends_on)
             VALUES ('fleet-1', 'T-002', 'task two', 'gemini', 'in_progress', '[]')",
            [],
        )
        .expect("insert in progress task");
        conn.execute(
            "INSERT INTO tasks (fleet_id, task_id, title, assigned_agent, state, depends_on)
             VALUES ('fleet-1', 'T-003', 'task three', 'claude', 'done', '[]')",
            [],
        )
        .expect("insert done task");

        let wt = project_root
            .join(".triumvirate")
            .join("worktrees")
            .join("fleet-1-T-001-codex");
        fs::create_dir_all(&wt).expect("create worktree");
        fs::write(wt.join("file.txt"), "x").expect("write worktree file");

        let result = recover_crashed_fleets(project_root.clone()).expect("run recovery");
        assert_eq!(result.failed_fleets, vec!["fleet-1".to_string()]);
        assert!(result.cleaned_worktrees >= 1);
        assert!(!wt.exists());

        let fleet_state: (String, String) = conn
            .query_row(
                "SELECT state, failure_reason FROM fleets WHERE fleet_id = 'fleet-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read fleet state");
        assert_eq!(fleet_state.0, "failed");
        assert!(fleet_state.1.contains("crash recovery"));

        let reset_tasks: Vec<(String, Option<String>)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT state, assigned_agent
                     FROM tasks
                     WHERE fleet_id = 'fleet-1' AND task_id IN ('T-001', 'T-002')
                     ORDER BY task_id ASC",
                )
                .expect("prepare reset tasks query");
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))
                .expect("query reset tasks");
            let mut out = Vec::new();
            for row in rows {
                out.push(row.expect("row"));
            }
            out
        };
        assert_eq!(reset_tasks.len(), 2);
        assert_eq!(reset_tasks[0].0, "pending");
        assert!(reset_tasks[0].1.is_none());
        assert_eq!(reset_tasks[1].0, "pending");
        assert!(reset_tasks[1].1.is_none());

        let done_task: (String, Option<String>) = conn
            .query_row(
                "SELECT state, assigned_agent
                 FROM tasks
                 WHERE fleet_id = 'fleet-1' AND task_id = 'T-003'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read done task");
        assert_eq!(done_task.0, "done");
        assert_eq!(done_task.1.as_deref(), Some("claude"));

        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE event_type = 'fleet_recovery'",
                [],
                |row| row.get(0),
            )
            .expect("count recovery events");
        assert!(events >= 1);
    }

    use std::{path::Path, time::Duration};

    use crate::tasks::FleetTaskStore;
    use crate::worker_token::{self, test_support::*};

    use super::{RECOVERY_REASON, RecoveryOptions, recover_fleets_at_startup, recover_stale_fleets};

    const OPTS: RecoveryOptions = RecoveryOptions { include_ownerless: false, grace: Duration::from_millis(500) };

    /// A ledger with `fleet_id` running and one in-progress task per worktree name given.
    fn seed(root: &Path, fleet_id: &str, tasks: &[&str]) -> Vec<std::path::PathBuf> {
        fs::create_dir_all(root.join(".triumvirate")).expect("mkdir");
        let _ = LedgerStore::open(root.to_path_buf()).expect("ledger");
        let store = FleetTaskStore::new(root.to_path_buf()).expect("tasks");
        store.insert_fleet(fleet_id, "recovery test").expect("fleet");
        let conn = rusqlite::Connection::open(root.join(".triumvirate").join("ledger.db")).expect("db");
        conn.execute("UPDATE fleets SET state = 'running' WHERE fleet_id = ?1", [fleet_id]).expect("state");
        let mut worktrees = Vec::new();
        for t in tasks {
            let task_id = format!("{fleet_id}-{t}");
            store.insert_task(&task_id, fleet_id, t, &[]).expect("task");
            conn.execute(
                "UPDATE tasks SET state = 'in_progress', assigned_agent = 'codex' WHERE task_id = ?1",
                [&task_id],
            )
            .expect("task state");
            let wt = root.join(".triumvirate").join("worktrees").join(format!("{fleet_id}-{task_id}-codex"));
            fs::create_dir_all(&wt).expect("worktree");
            worktrees.push(wt);
        }
        worktrees
    }

    fn fleet_row(root: &Path, fleet_id: &str) -> (String, Option<String>) {
        let conn = rusqlite::Connection::open(root.join(".triumvirate").join("ledger.db")).expect("db");
        conn.query_row(
            "SELECT state, failure_reason FROM fleets WHERE fleet_id = ?1",
            [fleet_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row")
    }

    fn task_states(root: &Path, fleet_id: &str) -> Vec<String> {
        let conn = rusqlite::Connection::open(root.join(".triumvirate").join("ledger.db")).expect("db");
        let mut stmt = conn.prepare("SELECT state FROM tasks WHERE fleet_id = ?1 ORDER BY task_id").expect("q");
        stmt.query_map([fleet_id], |r| r.get(0)).expect("rows").map(|r| r.expect("row")).collect()
    }

    /// The crash case. The owner is dead; one orphan stops on SIGTERM, one ignores it; an
    /// operator's shell sits in a worktree with no token.
    /// RED IF: an orphan survives, the fleet is not failed with the existing reason, a task is not
    /// pending, a worktree is deleted, or the shell is signalled.
    #[test]
    fn a_dead_owners_orphans_are_stopped_then_the_fleet_fails_and_nothing_is_deleted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("project");
        let wts = seed(&root, "fleet-a", &["T-001", "T-002"]);
        let owner = worker_token::test_support::dead_owner();
        worker_token::write_owner_record(&root, "fleet-a", &owner).expect("owner");

        let polite = orphan("sleep 60", &wts[0]);
        let stubborn = orphan("trap \"\" TERM; while :; do sleep 1; done", &wts[1]);
        let shell = orphan("sleep 60", &wts[0]);
        let _reap = Reap(vec![polite, stubborn, shell]);
        token_for(polite, &root, "fleet-a", "fleet-a-T-001", owner);
        token_for(stubborn, &root, "fleet-a", "fleet-a-T-002", owner);

        let report = recover_stale_fleets(&root, OPTS).expect("recover");

        assert_eq!(report.recovered, vec!["fleet-a".to_string()]);
        assert_eq!(report.orphans_stopped, 2);
        assert!(gone_within(polite, Duration::from_secs(3)), "the polite orphan must be stopped");
        assert!(gone_within(stubborn, Duration::from_secs(3)), "SIGKILL escalation must stop the stubborn one");
        assert!(alive(shell), "a process in the worktree WITHOUT a token is never signalled");
        let (state, reason) = fleet_row(&root, "fleet-a");
        assert_eq!(state, "failed");
        assert_eq!(reason.as_deref(), Some(RECOVERY_REASON));
        assert_eq!(task_states(&root, "fleet-a"), vec!["pending", "pending"]);
        assert!(wts.iter().all(|w| w.is_dir()), "startup recovery never deletes a worktree");
    }

    /// The twin that makes the owner record matter. RED IF: a fleet whose owner is ALIVE (another
    /// session's `triumvirate mcp`) is touched: its worker killed or its tasks reset.
    #[test]
    fn a_live_owners_fleet_is_left_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("project");
        let wts = seed(&root, "fleet-live", &["T-001"]);
        let me = worker_token::ProcessIdentity::current().expect("me");
        worker_token::write_owner_record(&root, "fleet-live", &me).expect("owner");
        let worker = orphan("sleep 60", &wts[0]);
        let _reap = Reap(vec![worker]);
        token_for(worker, &root, "fleet-live", "fleet-live-T-001", me);

        let report = recover_stale_fleets(&root, OPTS).expect("recover");

        assert_eq!(report.live_owner, vec!["fleet-live".to_string()]);
        assert!(report.recovered.is_empty());
        assert!(alive(worker), "a live owner's worker must not be signalled");
        assert_eq!(fleet_row(&root, "fleet-live").0, "running");
        assert_eq!(task_states(&root, "fleet-live"), vec!["in_progress"]);
    }

    /// A pid reused by an unrelated process. RED IF: it is signalled.
    #[test]
    fn a_token_whose_pid_was_reused_is_never_signalled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("project");
        let wts = seed(&root, "fleet-reuse", &["T-001"]);
        let owner = worker_token::test_support::dead_owner();
        worker_token::write_owner_record(&root, "fleet-reuse", &owner).expect("owner");
        let stranger = orphan("sleep 60", &wts[0]);
        let _reap = Reap(vec![stranger]);
        let mut t = token_for(stranger, &root, "fleet-reuse", "fleet-reuse-T-001", owner);
        t.start_time_us += 1_000_000;
        worker_token::write_token(&root, &t).expect("rewrite");

        let report = recover_stale_fleets(&root, OPTS).expect("recover");

        assert!(alive(stranger), "a mismatched token must never be acted on");
        assert_eq!(report.orphans_stopped, 0);
        assert_eq!(report.recovered, vec!["fleet-reuse".to_string()], "our worker is gone, so the fleet is recoverable");
    }

    /// Fleets from before owner records cannot be checked. Startup leaves them alone.
    #[test]
    fn an_ownerless_fleet_is_skipped_at_startup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("project");
        seed(&root, "fleet-old", &["T-001"]);
        let report = recover_stale_fleets(&root, OPTS).expect("recover");
        assert_eq!(report.ownerless, vec!["fleet-old".to_string()]);
        assert_eq!(fleet_row(&root, "fleet-old").0, "running");
    }

    /// The startup entry point reads the index, skips a root whose ledger is gone WITHOUT
    /// creating one, and recovers the rest.
    #[test]
    fn startup_recovery_walks_the_index_and_never_creates_a_ledger() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("project");
        seed(&root, "fleet-s", &["T-001"]);
        worker_token::write_owner_record(&root, "fleet-s", &worker_token::test_support::dead_owner()).expect("owner");
        let gone = dir.path().join("gone");
        fs::create_dir_all(gone.join(".triumvirate")).expect("mkdir");
        let index = dir.path().join("fleets.json");
        crate::index::record_fleet_root_in(&index, "fleet-s", &root.display().to_string()).expect("index");
        crate::index::record_fleet_root_in(&index, "fleet-gone", &gone.display().to_string()).expect("index");

        let summary = recover_fleets_at_startup(&index, Duration::from_millis(500));

        assert_eq!(summary.recovered, vec!["fleet-s".to_string()]);
        assert_eq!(summary.projects_missing, 1);
        assert!(!gone.join(".triumvirate").join("ledger.db").exists(), "recovery must not create a ledger");
    }

    /// The spawn-to-token window. A marker with no token means a worker may be running that no
    /// token names. RED IF: recovery resets that task anyway (two claimants for one task).
    #[test]
    fn a_launch_marker_without_a_token_blocks_recovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("project");
        seed(&root, "fleet-m", &["T-001"]);
        worker_token::write_owner_record(&root, "fleet-m", &worker_token::test_support::dead_owner()).expect("owner");
        worker_token::write_launch_marker(&root, "fleet-m", "fleet-m-T-001").expect("marker");

        let report = recover_stale_fleets(&root, OPTS).expect("recover");

        assert_eq!(report.blocked.len(), 1, "{report:?}");
        assert!(report.blocked[0].1.contains("no launch token"), "{report:?}");
        assert_eq!(fleet_row(&root, "fleet-m").0, "running");
        assert_eq!(task_states(&root, "fleet-m"), vec!["in_progress"]);
    }

    /// Our worker, same pid and start time, but it moved to another process group (a CLI that
    /// calls setsid). RED IF: recovery calls it gone and resets its task, or signals anything.
    #[test]
    fn a_worker_that_left_its_group_blocks_recovery_and_is_not_signalled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("project");
        seed(&root, "fleet-g", &["T-001"]);
        worker_token::write_owner_record(&root, "fleet-g", &worker_token::test_support::dead_owner()).expect("owner");
        // A real process that is NOT in the group its token names: a child left in this test's
        // own group, with a token claiming it leads its own.
        let mut child = std::process::Command::new("sleep").arg("30").spawn().expect("spawn");
        let pid = child.id();
        let info = worker_token::proc_info(pid).expect("info");
        assert_ne!(info.pgid, pid);
        let t = worker_token::WorkerToken {
            pid,
            pgid: pid,
            start_time_us: info.start_time_us,
            fleet_id: "fleet-g".to_string(),
            task_id: "fleet-g-T-001".to_string(),
            agent: "codex".to_string(),
            owner: worker_token::test_support::dead_owner(),
        };
        worker_token::write_token(&root, &t).expect("token");

        let report = recover_stale_fleets(&root, OPTS).expect("recover");

        assert_eq!(report.blocked.len(), 1, "{report:?}");
        assert!(alive(pid), "a regrouped worker is never signalled");
        assert_eq!(task_states(&root, "fleet-g"), vec!["in_progress"]);
        let _ = child.kill();
        let _ = child.wait();
    }

    /// A live owner with NO tokens yet (a fleet still `spawning`). Isolates the owner-record
    /// check: the token check cannot rescue it. RED IF the owner-record check is removed
    /// (Antigravity, review of d434e38).
    #[test]
    fn a_live_owner_with_no_tokens_yet_is_left_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("project");
        seed(&root, "fleet-sp", &["T-001"]);
        worker_token::write_owner_record(&root, "fleet-sp", &worker_token::ProcessIdentity::current().expect("me")).expect("owner");
        let report = recover_stale_fleets(&root, OPTS).expect("recover");
        assert_eq!(report.live_owner, vec!["fleet-sp".to_string()]);
        assert_eq!(task_states(&root, "fleet-sp"), vec!["in_progress"]);
    }
}
