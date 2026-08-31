use daemon_core::{
    daemon_bind_addr as core_daemon_bind_addr,
    launchd_plist_path as core_launchd_plist_path,
    render_launch_agent_plist as core_render_launch_agent_plist,
    triumvirate_home_dir as core_triumvirate_home_dir,
};
use daemon_http::{fetch_daemon_status, fetch_daemon_status_snapshot};
use fallback_outbox::{count_pending_fallbacks, list_pending_fallback_paths};
use ledger::LedgerStore;
use mcp_bridge::{agy_command, agy_expected_version, daemon_base_url, daemon_status_url};
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


/// REQ-GROK-016: report grok's binary, version and AUTH KIND without spending a single token.
///
/// Auth kind matters and is not cosmetic. `XAI_API_KEY` bills a metered API account while a
/// cached login uses the operator's SuperGrok subscription. Reporting only "authenticated" would
/// hide which account a consult is actually charged against.
///
/// Deliberately never runs `-p`. A doctor that spends tokens is a doctor people stop running.
fn probe_grok() -> Vec<String> {
    let mut out = Vec::new();
    let (bin, _) = mcp_bridge::grok_command();
    let resolved = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    match resolved {
        None => {
            out.push(format!(
                "  grok: NOT FOUND ({bin}). Install from https://x.ai/cli or set TRIUMVIRATE_GROK_BIN"
            ));
            return out;
        }
        Some(path) => out.push(format!("  grok binary: {path}")),
    }

    match std::process::Command::new(&bin).arg("--no-auto-update").arg("--version").output() {
        Ok(o) if o.status.success() => {
            out.push(format!("  grok version: {}", String::from_utf8_lossy(&o.stdout).trim()));
        }
        _ => out.push("  grok version: WARN binary exists but --version failed".to_string()),
    }

    // Report WHICH credential is in use, never its value.
    let api_key = std::env::var("XAI_API_KEY").map(|v| !v.trim().is_empty()).unwrap_or(false);
    let cached_login = dirs::home_dir()
        .map(|h| h.join(".grok").join("auth.json").exists())
        .unwrap_or(false);
    out.push(match (api_key, cached_login) {
        (true, _) => "  grok auth: XAI_API_KEY set (METERED API billing, not the subscription)".to_string(),
        (false, true) => "  grok auth: cached login at ~/.grok/auth.json (subscription)".to_string(),
        (false, false) => "  grok auth: NONE. Run `grok login --oauth`, or set XAI_API_KEY".to_string(),
    });

    // Orphaned sessions. Grok has NO session GC of its own, and until now every one-shot consult
    // minted an id it then abandoned, so `~/.grok/sessions` grew per consult. Report the count
    // rather than deleting anything: these are conversation transcripts, and silently destroying
    // user data to tidy a directory is not a trade a doctor gets to make.
    if let Some(home) = dirs::home_dir() {
        let root = home.join(".grok").join("sessions");
        if root.is_dir() {
            let mut dirs = 0usize;
            let mut locks = 0usize;
            if let Ok(cwds) = std::fs::read_dir(&root) {
                for cwd in cwds.flatten().filter(|e| e.path().is_dir()) {
                    if let Ok(entries) = std::fs::read_dir(cwd.path()) {
                        for e in entries.flatten() {
                            let p = e.path();
                            if p.is_dir() {
                                dirs += 1;
                            } else if p.extension().is_some_and(|x| x == "lock") {
                                locks += 1;
                            }
                        }
                    }
                }
            }
            out.push(format!(
                "  grok sessions on disk: {dirs} transcripts, {locks} lock files"
            ));
            if dirs > 25 {
                out.push(
                    "  grok sessions: HIGH. Grok has no session GC; review with `grok sessions list` \
                     and remove with `grok sessions delete <id>`"
                        .to_string(),
                );
            }
        }
    }

    let profile = mcp_bridge::grok::grok_sandbox_profile();
    out.push(match profile {
        Some(p) => format!("  grok sandbox: {p} (consults are write-contained)"),
        None => "  grok sandbox: OFF. Consults can write to the workspace".to_string(),
    });
    out
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

    write_line_stdout(&format!("Triumvirate daemon v{}", daemon_core::VERSION))?;
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

    // AGY backend readiness (REQ-059). Runs a real `agy -p "2+2"` probe — success
    // proves OAuth + capture both work non-interactively (covers the spec's OAuth and
    // PTY-probe checks). Costs one quota call.
    let (agy_bin, _) = agy_command();
    let expected_version = agy_expected_version();
    write_line_stdout("AGY backend:")?;
    write_line_stdout(&format!("  agy_bin: {agy_bin}"))?;
    match crate::agy::agy_installed_version() {
        Ok(installed) => {
            let matches = installed == expected_version;
            write_line_stdout(&format!(
                "  version: {} (installed={installed}, expected={expected_version})",
                if matches { "PASS" } else { "WARN" }
            ))?;
            match crate::agy::doctor_probe().await {
                Ok(resp) if resp.contains('4') => {
                    write_line_stdout("  probe (agy -p \"2+2\"): PASS (oauth + capture ok)")?;
                }
                Ok(resp) => {
                    let snippet = resp.replace('\n', " ");
                    let snippet: String = snippet.chars().take(60).collect();
                    write_line_stdout(&format!("  probe (agy -p \"2+2\"): WARN (no '4' in: {snippet})"))?;
                }
                Err(e) => {
                    write_line_stdout(&format!("  probe (agy -p \"2+2\"): FAIL ({e})"))?;
                }
            }
        }
        Err(e) => {
            write_line_stdout(&format!("  binary_runnable: FAIL ({e})"))?;
        }
    }

    write_line_stdout("grok:")?;
    for line in probe_grok() {
        write_line_stdout(&line)?;
    }
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
                .unwrap_or_else(|| {
                    // REQ-GROK-003: one list, from mcp_bridge. A literal here is what let this
                    // fallback drift two agents behind the allowlist.
                    mcp_bridge::supported_agent_names()
                        .iter()
                        .map(|s| s.to_string())
                        .collect()
                }),
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
            "supported_agents": mcp_bridge::supported_agent_names(),
            "pending_fallbacks": pending_fallbacks,
            "fallback_tickets": fallback_tickets,
            "daemon_bind_addr": daemon_bind_addr
        }
    })
}
