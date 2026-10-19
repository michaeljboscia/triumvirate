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
    // The launcher scripts/install.sh puts beside the binary; it loads the env from ~/.claude.json.
    let exe_path = std::env::current_exe()?;
    let launcher = exe_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("binary {} has no parent directory", exe_path.display()))?
        .join("triumvirate-start-daemon");
    if !launcher.exists() {
        anyhow::bail!(
            "missing {}: install with scripts/install.sh, which puts the launcher beside the binary",
            launcher.display()
        );
    }
    let plist = core_render_launch_agent_plist(&launcher.display().to_string(), &home.display().to_string());
    fs::write(&plist_path, plist)?;

    write_line_stdout(&format!("Installed launchd plist at {}", plist_path.display()))?;
    write_line_stdout(&format!(
        "Load with: launchctl bootstrap gui/$(id -u) {}  (or scripts/install-launch-agent.sh)",
        plist_path.display()
    ))?;
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
/// The five `[compat.claude]` keys that must all be `false`.
///
/// FIND-GROK-05. With inheritance ON, a grok consult loads `~/.claude.json` and advertises
/// roughly 420 tools plus 509 commands, measured at 66,559 input tokens to answer "pong". With
/// the curated set it is about 126 tools. Nothing detects drift: a grok upgrade that ignores
/// the section, or a user deleting five lines, silently reinstates the tax and every Fast
/// consult pays it.
const COMPAT_CLAUDE_KEYS: &[&str] = &["mcps", "skills", "hooks", "agents", "rules"];

