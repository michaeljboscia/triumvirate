mod agent;
mod config;
mod digest;
mod fabric;
mod memory;
mod routing;
mod steno;
mod web;

use std::sync::Arc;

use tracing::{error, info};
use uuid::Uuid;

use agent::{AgentConnector, ClaudeConnector, CodexConnector, GeminiConnector, HealthMonitor};
use digest::DigestEngine;
use fabric::MessageBus;
use memory::MemoryStore;
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

    // Step 3: Open SQLite WAL database
    let store = MemoryStore::open(&cfg.db_path)?;
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    store.start_session(&session_id.to_string(), &["claude", "gemini", "codex"], &cwd)?;
    info!(db = %cfg.db_path.display(), "memory store ready");

    // Steps 5-7: Spawn agent connectors
    let mut health_monitor = HealthMonitor::new(bus.clone());
    let mut agents_ready = 0u8;
    let mut agents_total = 0u8;

    if cfg.agents.claude_enabled {
        agents_total += 1;
        let mut claude = ClaudeConnector::new();
        health_monitor.register(claude.agent_id(), claude.health_watch());
        match claude.spawn(bus.clone()).await {
            Ok(()) if claude.health() == triumvirate_proto::HealthStatus::Ready => {
                agents_ready += 1;
                info!("claude: READY");
            }
            Ok(()) => info!("claude: NOT READY (CLI not found)"),
            Err(e) => error!("claude: FAILED to spawn: {e}"),
        }
    }

    if cfg.agents.gemini_enabled {
        agents_total += 1;
        let mut gemini = GeminiConnector::new();
        health_monitor.register(gemini.agent_id(), gemini.health_watch());
        match gemini.spawn(bus.clone()).await {
            Ok(()) if gemini.health() == triumvirate_proto::HealthStatus::Ready => {
                agents_ready += 1;
                info!("gemini: READY");
            }
            Ok(()) => info!("gemini: NOT READY (CLI not found)"),
            Err(e) => error!("gemini: FAILED to spawn: {e}"),
        }
    }

    if cfg.agents.codex_enabled {
        agents_total += 1;
        let mut codex = CodexConnector::new();
        health_monitor.register(codex.agent_id(), codex.health_watch());
        match codex.spawn(bus.clone()).await {
            Ok(()) if codex.health() == triumvirate_proto::HealthStatus::Ready => {
                agents_ready += 1;
                info!("codex: READY");
            }
            Ok(()) => info!("codex: NOT READY (CLI not found)"),
            Err(e) => error!("codex: FAILED to spawn: {e}"),
        }
    }

    // Start health monitor
    health_monitor.run();
    info!(ready = agents_ready, total = agents_total, "agent health monitor started");

    // Step 9: Start Stenographer
    let steno = Stenographer::new(bus.clone(), session_id, cfg.db_path.clone());
    steno.run();
    info!("stenographer started");

    // Step 9b: Start digest fan-out for idle peer agents
    let digest = DigestEngine::new(bus.clone());
    digest.run();
    info!("digest engine started");

    // Step 8: Start web dashboard (this blocks — it's the main event loop)
    info!(
        port = cfg.web_port,
        agents_ready,
        agents_total,
        "triumvirate-agentd ready — dashboard at http://127.0.0.1:{}",
        cfg.web_port
    );

    web::start_web_server(bus, cfg.web_port).await?;

    Ok(())
}
