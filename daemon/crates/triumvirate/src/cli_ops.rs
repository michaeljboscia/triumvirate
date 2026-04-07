use daemon_core::{
    daemon_bind_addr as core_daemon_bind_addr,
    launchd_plist_path as core_launchd_plist_path,
    render_launch_agent_plist as core_render_launch_agent_plist,
    triumvirate_home_dir as core_triumvirate_home_dir,
};
use daemon_http::{fetch_daemon_status, fetch_daemon_status_snapshot};
use fallback_outbox::{count_pending_fallbacks, list_pending_fallback_paths};
use ledger::LedgerStore;
use mcp_bridge::{daemon_base_url, daemon_status_url};
use shared_types::{DaemonHealthResponse, DaemonStatusSnapshot};
use std::{fs, io::Write as _};

fn write_line_stdout(line: &str) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(line.as_bytes())?;
    stdout.write_all(b"\n")?;
    Ok(())
}

fn write_json_stdout(value: &serde_json::Value) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    Ok(())
}

pub(crate) fn run_install() -> anyhow::Result<()> {
    let home = core_triumvirate_home_dir()?;
    fs::create_dir_all(&home)?;
    let launch_agents = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("failed to determine user home directory"))?
        .join("Library/LaunchAgents");
    fs::create_dir_all(&launch_agents)?;

    let plist_path = core_launchd_plist_path()?;
    let exe_path = std::env::current_exe()?;
    let plist = core_render_launch_agent_plist(&exe_path.display().to_string(), &home.display().to_string());
    fs::write(&plist_path, plist)?;

    write_line_stdout(&format!("Installed launchd plist at {}", plist_path.display()))?;
    write_line_stdout(&format!("Load with: launchctl load {}", plist_path.display()))?;
    write_line_stdout("Start now with: launchctl start com.triumvirate.daemon-v2")?;
    Ok(())
}

pub(crate) fn run_uninstall() -> anyhow::Result<()> {
    let plist_path = core_launchd_plist_path()?;
    if plist_path.exists() {
        fs::remove_file(&plist_path)?;
        write_line_stdout(&format!("Removed launchd plist at {}", plist_path.display()))?;
    } else {
        write_line_stdout(&format!("No launchd plist found at {}", plist_path.display()))?;
    }
    write_line_stdout(&format!("Unload with: launchctl unload {}", plist_path.display()))?;
    Ok(())
}

