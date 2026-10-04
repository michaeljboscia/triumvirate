//! Stage D, live: the MCP fleet tools on the Temporal engine (TRIUMVIRATE_FLEET_ENGINE=temporal).
//!
//! Real homebox server, queue triumvirate-fleet-exitcheck, a worker running in this test process,
//! stub agents through the real `triumvirate fleet-shim`. Ignored in CI; run explicitly:
//!   TRIUMVIRATE_SHIM_BIN=/abs/path/target/debug/triumvirate \
//!     cargo test -p mcp-tools --test temporal_fleet_live -- --ignored --test-threads=1

use std::{collections::HashMap, path::Path, sync::Arc, time::{Duration, Instant}};

use daemon_core::metrics::DaemonMetrics;
use fleet::{git_ops::RealGitOps, orchestrator::FleetOrchestrator};
use mcp_tools::fleet::{fleet_cancel, fleet_spawn, fleet_status};
use shared_types::{FleetCancelRequest, FleetSpawnRequest, FleetStatusRequest, FleetStatusResponse};
use tokio::sync::Mutex;

fn git(root: &Path, args: &[&str]) {
    let out = std::process::Command::new("git").arg("-C").arg(root).args(args).output().expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn repo(dir: &Path) -> std::path::PathBuf {
    let root = dir.join("repo");
    std::fs::create_dir_all(&root).expect("mkdir");
    git(&root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join(".gitignore"), ".triumvirate/\n").expect("ignore");
    git(&root, &["add", ".gitignore"]);
    git(&root, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "init"]);
    let _ = ledger::LedgerStore::open(root.clone()).expect("ledger");
    root
}

fn ledger_state(root: &Path, fleet_id: &str) -> String {
    rusqlite::Connection::open(root.join(".triumvirate/ledger.db"))
        .and_then(|c| c.query_row("SELECT state FROM fleets WHERE fleet_id = ?1", [fleet_id], |r| r.get(0)))
        .unwrap_or_else(|_| "none".to_string())
}

fn unused_factory(_: std::path::PathBuf) -> Result<FleetOrchestrator<RealGitOps>, String> {
    Err("the legacy orchestrator must not be used on the Temporal engine".to_string())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: needs the homebox server, the triumvirate-worker cert and TRIUMVIRATE_SHIM_BIN"]
async fn mcp_fleet_tools_drive_the_temporal_engine() {
    assert!(std::env::var("TRIUMVIRATE_SHIM_BIN").is_ok(), "set TRIUMVIRATE_SHIM_BIN");
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("invocations");
    // SAFETY: one test, run single-threaded by the instructions above.
    unsafe {
        std::env::set_var("TRIUMVIRATE_HOME", dir.path().join("home"));
        std::env::set_var("TRIUMVIRATE_FLEET_ENGINE", "temporal");
        std::env::set_var("TRIUMVIRATE_TEMPORAL_TASK_QUEUE", "triumvirate-fleet-exitcheck");
        std::env::set_var("TRIUMVIRATE_FLEET_KILL_GRACE_SECS", "2");
        // A stub that commits a file, or sleeps when SLEEPY exists in its worktree's parent dir.
        std::env::set_var(
            "TRIUMVIRATE_FLEET_STUB_SCRIPT",
            format!(
                "echo run >> {log}; if [ -f {sleepy} ]; then sleep 300; fi; f=\"stub-$(basename \"$PWD\").txt\"; echo hi > \"$f\"; git add \"$f\"; git -c user.email=s@s -c user.name=stub commit -q -m stub",
                log = log.display(),
                sleepy = dir.path().join("SLEEPY").display()
            ),
        );
    }
    let cfg = fleet_temporal::WorkerConfig::from_env().expect("cfg");
    fleet_temporal::spawn_worker_thread(cfg).expect("worker thread");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let states: Arc<Mutex<HashMap<String, FleetStatusResponse>>> = Arc::default();
    let metrics = DaemonMetrics::new().expect("metrics");

    // D1: wait=true runs to the end through the engine and merges.
    let root = repo(dir.path());
    let spawned = fleet_spawn(
        &states,
        &metrics,
        None,
        FleetSpawnRequest {
            project_root: Some(root.display().to_string()),
            agents: Some(vec!["stub".to_string()]),
            dry_run: Some(false),
            wait: Some(true),
            task_description: Some("stage D".to_string()),
        },
        unused_factory,
    )
    .await
    .expect("spawn");
    assert_eq!(spawned.state, "done", "{spawned:?}");
    assert!(spawned.plan.contains("engine: temporal"));
    assert_eq!(ledger_state(&root, &spawned.fleet_id), "done");
    let status = fleet_status(&states, FleetStatusRequest { fleet_id: spawned.fleet_id.clone() })
        .await
        .expect("status");
    assert_eq!(status.state, "done", "fleet_status reads the ledger the engine wrote");
    let merged = std::process::Command::new("git").arg("-C").arg(&root).args(["ls-files"]).output().expect("ls");
    assert!(String::from_utf8_lossy(&merged.stdout).contains("stub-"), "the stub's commit was merged");

    // D2: wait=false, then cancel through the MCP tool.
    std::fs::write(dir.path().join("SLEEPY"), "").expect("sleepy");
    let root2 = {
        let d = dir.path().join("second");
        std::fs::create_dir_all(&d).expect("mkdir");
        repo(&d)
    };
    let spawned2 = fleet_spawn(
        &states,
        &metrics,
        None,
        FleetSpawnRequest {
            project_root: Some(root2.display().to_string()),
            agents: Some(vec!["stub".to_string()]),
            dry_run: Some(false),
            wait: Some(false),
            task_description: Some("stage D cancel".to_string()),
        },
        unused_factory,
    )
    .await
    .expect("spawn 2");
    assert_eq!(spawned2.state, "spawning");
    // Wait for the worker's token: the stub is running.
    let token_dir = root2.join(".triumvirate/fleet-workers").join(&spawned2.fleet_id);
    let started = Instant::now();
    let token = loop {
        if let Some(t) = std::fs::read_dir(&token_dir).ok().and_then(|d| {
            d.flatten().find(|e| e.path().extension().is_some_and(|x| x == "json"))
        }) {
            break t.path();
        }
        assert!(started.elapsed() < Duration::from_secs(60), "the worker never started");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let pid: u32 = serde_json::from_slice::<serde_json::Value>(&std::fs::read(&token).expect("token"))
        .expect("json")["pid"]
        .as_u64()
        .expect("pid") as u32;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let cancelled = fleet_cancel(&states, &metrics, None, FleetCancelRequest { fleet_id: spawned2.fleet_id.clone() })
        .await
        .expect("cancel");
    assert!(cancelled.canceled, "{cancelled:?}");
    let started = Instant::now();
    loop {
        let gone = fleet::worker_token::proc_info(pid).is_none();
        if gone && ledger_state(&root2, &spawned2.fleet_id) == "cancelled" {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "after cancel: shim alive={} ledger={}",
            !gone,
            ledger_state(&root2, &spawned2.fleet_id)
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let runs = std::fs::read_to_string(&log).unwrap_or_default().lines().count();
    assert_eq!(runs, 2, "each fleet's stub ran exactly once");
}
