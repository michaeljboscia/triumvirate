use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use anyhow::{Context, Result};
use daemon_core::observability::ObservabilityBus;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::{
    sync::mpsc,
    task,
    time::{self, Duration, MissedTickBehavior},
};
use tracing::warn;

use crate::{
    TokenDb, TokenRecord, insert_record, scan_claude_file, scan_codex_file, scan_gemini_chat_file,
    scan_gemini_telemetry_file,
};

const RECONCILIATION_INTERVAL: Duration = Duration::from_secs(10 * 60);

pub async fn run_scanner_loop(db: Arc<TokenDb>, bus: ObservabilityBus) {
    // Skip startup reconciliation if env var is set, or if this is the first boot
    // (first boot would scan ALL historical files including 500MB+ Gemini telemetry,
    // blocking the daemon for minutes). Instead, just start watching for new changes.
    // Historical backfill can be triggered manually via MCP tool later.
    if std::env::var("TRIUMVIRATE_SKIP_SCANNER_RECONCILIATION").is_ok() {
        tracing::info!("token scanner: skipping startup reconciliation (TRIUMVIRATE_SKIP_SCANNER_RECONCILIATION set)");
    } else {
        // Run reconciliation in a spawned task so it doesn't block the event loop
        let recon_db = db.clone();
        let recon_bus = bus.clone();
        tokio::spawn(async move {
            if let Err(err) = run_full_reconciliation(recon_db, &recon_bus).await {
                warn!("token scanner startup reconciliation failed: {err}");
            }
        });
    }

    let (file_tx, mut file_rx) = mpsc::unbounded_channel::<PathBuf>();
    let _watcher = match build_watcher(file_tx) {
        Ok(watcher) => Some(watcher),
        Err(err) => {
            warn!("token scanner file watcher unavailable, relying on periodic reconciliation: {err}");
            None
        }
    };

    let mut ticker = time::interval(RECONCILIATION_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(err) = run_full_reconciliation(db.clone(), &bus).await {
                    warn!("token scanner periodic reconciliation failed: {err}");
                }
            }
            maybe_path = file_rx.recv() => {
                let Some(path) = maybe_path else {
                    warn!("token scanner file event channel closed; switching to periodic reconciliation only");
                    continue;
                };
                // Incremental: emit per file (a single file change is not a burst).
                match scan_single_path(db.clone(), &bus, path).await {
                    Ok(t) if t.records > 0 => publish_token_scan(&bus, "incremental", t),
                    Ok(_) => {}
                    Err(err) => warn!("token scanner incremental scan failed: {err}"),
                }
            }
        }
    }
}

/// One token-economics scan cycle's aggregate, for the `token_scan` summary event.
#[derive(Default, Clone, Copy)]
struct ScanTotals {
    records: u64,
    tokens: u64,
    cost_usd: f64,
    duration_ms: u64,
}

async fn run_full_reconciliation(db: Arc<TokenDb>, bus: &ObservabilityBus) -> Result<()> {
    let started = Instant::now();
    let paths = collect_known_session_files()?;
    // Accumulate across ALL files and emit ONE token_scan for the whole run. Emitting per file
    // (a reconciliation can touch hundreds) would burst-overflow the shared broadcast channel
    // and get Lag-dropped on the receiver, silently losing most of the events (Antigravity).
    let mut totals = ScanTotals::default();
    for path in paths {
        match scan_single_path(db.clone(), bus, path).await {
            Ok(t) => {
                totals.records = totals.records.saturating_add(t.records);
                totals.tokens = totals.tokens.saturating_add(t.tokens);
                totals.cost_usd += t.cost_usd;
            }
            Err(err) => warn!("token scanner reconciliation scan failed: {err}"),
        }
    }
    totals.duration_ms = started.elapsed().as_millis() as u64;
    if totals.records > 0 {
        publish_token_scan(bus, "reconciliation", totals);
    }
    Ok(())
}

/// Publish the per-cycle `token_scan` summary onto the bus. This crate cannot call
/// mcp_bridge::posthog (mcp-bridge depends on token-economics, so the reverse would cycle);
/// main.rs subscribes to the bus and forwards it to PostHog.
fn publish_token_scan(bus: &ObservabilityBus, source: &'static str, t: ScanTotals) {
    bus.publish_event(
        "token_scan",
        serde_json::json!({
            "source": source,
            "records": t.records,
            "tokens": t.tokens,
            "cost_usd": t.cost_usd,
            "scan_duration_ms": t.duration_ms,
        }),
    );
}

