// Pre-existing lint debt acknowledged in PR #29. Each allow has a tracking
// issue for follow-up cleanup; remove the allow once the underlying lint is fixed.
#![allow(clippy::type_complexity, clippy::collapsible_if)]

use rmcp::{
    model::{LoggingLevel, LoggingMessageNotificationParam, ProgressNotificationParam, ProgressToken},
    service::{RequestContext, RoleServer},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    OnceLock,
};
use tokio::time::Duration;

pub mod abe;
pub mod aliases;
pub mod fleet;
pub mod gemini_query;
pub mod inter_agent;
pub mod knowledge;
pub mod review;
pub mod token_tools;

#[derive(Clone, Debug)]
pub struct ProgressEmitter {
    peer: rmcp::service::Peer<RoleServer>,
    progress_token: Option<ProgressToken>,
    progress_counter: Arc<AtomicU64>,
}

impl ProgressEmitter {
    pub fn from_context(context: &RequestContext<RoleServer>) -> Self {
        Self {
            peer: context.peer.clone(),
            progress_token: context.meta.get_progress_token(),
            progress_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    pub async fn emit(&self, message: impl Into<String>) {
        if !should_emit_progress_notifications() {
            return;
        }
        let message = message.into();
        if let Err(err) = self
            .peer
            .notify_logging_message(
                LoggingMessageNotificationParam::new(
                    LoggingLevel::Info,
                    serde_json::Value::String(message.clone()),
                )
                .with_logger("triumvirate"),
            )
            .await
        {
            tracing::debug!("progress logging notification failed: {err}");
        }

        if let Some(token) = self.progress_token.as_ref() {
            let progress = self
                .progress_counter
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1) as f64;
            let mut params = ProgressNotificationParam::new(token.clone(), progress);
            params.message = Some(message);
            if let Err(err) = self.peer.notify_progress(params).await {
                tracing::debug!("progress notification failed: {err}");
            }
        }
    }
}

fn should_emit_progress_notifications() -> bool {
    static SHOULD_EMIT: OnceLock<bool> = OnceLock::new();
    *SHOULD_EMIT.get_or_init(|| {
        if let Ok(raw) = std::env::var("TRIUMVIRATE_MCP_EMIT_PROGRESS") {
            let normalized = raw.trim().to_ascii_lowercase();
            return matches!(normalized.as_str(), "1" | "true" | "yes" | "on");
        }
        !matches!(
            std::env::args().nth(1).as_deref(),
            Some("mcp") | Some("proxy")
        )
    })
}

pub fn display_agent_name(agent: &str) -> String {
    match agent.to_lowercase().as_str() {
        "codex" => "Codex".to_string(),
        "gemini" => "Gemini".to_string(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => "Agent".to_string(),
            }
        }
    }
}

pub fn next_heartbeat_offset(current: Duration) -> Duration {
    if current == Duration::from_secs(10) {
        Duration::from_secs(40)
    } else {
        current.saturating_add(Duration::from_secs(60))
    }
}