/// Refuse rather than warn when compat drift is found.
pub fn grok_strict_compat() -> bool {
    std::env::var("TRIUMVIRATE_GROK_STRICT_COMPAT")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

/// Read `~/.grok/config.toml` and report any `[compat.claude]` key that is missing or true.
///
/// Deliberately a line scanner rather than a TOML parse: this crate has no toml dependency,
/// the check is a tripwire and not a config system, and a parse failure on an unrelated part
/// of the file must not hide the answer. Returns the offending keys.
fn grok_compat_drift(config: &str) -> Vec<String> {
    let mut in_section = false;
    let mut seen: Vec<(String, bool)> = Vec::new();
    for line in config.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_section = t.starts_with("[compat.claude]");
            continue;
        }
        if !in_section || t.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            let k = k.trim().to_string();
            let v = v.trim().trim_matches('"').to_ascii_lowercase();
            if COMPAT_CLAUDE_KEYS.contains(&k.as_str()) {
                seen.push((k, v == "true"));
            }
        }
    }
    let mut bad = Vec::new();
    for key in COMPAT_CLAUDE_KEYS {
        match seen.iter().find(|(k, _)| k == key) {
            Some((_, true)) => bad.push(format!("{key}=true")),
            None => bad.push(format!("{key} missing")),
            Some((_, false)) => {}
        }
    }
    bad
}

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
    // FIND-GROK-05: report `[compat.claude]` drift BEFORE the first slow consult.
    if let Some(home) = dirs::home_dir() {
        let cfg_path = home.join(".grok").join("config.toml");
        match std::fs::read_to_string(&cfg_path) {
            Ok(cfg) => {
                let bad = grok_compat_drift(&cfg);
                if bad.is_empty() {
                    out.push("  grok compat.claude: closed (all five keys false)".to_string());
                } else {
                    out.push(format!(
                        "  grok compat.claude: OPEN ({}). Consults will inherit the \
                         ~/.claude.json tool surface: roughly 420 tools instead of 126, \
                         measured at 66,559 input tokens to answer \"pong\". Set mcps, skills, \
                         hooks, agents and rules to false in {}.",
                        bad.join(", "),
                        cfg_path.display()
                    ));
                }
            }
            Err(_) => out.push(format!(
                "  grok compat.claude: UNKNOWN (no {}). If grok inherits ~/.claude.json, every \
                 consult pays the full tool-surface tax.",
                cfg_path.display()
            )),
        }
    }

    if let Some(home) = dirs::home_dir() {
        let root = home.join(".grok").join("sessions");
        if root.is_dir() {
            let mut dirs = 0usize;
            let mut locks = 0usize;
            // Locks live NESTED inside each session directory, not beside them. Counting only
            // the cwd level reported 0 while 181 sat one level down. Grok measured the real
            // layout; this now walks both levels.
            if let Ok(cwds) = std::fs::read_dir(&root) {
                for cwd in cwds.flatten().filter(|e| e.path().is_dir()) {
                    if let Ok(entries) = std::fs::read_dir(cwd.path()) {
                        for e in entries.flatten() {
                            let p = e.path();
                            if p.extension().is_some_and(|x| x == "lock") {
                                locks += 1;
                            } else if p.is_dir() {
                                dirs += 1;
                                if let Ok(inner) = std::fs::read_dir(&p) {
                                    locks += inner
                                        .flatten()
                                        .filter(|f| {
                                            f.path().extension().is_some_and(|x| x == "lock")
                                        })
                                        .count();
                                }
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

pub(crate) async fn run_doctor(live: bool) -> anyhow::Result<()> {
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
    let grok_lines = probe_grok();
    let compat_open = grok_lines.iter().any(|l| l.contains("compat.claude: OPEN"));
    for line in grok_lines {
        write_line_stdout(&line)?;
    }

    // FIND-GROK-05 required a NON-ZERO exit on drift, not just a printed line, and a fatal
    // mode. Grok reviewed the first attempt and pointed out that doctor still returned Ok(())
    // no matter what, so any script gating on `triumvirate doctor` could not see the drift, and
    // I had marked the finding CLOSED anyway.
    //
    // Warn is non-fatal by default because an open compat surface is expensive, not broken.
    // TRIUMVIRATE_GROK_STRICT_COMPAT=1 makes it refuse, for a host that must not pay the tax.
    if compat_open {
        if grok_strict_compat() {
            anyhow::bail!(
                "grok compat.claude is OPEN and TRIUMVIRATE_GROK_STRICT_COMPAT=1. Consults \
                 would inherit the ~/.claude.json tool surface. Set mcps, skills, hooks, \
                 agents and rules to false in ~/.grok/config.toml. Expected config: \
                 daemon/docs/grok-compat-claude.example.toml"
            );
        }
        write_line_stdout(
            "  grok compat.claude: WARN. Expected config committed at \
             daemon/docs/grok-compat-claude.example.toml. \
             Set TRIUMVIRATE_GROK_STRICT_COMPAT=1 to make this fatal.",
        )?;
    }
    // Seven Primes T-203 (REQ-054): the new seats' static checks, and with `--live` the live probe.
    print_seat_doctor(&SeatDoctorCtx::from_process()?, live)?;
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
            // Seven Primes REQ-080: the daemon's seats map as /status reported it (null from an older daemon).
            "seats": snapshot.seats,
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

// ---- Seven Primes seat doctor (T-203: REQ-054, REQ-091) ----------------------------------------------------------

/// (seat, CLI program, pinned version) for every seat verified by `doctor --live`. Pinned from TECH_STACK.md: a
/// different installed version FAILS the version check (re-capture fixtures and record it in progress.txt first).
/// `seat_clis_cover_exactly_the_doctor_verified_rows` proves this covers exactly the SeatProfile rows that need it.
pub(crate) const SEAT_CLIS: &[(&str, &str, &str)] = &[
    ("kimi", "kimi", "2.2.0"),
    ("mistral", "vibe", "2.26.1"),
    ("qwen", "qwen", "0.25.2"),
];

/// How the doctor finds a CLI on this machine (PATH in production, a fixture dir in tests).
pub(crate) type BinLookup = Box<dyn Fn(&str) -> Option<std::path::PathBuf>>;

/// Everything the seat checks read from the machine, injectable so each check has its own failing fixture.
pub(crate) struct SeatDoctorCtx {
    pub triumvirate_home: std::path::PathBuf,
    pub lookup: BinLookup,
    pub base_env: Vec<(String, String)>,
    pub sandbox_exec: std::path::PathBuf,
}

impl SeatDoctorCtx {
    /// The real machine: `$TRIUMVIRATE_HOME`, PATH lookup, this process's env, macOS seatbelt.
    pub(crate) fn from_process() -> anyhow::Result<Self> {
        Ok(Self {
            triumvirate_home: core_triumvirate_home_dir()?,
            lookup: Box::new(find_on_path),
            base_env: std::env::vars().collect(),
            sandbox_exec: std::path::PathBuf::from("/usr/bin/sandbox-exec"),
        })
    }
}

fn find_on_path(program: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(program))
            .find(|p| {
                p.is_file()
                    && p.metadata()
                        .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
            })
    })
}

/// One check's verdict, printed as `<seat>.<name>: PASS|FAIL (<detail>)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeatCheck {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

fn check(name: &'static str, ok: bool, detail: impl Into<String>) -> SeatCheck {
    SeatCheck {
        name,
        ok,
        detail: detail.into(),
    }
}

/// `<bin> --version`, first `x.y.z` in its output. None when it does not run, exits nonzero, or prints no version.
pub(crate) fn installed_version(bin: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new(bin)
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    regex_lite::Regex::new(r"\b(\d+\.\d+\.\d+)\b")
        .ok()?
        .captures(&text)
        .map(|c| c[1].to_string())
}

/// REQ-033: no inherited vendor credential or endpoint variable reaches the child. `seat_set` names what the seat
/// itself puts in the env (its env_add keys, its home variable, its own plan key): the scrub list has prefix patterns
/// (`KIMI_*`) that also match those, and they are the seat's own settings, not inherited credentials.
pub(crate) fn env_scrub_check(env: &[(String, String)], seat_set: &[&str]) -> SeatCheck {
    let stray: Vec<String> = mcp_bridge::containment::scan_stray_keys(env)
        .unwrap_or_default()
        .into_iter()
        .filter(|k| !seat_set.contains(&k.as_str()))
        .collect();
    check(
        "env_scrub",
        stray.is_empty(),
        if stray.is_empty() {
            "no inherited vendor credential".to_string()
        } else {
            format!("not scrubbed: {}", stray.join(", "))
        },
    )
}

/// Vibe #1239: the Mistral child env must select the Python CLI (research/02).
pub(crate) fn vibe_cli_check(env: &[(String, String)]) -> SeatCheck {
    let value = env
        .iter()
        .find(|(k, _)| k == "VIBE_CLI")
        .map(|(_, v)| v.as_str());
    check(
        "vibe_cli_python",
        value == Some("python"),
        format!("VIBE_CLI={}", value.unwrap_or("<unset>")),
    )
}

/// Findings under the seat home: files holding a key-shaped string that are not the seat's own expected credential
/// file, plus every entry the scan could not read (a scanner that skips what it cannot read fails open).
/// Symlinks are never followed (`DirEntry::file_type` does not traverse them), so the walk cannot leave the canonical
/// seat home or loop; files are streamed line by line, so no size makes a file unscannable. A missing seat home has
/// nothing to scan (the `seat_home` check reports it). Sockets and FIFOs hold no data at rest and opening a FIFO would
/// block, so only directories and regular files are walked.
fn stray_key_files(seat_home: &std::path::Path, expected: &[&str]) -> Vec<String> {
    use std::io::BufRead;
    let Ok(patterns) = regex_lite::Regex::new(
        r"(?m)(sk-[A-Za-z0-9_-]{16,}|^\s*[A-Z0-9_]*(API_KEY|TOKEN)\s*=\s*\S+)",
    ) else {
        return vec!["key regex failed to compile".to_string()];
    };
    let root = match fs::canonicalize(seat_home) {
        Ok(root) => root,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => return vec![format!("{} (unreadable: {e})", seat_home.display())],
    };
    let rel = |path: &std::path::Path| {
        path.strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    };
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                found.push(format!("{} (unreadable: {e})", rel(&dir)));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    found.push(format!("{} (unreadable entry: {e})", rel(&dir)));
                    continue;
                }
            };
            let path = entry.path();
            let name = rel(&path);
            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(e) => {
                    found.push(format!("{name} (unreadable: {e})"));
                    continue;
                }
            };
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() || expected.contains(&name.as_str()) {
                continue;
            }
            let file = match fs::File::open(&path) {
                Ok(file) => file,
                Err(e) => {
                    found.push(format!("{name} (unreadable: {e})"));
                    continue;
                }
            };
            for line in std::io::BufReader::new(file).split(b'\n') {
                match line {
                    Ok(line) if patterns.is_match(&String::from_utf8_lossy(&line)) => {
                        found.push(name.clone());
                        break;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        found.push(format!("{name} (unreadable: {e})"));
                        break;
                    }
                }
            }
        }
    }
    found.sort();
    found
}