/// Scan one file, store its records, emit per-record `token_update`s (dashboard tail), and
/// RETURN the aggregate. It no longer emits the `token_scan` summary itself — the caller does,
/// so the incremental path emits per file while reconciliation emits once for the whole run.
async fn scan_single_path(db: Arc<TokenDb>, bus: &ObservabilityBus, path: PathBuf) -> Result<ScanTotals> {
    if !path.is_file() {
        return Ok(ScanTotals::default());
    }

    let started = Instant::now();
    let scan_path = path.clone();
    let scanned_records = task::spawn_blocking(move || scan_and_store_records(db.as_ref(), &scan_path))
        .await
        .context("scanner worker task join failure")??;

    if scanned_records.is_empty() {
        return Ok(ScanTotals::default());
    }

    let scan_duration_ms = started.elapsed().as_millis() as u64;
    let totals = ScanTotals {
        records: scanned_records.len() as u64,
        tokens: scanned_records
            .iter()
            .fold(0u64, |acc, r| acc.saturating_add(r.total_tokens.max(0) as u64)),
        cost_usd: scanned_records.iter().filter_map(|r| r.cost_usd).sum(),
        duration_ms: scan_duration_ms,
    };

    for record in scanned_records {
        bus.publish_event(
            "token_update",
            serde_json::json!({
                "agent": record.agent,
                "session_id": record.session_id,
                "tokens_added": record.total_tokens,
                "total_cost_usd": record.cost_usd.unwrap_or(0.0),
                "scan_duration_ms": scan_duration_ms,
            }),
        );
    }

    Ok(totals)
}

fn scan_and_store_records(db: &TokenDb, path: &Path) -> Result<Vec<TokenRecord>> {
    let records = scan_records_for_path(db, path)?;
    if records.is_empty() {
        return Ok(Vec::new());
    }

    for record in &records {
        insert_record(db, record)?;
    }

    Ok(records)
}

fn scan_records_for_path(db: &TokenDb, path: &Path) -> Result<Vec<TokenRecord>> {
    if is_claude_session_file(path) {
        return scan_claude_file(db, path)
            .with_context(|| format!("failed scanning Claude session file {}", path.display()));
    }
    if is_codex_session_file(path) {
        return scan_codex_file(db, path)
            .with_context(|| format!("failed scanning Codex session file {}", path.display()));
    }
    if is_gemini_telemetry_file(path) {
        return scan_gemini_telemetry_file(db, path)
            .with_context(|| format!("failed scanning Gemini telemetry file {}", path.display()));
    }
    if is_gemini_chat_file(path) {
        return scan_gemini_chat_file(db, path)
            .with_context(|| format!("failed scanning Gemini chat file {}", path.display()));
    }

    Ok(Vec::new())
}

fn build_watcher(file_tx: mpsc::UnboundedSender<PathBuf>) -> Result<RecommendedWatcher> {
    let mut watcher = RecommendedWatcher::new(
        move |event: notify::Result<Event>| {
            let Ok(event) = event else {
                return;
            };
            if !is_relevant_event_kind(&event.kind) {
                return;
            }
            for path in event.paths {
                let _ = file_tx.send(path);
            }
        },
        Config::default(),
    )?;

    for root in watch_roots() {
        if !root.exists() {
            continue;
        }
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .with_context(|| format!("failed to watch {}", root.display()))?;
    }

    Ok(watcher)
}

fn is_relevant_event_kind(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Create(_) | EventKind::Modify(_))
}

fn collect_known_session_files() -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for root in watch_roots() {
        if !root.exists() {
            continue;
        }
        collect_files_recursive(&root, &mut out)
            .with_context(|| format!("failed walking {}", root.display()))?;
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn collect_files_recursive(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) => {
                warn!("token scanner could not read {}: {err}", dir.display());
                continue;
            }
        };

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            if is_claude_session_file(&path)
                || is_codex_session_file(&path)
                || is_gemini_chat_file(&path)
                || is_gemini_telemetry_file(&path)
            {
                out.push(path);
            }
        }
    }
    Ok(())
}

fn watch_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };

    vec![
        home.join(".claude").join("projects"),
        home.join(".codex").join("sessions"),
        home.join(".gemini"),
    ]
}

fn is_claude_session_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "jsonl") && path_contains(path, "/.claude/projects/")
}

fn is_codex_session_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "jsonl") && path_contains(path, "/.codex/sessions/")
}

fn is_gemini_chat_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "json")
        && path_contains(path, "/.gemini/")
        && path_contains(path, "/chats/")
}

fn is_gemini_telemetry_file(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "telemetry.jsonl") && path_contains(path, "/.gemini/")
}

fn path_contains(path: &Path, needle: &str) -> bool {
    path.to_string_lossy().replace('\\', "/").contains(needle)
}
