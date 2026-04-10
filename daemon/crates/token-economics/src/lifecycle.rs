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
                if let Err(err) = scan_single_path(db.clone(), &bus, path).await {
                    warn!("token scanner incremental scan failed: {err}");
                }
            }
        }
    }
}

async fn run_full_reconciliation(db: Arc<TokenDb>, bus: &ObservabilityBus) -> Result<()> {
    let paths = collect_known_session_files()?;
    for path in paths {
        if let Err(err) = scan_single_path(db.clone(), bus, path).await {
            warn!("token scanner reconciliation scan failed: {err}");
        }
    }
    Ok(())
}

async fn scan_single_path(db: Arc<TokenDb>, bus: &ObservabilityBus, path: PathBuf) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }

    let started = Instant::now();
    let scan_path = path.clone();
    let scanned_records = task::spawn_blocking(move || scan_and_store_records(db.as_ref(), &scan_path))
        .await
        .context("scanner worker task join failure")??;

    if scanned_records.is_empty() {
        return Ok(());
    }

    let scan_duration_ms = started.elapsed().as_millis() as u64;
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

    Ok(())
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