/// The static checks for one new seat (REQ-054). None of them spawns the vendor CLI except `--version`.
pub(crate) fn seat_static_checks(seat: &str, ctx: &SeatDoctorCtx) -> Vec<SeatCheck> {
    let mut checks = Vec::new();
    let Some(&(_, program, pinned)) = SEAT_CLIS.iter().find(|(s, _, _)| *s == seat) else {
        return vec![check(
            "seat",
            false,
            format!("{seat} is not a doctor-verified seat"),
        )];
    };
    let Some(row) = mcp_bridge::seat_profile::seat_profile(seat) else {
        return vec![check(
            "seat",
            false,
            format!("{seat} has no SeatProfile row"),
        )];
    };

    let bin = (ctx.lookup)(program);
    checks.push(match &bin {
        Some(path) => check("binary", true, path.display().to_string()),
        None => check("binary", false, format!("`{program}` not on PATH")),
    });
    let version = bin.as_deref().and_then(installed_version);
    checks.push(match version.as_deref() {
        Some(v) if v == pinned => check("version", true, v),
        Some(v) => check(
            "version",
            false,
            format!(
                "installed {v}, pinned {pinned}: re-capture fixtures before trusting this seat"
            ),
        ),
        None => check(
            "version",
            false,
            format!("no version from `{program} --version`"),
        ),
    });

    let home = mcp_bridge::containment::seat_home(&ctx.triumvirate_home, seat).unwrap_or_default();
    checks.push(check(
        "seat_home",
        home.is_dir(),
        home.display().to_string(),
    ));

    // The env the runner will give the child: the builder's env_add on top of the scrubbed inherited env.
    let req = mcp_bridge::containment::SeatRequest {
        prompt: "doctor",
        cwd: &home,
        seat_home: &home,
        output_dir: &home,
        model: None,
        resume_session_id: None,
        extra_args: &[],
        max_turns: 1,
        time_limit: std::time::Duration::from_secs(60),
    };
    let built = match seat {
        "kimi" => mcp_bridge::kimi::build_kimi_invocation(&req),
        "mistral" => mcp_bridge::mistral::build_mistral_invocation(&req),
        _ => mcp_bridge::qwen::build_qwen_invocation(&req),
    };
    let (own_key, base_url) = if seat == "qwen" {
        mcp_bridge::qwen::plan_auth(None).map_or((None, None), |(k, u)| (Some(k), Some(u)))
    } else {
        (None, None)
    };
    let env = built.and_then(|inv| {
        mcp_bridge::containment::child_env(&ctx.base_env, row, &home, &inv.env_add)
            .map(|env| (env, inv.env_add))
    });
    match env {
        Ok((env, env_add)) => {
            let mut seat_set: Vec<&str> = env_add.iter().map(|(k, _)| k.as_str()).collect();
            seat_set.extend(row.home_env_var);
            seat_set.extend(own_key);
            checks.push(env_scrub_check(&env, &seat_set));
            if seat == "mistral" {
                checks.push(vibe_cli_check(&env));
            }
        }
        Err(e) => checks.push(check("env_scrub", false, e)),
    }

    // The seatbelt must START: a profile seatbelt cannot load is no containment at all.
    let started =
        mcp_bridge::containment::render_seatbelt_profile(&home, &home, &std::env::temp_dir())
            .is_ok_and(|profile| {
                std::process::Command::new(&ctx.sandbox_exec)
                    .args(["-p", &profile, "/usr/bin/true"])
                    .status()
                    .is_ok_and(|s| s.success())
            });
    checks.push(check(
        "seatbelt",
        started,
        ctx.sandbox_exec.display().to_string(),
    ));

    let expected_key_file: &[&str] = match seat {
        "qwen" => &["credentials.env"],
        "mistral" => &[".env"],
        _ => &[],
    };
    if seat == "qwen" {
        let creds = home.join("credentials.env");
        checks.push(
            match mcp_bridge::containment::check_credentials_file(&creds) {
                Ok(()) => check("credentials_mode", true, "0600"),
                Err(e) => check("credentials_mode", false, e),
            },
        );
        let configured_url = fs::read(home.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| {
                // The daemon's entry is the one whose id is the pinned model, not necessarily the first (T-104
                // merges into settings.json written at login).
                v["modelProviders"]["openai"]
                    .as_array()?
                    .iter()
                    .find(|e| e["id"].as_str() == Some(mcp_bridge::qwen::DEFAULT_QWEN_MODEL))?["baseUrl"]
                    .as_str()
                    .map(str::to_string)
            });
        checks.push(check(
            "base_url",
            configured_url.as_deref() == base_url && base_url.is_some(),
            format!(
                "settings.json baseUrl {}",
                configured_url.as_deref().unwrap_or("<missing>")
            ),
        ));
    }
    let stray = stray_key_files(&home, expected_key_file);
    checks.push(check(
        "stray_keys",
        stray.is_empty(),
        if stray.is_empty() {
            "no extra key-shaped strings".to_string()
        } else {
            format!(
                "key-shaped string or unscannable entry: {}",
                stray.join(", ")
            )
        },
    ));
    checks
}

