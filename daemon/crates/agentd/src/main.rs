mod agent;
mod config;
mod digest;
mod fabric;
mod fleet;
mod governance;
mod memory;
mod metrics;
mod routing;
mod quota;
mod shutdown;
mod steno;
mod web;

use std::sync::Arc;

use tracing::info;
use triumvirate_workflow::{WorkflowEngine, inspect_recovery};
use uuid::Uuid;

use agent::{
    SharedHealthRegistry, spawn_claude_supervisor, spawn_codex_supervisor, spawn_gemini_supervisor,
};
use digest::DigestEngine;
use fabric::MessageBus;
use memory::MemoryStore;
use metrics::SharedMetricsRegistry;
use quota::{QuotaTracker, SharedQuotaRegistry};
use steno::Stenographer;

/// triumvirate-agentd — multi-agent daemon
///
/// Startup sequence (from SPEC.md):
///   1. Load config from ~/.triumvirate/config.toml
///   2. Initialize message fabric (Tokio channels; NATS in v2)
///   3. Open SQLite WAL database
///   4. Initialize tracing/observability
///   5. Spawn Claude CLI subprocess → health check → mark ready
///   6. Spawn Gemini CLI subprocess → health check → mark ready
///   7. Spawn Codex CLI subprocess → health check → mark ready
///   8. Start web dashboard (GR1-D1: web-only UI)
///   9. Start Stenographer consumer
///  10. Ready. All 4 participants online.
///
/// If ANY step fails: display error in terminal, don't silently continue.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Step 4: Initialize tracing (moved up so all subsequent steps can log)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "triumvirate_agentd=info,tower_http=info".into()),
        )
        .with_target(false)
        .init();

    info!("triumvirate-agentd v{} starting", env!("CARGO_PKG_VERSION"));

    let session_id = Uuid::new_v4();
    info!(%session_id, "session initialized");

    // Step 1: Load config
    config::ensure_dirs()?;
    let cfg = config::load()?;
    info!(?cfg, "config loaded");

    // Step 2: Initialize message fabric
    let bus = Arc::new(MessageBus::new());
    info!("message fabric initialized (tokio broadcast channels)");

    // Step 2b: Initialize quota tracker
    let quota_registry = SharedQuotaRegistry::default();
    let metrics_registry = SharedMetricsRegistry::default();
    let quota_tracker = QuotaTracker::new(bus.clone(), quota_registry.clone(), metrics_registry.clone());
    quota_tracker.run();
    info!("quota tracker started");

    // Step 3: Open SQLite WAL database
    let store = MemoryStore::open(&cfg.db_path)?;
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    store.start_session(&session_id.to_string(), &["claude", "gemini", "codex"], &cwd)?;
    info!(db = %cfg.db_path.display(), "memory store ready");

    // Step 3b: Open workflow store (Temporal-inspired event-sourced state machine)
    let workflow_db_path = cfg
        .db_path
        .parent()
        .map(|p| p.join("workflow.db"))
        .unwrap_or_else(|| std::path::PathBuf::from("workflow.db"));
    let recovery = inspect_recovery(&workflow_db_path)?;
    if recovery.resumable_count() > 0 {
        info!(
            resumable = recovery.resumable_count(),
            "workflow recovery found resumable executions"
        );
    } else {
        info!("workflow recovery found no resumable executions");
    }
    let workflow_engine = WorkflowEngine::open(&workflow_db_path)?;
    let boot_workflow_id = workflow_engine.start_workflow(triumvirate_workflow::WorkflowType::Conversation)?;
    workflow_engine.advance_step(
        &boot_workflow_id,
        0,
        &serde_json::json!({ "event": "daemon_boot" }).to_string(),
    )?;
    workflow_engine.complete(&boot_workflow_id, 1)?;
    info!(db = %workflow_db_path.display(), workflow_id = %boot_workflow_id, "workflow engine ready");

    // Steps 5-7: Spawn agent connectors under supervisor loops
    let health_registry = SharedHealthRegistry::default();
    let mut agents_total = 0u8;

    if cfg.agents.claude_enabled {
        agents_total += 1;
        spawn_claude_supervisor(bus.clone(), health_registry.clone());
    }

    if cfg.agents.gemini_enabled {
        agents_total += 1;
        spawn_gemini_supervisor(bus.clone(), health_registry.clone());
    }

    if cfg.agents.codex_enabled {
        agents_total += 1;
        spawn_codex_supervisor(bus.clone(), health_registry.clone());
    }

    info!(total = agents_total, "agent supervisors started");

    // Step 9: Start Stenographer
    let steno = Stenographer::new(bus.clone(), session_id, cfg.db_path.clone(), std::env::current_dir()?);
    steno.run();
    info!("stenographer started");

    // Step 9b: Start digest fan-out for idle peer agents
    let digest = DigestEngine::new(bus.clone(), quota_registry.clone());
    digest.run();
    info!("digest engine started");

    // Step 8: Start web dashboard (this blocks — it's the main event loop)
    info!(
        port = cfg.web_port,
        agents_ready = 0,
        agents_total,
        "triumvirate-agentd ready — dashboard at http://127.0.0.1:{}",
        cfg.web_port
    );

    web::start_web_server(
        bus,
        health_registry,
        quota_registry,
        metrics_registry,
        cfg.db_path.clone(),
        workflow_db_path,
        cfg.web_port,
    )
    .await?;

    Ok(())
}