pub(crate) async fn run_doctor() -> anyhow::Result<()> {
    let token_path = core_triumvirate_home_dir()?.join("daemon.token");
    let plist_path = core_launchd_plist_path()?;
    let daemon_health = fetch_daemon_status().await.ok();
    let daemon_bind_addr =
        core_daemon_bind_addr(std::env::var("TRIUMVIRATE_DAEMON_BIND_ADDR").ok().as_deref());
    let daemon_base_url = daemon_base_url();
    let daemon_status_url = daemon_status_url();
    let project_root = std::env::current_dir()?;
    let ledger_dir = project_root.join(".triumvirate");
    let ledger_db_path = ledger_dir.join("ledger.db");
    let spool_dir = ledger_dir.join("spool");
    let db_exists = ledger_db_path.exists();
    let spool_files = if spool_dir.exists() {
        fs::read_dir(&spool_dir)?
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .count()
    } else {
        0
    };

    let mut wal_enabled = false;
    let mut stale_jobs = 0_i64;
    let mut events_last_5min = 0_i64;
    if let Ok(store) = LedgerStore::open(project_root) {
        wal_enabled = store
            .journal_mode()
            .map(|mode| mode.eq_ignore_ascii_case("wal"))
            .unwrap_or(false);
        if let Ok(health) = store.health() {
            stale_jobs = health.stale_jobs;
            events_last_5min = health.events_last_5min;
        }
    }

    write_line_stdout("Daemon:")?;
    write_line_stdout(&format!("  token_file_exists: {}", token_path.exists()))?;
    write_line_stdout(&format!("  launchd_plist_exists: {}", plist_path.exists()))?;
    write_line_stdout(&format!("  daemon_bind_addr: {daemon_bind_addr}"))?;
    write_line_stdout(&format!("  daemon_base_url: {daemon_base_url}"))?;
    write_line_stdout(&format!("  daemon_status_url: {daemon_status_url}"))?;
    write_line_stdout(&format!("  daemon_reachable: {}", daemon_health.is_some()))?;

    write_line_stdout("Ledger:")?;
    write_line_stdout(&format!(
        "  db_file_exists: {} ({})",
        if db_exists { "PASS" } else { "FAIL" },
        ledger_db_path.display()
    ))?;
    write_line_stdout(&format!(
        "  wal_mode_enabled: {}",
        if wal_enabled { "PASS" } else { "FAIL" }
    ))?;
    write_line_stdout(&format!(
        "  spool_empty_or_draining: {} (files={spool_files})",
        if spool_files == 0 || events_last_5min > 0 {
            "PASS"
        } else {
            "FAIL"
        }
    ))?;
    write_line_stdout(&format!(
        "  stale_jobs: {} ({})",
        if stale_jobs == 0 { "PASS" } else { "FAIL" },
        stale_jobs
    ))?;
    write_line_stdout(&format!(
        "  last_event_recent: {} (events_last_5min={events_last_5min})",
        if events_last_5min > 0 { "PASS" } else { "FAIL" }
    ))?;
    Ok(())
}

pub(crate) async fn run_status() -> anyhow::Result<()> {
    let daemon_bind_addr =
        core_daemon_bind_addr(std::env::var("TRIUMVIRATE_DAEMON_BIND_ADDR").ok().as_deref());

    let health = fetch_daemon_status().await.ok();
    let snapshot = fetch_daemon_status_snapshot().await.ok();
    let pending_fallbacks = count_pending_fallbacks().unwrap_or(0);
    let fallback_tickets = list_pending_fallback_paths(10)
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>();
    let report = build_status_report(
        daemon_bind_addr,
        health,
        snapshot,
        pending_fallbacks,
        fallback_tickets,
    );

    write_json_stdout(&report)?;
    Ok(())
}

pub(crate) fn build_status_report(
    daemon_bind_addr: String,
    health: Option<DaemonHealthResponse>,
    snapshot: Option<DaemonStatusSnapshot>,
    pending_fallbacks: usize,
    fallback_tickets: Vec<String>,
) -> serde_json::Value {
    if let (Some(health), Some(snapshot)) = (health, snapshot) {
        let snapshot_value = serde_json::json!({
            "daemon_mode": snapshot.daemon_mode.unwrap_or_else(|| "incremental-dev".to_string()),
            "supported_agents": snapshot
                .supported_agents
                .unwrap_or_else(|| vec!["gemini".to_string(), "codex".to_string()]),
            "pending_fallbacks": snapshot.pending_fallbacks.unwrap_or(0),
            "fallback_tickets": snapshot.fallback_tickets.unwrap_or_default(),
            "daemon_bind_addr": snapshot.daemon_bind_addr.unwrap_or_else(|| daemon_bind_addr.clone()),
        });

        return serde_json::json!({
            "daemon_reachable": true,
            "daemon_bind_addr": daemon_bind_addr,
            "health": health,
            "snapshot": snapshot_value
        });
    }

    serde_json::json!({
        "daemon_reachable": false,
        "daemon_bind_addr": daemon_bind_addr.clone(),
        "health": null,
        "snapshot": {
            "daemon_mode": "incremental-dev",
            "supported_agents": ["gemini", "codex"],
            "pending_fallbacks": pending_fallbacks,
            "fallback_tickets": fallback_tickets,
            "daemon_bind_addr": daemon_bind_addr
        }
    })
}