/// `doctor --live` for a new seat. The live probe needs T-205's dispatch runner; until then it reports unverified and
/// writes nothing, so a seat can never become available before it can actually be dispatched.
pub(crate) fn seat_live_probe(seat: &str) -> Result<(), String> {
    Err(format!(
        "seat_unverified: {seat}: live probe runner not wired (T-205)"
    ))
}

/// Print every new seat's checks (and the live probe with `--live`).
pub(crate) fn print_seat_doctor(ctx: &SeatDoctorCtx, live: bool) -> anyhow::Result<()> {
    write_line_stdout("Seats:")?;
    for (seat, _, _) in SEAT_CLIS {
        for c in seat_static_checks(seat, ctx) {
            write_line_stdout(&format!(
                "  {seat}.{}: {} ({})",
                c.name,
                if c.ok { "PASS" } else { "FAIL" },
                c.detail
            ))?;
        }
        if live {
            match seat_live_probe(seat) {
                Ok(()) => write_line_stdout(&format!("  {seat}.live: PASS"))?,
                Err(e) => write_line_stdout(&format!("  {seat}.live: FAIL ({e})"))?,
            }
        }
    }
    Ok(())
}

/// Register each installed new-seat CLI's version with the gate, so a verified.json written for that version counts.
/// Called at daemon start; a seat whose CLI is absent stays unregistered and therefore unverified (fails closed).
pub(crate) fn register_installed_seat_versions() {
    for (seat, program, _) in SEAT_CLIS {
        if let Some(version) = find_on_path(program).as_deref().and_then(installed_version) {
            mcp_bridge::seat_gate::register_installed_cli_version(seat, &version);
        }
    }
}

/// `triumvirate seat clear <seat>`: the only way a suspension is lifted (REQ-091).
pub(crate) fn run_seat_clear(seat: &str) -> anyhow::Result<()> {
    run_seat_clear_in(&core_triumvirate_home_dir()?, seat)
}

/// `run_seat_clear` against an explicit Triumvirate home, so a test drives the command path without touching the
/// process-global home (the `seat_gate_in` pattern).
pub(crate) fn run_seat_clear_in(home: &std::path::Path, seat: &str) -> anyhow::Result<()> {
    mcp_bridge::seat_state::clear(home, seat).map_err(anyhow::Error::msg)?;
    write_line_stdout(&format!(
        "{seat}: suspension cleared (suspended.json removed if present)"
    ))
}

#[cfg(test)]
mod grok_compat_tests {
    use super::*;

    /// FIND-GROK-05: doctor must catch inheritance being ON before the first slow consult.
    ///
    /// With inheritance on, a grok consult advertises roughly 420 tools instead of 126 and was
    /// measured at 66,559 input tokens to answer "pong". Nothing detected drift: a grok upgrade
    /// that ignores the section, or a user deleting five lines, silently reinstates the tax.
    ///
    /// RED IF: a `true` key stops being reported.
    #[test]
    fn doctor_flags_an_open_compat_key() {
        let cfg = r#"
[compat.claude]
mcps = true
skills = false
hooks = false
agents = false
rules = false
"#;
        let bad = grok_compat_drift(cfg);
        assert_eq!(bad, vec!["mcps=true"], "an open key must be named, not just counted");
    }

    /// A MISSING key is drift too: the default is inherit-on, so silence is not safety.
    /// RED IF: absent keys stop being reported, which is how a partial config passes.
    #[test]
    fn doctor_flags_a_missing_compat_key() {
        let cfg = "[compat.claude]\nmcps = false\n";
        let bad = grok_compat_drift(cfg);
        assert!(bad.contains(&"skills missing".to_string()), "got: {bad:?}");
        assert_eq!(bad.len(), 4, "four of five keys are absent; got: {bad:?}");
    }

    /// The correct config reports clean.
    /// RED IF: a correct config starts warning, which trains people to ignore the warning.
    #[test]
    fn doctor_is_quiet_when_all_five_keys_are_closed() {
        let cfg = r#"
[compat.claude]
mcps = false
skills = false
hooks = false
agents = false
rules = false
"#;
        assert!(grok_compat_drift(cfg).is_empty());
    }

    /// Keys in OTHER sections must not satisfy the check.
    /// RED IF: the section scoping is dropped, so an unrelated `mcps = false` elsewhere counts.
    #[test]
    fn keys_outside_the_compat_section_do_not_count() {
        let cfg = r#"
[some.other]
mcps = false
skills = false
hooks = false
agents = false
rules = false

[compat.claude]
mcps = true
"#;
        let bad = grok_compat_drift(cfg);
        assert!(bad.contains(&"mcps=true".to_string()));
        assert!(
            bad.contains(&"skills missing".to_string()),
            "a key in another section must not satisfy compat.claude; got: {bad:?}"
        );
    }
}

/// Seven Primes T-203 (REQ-054, REQ-091, REQ-080): the seat doctor, one fixture per failing check.
#[cfg(test)]
mod seven_primes_doctor_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    /// A key-shaped test value, assembled at runtime so no secret-looking literal is ever committed (gitleaks scans
    /// every commit on the branch, and a fixture is not an exception).
    fn fake_secret(prefix: &str) -> String {
        [prefix, "0123456789", "abcdefghij"].concat()
    }

    /// A fake CLI that prints `version` for `--version`, written through the crate's ETXTBSY-safe helper.
    fn fake_bin(dir: &Path, program: &str, version: &str) -> PathBuf {
        let p = dir.join(program);
        crate::write_test_executable(&p, format!("#!/bin/sh\necho \"{program} {version}\"\n"))
            .unwrap();
        p
    }

    /// The real seatbelt on macOS; elsewhere `/usr/bin/true` stands in, because seatbelt only exists on macOS (the
    /// macos-latest CI job runs these with the real one).
    fn sandbox() -> PathBuf {
        if cfg!(target_os = "macos") {
            PathBuf::from("/usr/bin/sandbox-exec")
        } else {
            PathBuf::from("/usr/bin/true")
        }
    }

    /// A machine where every seat check passes: pinned CLIs, seat homes, Qwen credentials 0600 and plan baseUrl.
    struct Fixture {
        _dir: tempfile::TempDir,
        home: PathBuf,
        bins: PathBuf,
    }

    fn good() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("th");
        let bins = dir.path().join("bin");
        fs::create_dir_all(&bins).unwrap();
        for (seat, program, pinned) in SEAT_CLIS {
            fake_bin(&bins, program, pinned);
            fs::create_dir_all(home.join("homes").join(seat)).unwrap();
        }
        let qwen = home.join("homes/qwen");
        let row = mcp_bridge::seat_profile::seat_profile("qwen").unwrap();
        mcp_bridge::containment::write_seat_files(&qwen, row).unwrap();
        let creds = qwen.join("credentials.env");
        fs::write(
            &creds,
            format!("QWENCLOUD_TOKEN_PLAN_API_KEY={}\n", fake_secret("sk-sp-")),
        )
        .unwrap();
        fs::set_permissions(&creds, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(
            home.join("homes/mistral/.env"),
            format!("MISTRAL_API_KEY={}\n", fake_secret("")),
        )
        .unwrap();
        Fixture {
            _dir: dir,
            home,
            bins,
        }
    }

    fn ctx(f: &Fixture, base_env: Vec<(String, String)>, sandbox_exec: PathBuf) -> SeatDoctorCtx {
        let bins = f.bins.clone();
        SeatDoctorCtx {
            triumvirate_home: f.home.clone(),
            lookup: Box::new(move |p| Some(bins.join(p)).filter(|b| b.is_file())),
            base_env,
            sandbox_exec,
        }
    }

    fn failing(seat: &str, c: &SeatDoctorCtx) -> Vec<&'static str> {
        seat_static_checks(seat, c)
            .into_iter()
            .filter(|c| !c.ok)
            .map(|c| c.name)
            .collect()
    }

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// RED IF: a check stops passing on a correctly set up machine (a doctor that always fails is useless too).
    #[test]
    fn every_check_passes_when_everything_is_fixed() {
        let f = good();
        // Inherited metered keys are present in the daemon's env; the scrub must keep them from every seat.
        let c = ctx(
            &f,
            env(&[
                ("PATH", "/usr/bin"),
                ("MISTRAL_API_KEY", "metered"),
                ("OPENAI_API_KEY", "metered"),
            ]),
            sandbox(),
        );
        for (seat, _, _) in SEAT_CLIS {
            let checks = seat_static_checks(seat, &c);
            assert!(checks.iter().all(|c| c.ok), "{seat}: {checks:?}");
            assert!(checks.len() >= 6, "{seat}: every check ran: {checks:?}");
        }
    }

    /// Each fixture breaks ONE thing, and exactly that check fails.
    /// RED IF: any static check stops detecting its failure.
    #[test]
    fn each_static_check_fails_on_its_own_fixture() {
        // missing binary (and therefore no version)
        let f = good();
        fs::remove_file(f.bins.join("kimi")).unwrap();
        assert_eq!(
            failing("kimi", &ctx(&f, vec![], sandbox())),
            ["binary", "version"]
        );
        // wrong version string
        let f = good();
        fake_bin(&f.bins, "vibe", "2.27.0");
        assert_eq!(failing("mistral", &ctx(&f, vec![], sandbox())), ["version"]);
        // missing seat home
        let f = good();
        fs::remove_dir_all(f.home.join("homes/kimi")).unwrap();
        assert_eq!(failing("kimi", &ctx(&f, vec![], sandbox())), ["seat_home"]);
        // seatbelt not starting
        let f = good();
        assert_eq!(
            failing("kimi", &ctx(&f, vec![], PathBuf::from("/usr/bin/false"))),
            ["seatbelt"]
        );
        // credentials.env 0644
        let f = good();
        fs::set_permissions(
            f.home.join("homes/qwen/credentials.env"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert_eq!(
            failing("qwen", &ctx(&f, vec![], sandbox())),
            ["credentials_mode"]
        );
        // wrong baseUrl
        let f = good();
        let settings = f.home.join("homes/qwen/settings.json");
        let text = fs::read_to_string(&settings)
            .unwrap()
            .replace("token-plan.maas.qwencloudapi.com", "example.invalid");
        fs::write(&settings, text).unwrap();
        assert_eq!(failing("qwen", &ctx(&f, vec![], sandbox())), ["base_url"]);
        // stray key under the seat home
        let f = good();
        fs::write(
            f.home.join("homes/kimi/notes.txt"),
            format!("OPENAI_API_KEY={}\n", fake_secret("sk-proj-")),
        )
        .unwrap();
        assert_eq!(failing("kimi", &ctx(&f, vec![], sandbox())), ["stray_keys"]);
    }

    /// The env checks are pure over the final child env, because the composition (builder env_add + child_env scrub)
    /// cannot be made to produce a bad env without breaking a T-104 or T-102 test first; these prove the checks catch it.
    /// RED IF: MISTRAL_API_KEY in the child env passes, or a Mistral child without VIBE_CLI=python passes.
    #[test]
    fn env_checks_catch_an_unscrubbed_key_and_a_missing_vibe_cli() {
        let bad = env(&[("MISTRAL_API_KEY", "metered"), ("VIBE_CLI", "rust")]);
        let c = env_scrub_check(&bad, &[]);
        assert!(!c.ok && c.detail.contains("MISTRAL_API_KEY"), "{c:?}");
        assert!(!vibe_cli_check(&bad).ok);
        assert!(!vibe_cli_check(&[]).ok);
        assert!(vibe_cli_check(&env(&[("VIBE_CLI", "python")])).ok);
        // The seat's own plan key is allowed; anything else is not.
        let qwen = env(&[
            ("QWENCLOUD_TOKEN_PLAN_API_KEY", "sk-sp-x"),
            ("DASHSCOPE_API_KEY", "x"),
        ]);
        let c = env_scrub_check(&qwen, &["QWENCLOUD_TOKEN_PLAN_API_KEY"]);
        assert!(
            !c.ok && c.detail == "not scrubbed: DASHSCOPE_API_KEY",
            "{c:?}"
        );
    }

    /// The T-203 placeholder T-205 flips: until the runner is wired, a live probe cannot verify a seat.
    /// RED IF: `doctor --live` reports a new seat verified before T-205 wires the runner.
    #[test]
    fn live_probe_is_not_wired_until_t205() {
        for (seat, _, _) in SEAT_CLIS {
            let err = seat_live_probe(seat).unwrap_err();
            assert!(
                err.starts_with("seat_unverified:") && err.contains("runner not wired"),
                "{err}"
            );
        }
    }

    /// SEAT_CLIS is a second list, so it is made safe the census way: it covers exactly the doctor-verified rows.
    /// RED IF: a seat needing doctor verification has no CLI entry, or an entry has no such row.
    #[test]
    fn seat_clis_cover_exactly_the_doctor_verified_rows() {
        let mut rows: Vec<&str> = mcp_bridge::seat_profile::all_seats()
            .iter()
            .filter(|r| r.verify_policy == mcp_bridge::seat_profile::VerifyPolicy::DoctorLive)
            .map(|r| r.key)
            .collect();
        let mut clis: Vec<&str> = SEAT_CLIS.iter().map(|(s, _, _)| *s).collect();
        rows.sort_unstable();
        clis.sort_unstable();
        assert_eq!(rows, clis);
    }

    /// `seat clear` lifts a suspension through the same state file the gate reads, via the command's own code path.
    /// RED IF: `run_seat_clear` (through `run_seat_clear_in`) leaves suspended.json, or the version regex stops reading
    /// a CLI's version.
    #[test]
    fn seat_clear_removes_the_suspension_and_versions_parse() {
        let dir = tempfile::tempdir().unwrap();
        mcp_bridge::seat_state::write_suspended(dir.path(), "qwen", "403").unwrap();
        assert!(dir.path().join("seats/qwen/suspended.json").exists());
        run_seat_clear_in(dir.path(), "qwen").unwrap();
        assert!(!dir.path().join("seats/qwen/suspended.json").exists());
        let bin = fake_bin(dir.path(), "kimi", "2.2.0");
        assert_eq!(installed_version(&bin).as_deref(), Some("2.2.0"));
    }

    /// A CLI that prints the pinned version but exits nonzero is broken, not installed at that version.
    /// RED IF: `installed_version` reads a version from a `--version` run that failed.
    #[test]
    fn a_nonzero_version_exit_reports_no_version() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("kimi");
        crate::write_test_executable(&bin, "#!/bin/sh\necho \"kimi 2.2.0\"\nexit 3\n").unwrap();
        assert_eq!(installed_version(&bin), None);
    }

    /// The stray scan never follows a symlink: a link inside the seat home to a directory outside it (which holds a
    /// key-shaped file) must not be walked, and a link back to the home must not loop.
    /// RED IF: the scanner follows a directory symlink out of the seat home or into a cycle.
    #[test]
    fn stray_scan_does_not_follow_symlinks() {
        let f = good();
        let outside = f._dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(
            outside.join("leak.txt"),
            format!("OPENAI_API_KEY={}\n", fake_secret("sk-proj-")),
        )
        .unwrap();
        let kimi = f.home.join("homes/kimi");
        std::os::unix::fs::symlink(&outside, kimi.join("escape")).unwrap();
        std::os::unix::fs::symlink(&kimi, kimi.join("loop")).unwrap();
        assert_eq!(stray_key_files(&kimi, &[]), Vec::<String>::new());
        assert!(failing("kimi", &ctx(&f, vec![], sandbox())).is_empty());
    }

    /// An entry the scan cannot read is a finding, not a skip: a scanner that skips what it cannot read fails open.
    /// RED IF: an unreadable file or directory under the seat home lets `stray_keys` PASS.
    #[test]
    fn stray_scan_fails_on_an_unreadable_entry() {
        let f = good();
        let kimi = f.home.join("homes/kimi");
        let file = kimi.join("locked.txt");
        fs::write(&file, "nothing").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
        let sub = kimi.join("lockeddir");
        fs::create_dir_all(&sub).unwrap();
        fs::set_permissions(&sub, fs::Permissions::from_mode(0o000)).unwrap();
        let found = stray_key_files(&kimi, &[]);
        let failed = failing("kimi", &ctx(&f, vec![], sandbox()));
        // Restore so the tempdir can be removed (r-x is enough to list, and so remove, the empty directory).
        fs::set_permissions(&sub, fs::Permissions::from_mode(0o500)).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|e| e.contains("unreadable")), "{found:?}");
        assert!(
            found[0].starts_with("locked.txt") && found[1].starts_with("lockeddir"),
            "{found:?}"
        );
        assert_eq!(failed, ["stray_keys"]);
    }

    /// The Qwen baseUrl check reads the daemon's entry by model id: other openai entries written at login may come
    /// first (T-104 merges rather than replaces).
    /// RED IF: the check reads `openai[0]` instead of the entry whose id is the pinned model.
    #[test]
    fn qwen_base_url_is_read_from_the_daemon_entry_not_the_first() {
        let f = good();
        let settings = f.home.join("homes/qwen/settings.json");
        let mut doc: serde_json::Value =
            serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
        doc["modelProviders"]["openai"]
            .as_array_mut()
            .unwrap()
            .insert(
                0,
                serde_json::json!({"id": "login-model", "baseUrl": "https://example.invalid"}),
            );
        fs::write(&settings, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
        assert!(failing("qwen", &ctx(&f, vec![], sandbox())).is_empty());
    }

    /// `triumvirate status` shows the daemon's seats map.
    /// RED IF: the CLI status report drops `seats` from a reachable daemon's snapshot.
    #[test]
    fn status_report_includes_the_daemon_seats() {
        let seat = shared_types::SeatStatus {
            configured: false,
            verified: false,
            suspended: false,
            available: false,
            reason: Some("seat_not_configured: kimi".to_string()),
        };
        let report = build_status_report(
            "127.0.0.1:1".to_string(),
            Some(DaemonHealthResponse {
                status: "ok".to_string(),
                version: None,
                service: None,
                mode: None,
                daemon: None,
                auth: None,
                daemon_bind_addr: None,
            }),
            Some(DaemonStatusSnapshot {
                seats: Some([("kimi".to_string(), seat)].into_iter().collect()),
                daemon_mode: None,
                supported_agents: None,
                pending_fallbacks: None,
                fallback_tickets: None,
                daemon_bind_addr: None,
            }),
            0,
            vec![],
        );
        assert_eq!(
            report["snapshot"]["seats"]["kimi"]["reason"],
            "seat_not_configured: kimi"
        );
        assert_eq!(report["snapshot"]["seats"]["kimi"]["configured"], false);
    }
}
