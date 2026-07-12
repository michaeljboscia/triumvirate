//! MCP bridge crate boundary.
//!
//! This crate will own tool routing and stdio-facing concerns as we continue
//! extracting logic from the monolithic `triumvirate` binary crate.

use shared_types::AskAgentRequest;
use agent_adapter::AgentVerbosity;
use tracing::instrument;

pub mod codex_capabilities;
pub use codex_capabilities::{
    CodexCapabilities, codex_capabilities, probe_and_cache_codex_capabilities,
};

pub mod agy;
pub mod agy_resilience;

// T-002 (REQ-DS-002/003/015): authoritative env-config loader for the DeepSeek sibling.
pub mod deepseek_config;

// T-004 (REQ-DS-006/010): semaphore, token bucket, three-state breaker, classify().
pub mod deepseek_resilience;

// T-005 (REQ-DS-007): reqwest::Client builder honouring rolling read_timeout.
pub mod deepseek;


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeInfo {
    pub name: &'static str,
}

impl Default for BridgeInfo {
    fn default() -> Self {
        Self {
            name: "triumvirate-mcp-bridge",
        }
    }
}

#[instrument(skip_all)]
pub fn is_supported_agent(req: &AskAgentRequest) -> bool {
    is_supported_agent_name(&req.agent)
}

/// Normalize a caller-supplied agent name to its canonical execution key.
///
/// The public Antigravity sibling is served by the internal execution key
/// `gemini` (the name is kept internally for now; the transport already runs the
/// `agy`/Antigravity CLI). Callers may say `antigravity` (product name) or `agy`
/// (CLI short form) — both normalize to the `gemini` key so a single dispatch
/// arm, worker-cache slot, and session record serve every alias. Everything else
/// passes through lowercased.
///
/// Apply this at EVERY boundary where a caller's agent string is first trusted —
/// before worker-acquire, before session storage, and before dispatch — or state
/// splits between the aliases (Codex/Gemini twin review, 2026-07-06).
#[instrument(skip_all)]
pub fn normalize_agent_name(agent: &str) -> String {
    match agent.to_lowercase().as_str() {
        "antigravity" | "agy" => "gemini".to_string(),
        other => other.to_string(),
    }
}

#[instrument(skip_all)]
pub fn is_supported_agent_name(agent: &str) -> bool {
    // Validate the CANONICAL name so aliases (antigravity/agy) are accepted via the
    // same allowlist the dispatch arms match on — no raw alias reaches dispatch.
    matches!(
        normalize_agent_name(agent).as_str(),
        "gemini" | "codex" | "deepseek" | "claude"
    )
}

#[instrument(skip_all)]
pub fn should_use_daemon_proxy(var_value: Option<&str>) -> bool {
    var_value
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

#[instrument(skip_all)]
pub fn use_daemon_for_mcp_from_env() -> bool {
    std::env::var("TRIUMVIRATE_MCP_USE_DAEMON")
        .ok()
        .map(|v| !matches!(v.to_lowercase().as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(true)
}

#[instrument(skip_all)]
pub fn daemon_autostart_enabled(var_value: Option<&str>) -> bool {
    var_value
        .map(|v| !matches!(v.to_lowercase().as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(true)
}

#[instrument(skip_all)]
pub fn caller_driver_identity() -> Option<String> {
    std::env::var("TRIUMVIRATE_DRIVER").ok()
}

#[instrument(skip_all)]
pub fn is_bearer_authorized(raw_auth_header: Option<&str>, token: &str) -> bool {
    let expected = format!("Bearer {token}");
    raw_auth_header.map(|v| v == expected).unwrap_or(false)
}

#[instrument(skip_all)]
pub fn daemon_base_url() -> String {
    if let Ok(base) = std::env::var("TRIUMVIRATE_DAEMON_BASE_URL") {
        return base;
    }
    if let Ok(bind_addr) = std::env::var("TRIUMVIRATE_DAEMON_BIND_ADDR") {
        let bind_addr = bind_addr.trim();
        if !bind_addr.is_empty() {
            return format!("http://{bind_addr}");
        }
    }
    "http://127.0.0.1:8080".to_string()
}

#[instrument(skip_all)]
pub fn daemon_status_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_URL")
        .unwrap_or_else(|_| format!("{}/status", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_health_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_HEALTH_URL")
        .unwrap_or_else(|_| format!("{}/health", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_ask_agent_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_ASK_AGENT_URL")
        .unwrap_or_else(|_| format!("{}/ask-agent", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_session_spawn_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SESSION_SPAWN_URL")
        .unwrap_or_else(|_| format!("{}/session/spawn", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_session_ask_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SESSION_ASK_URL")
        .unwrap_or_else(|_| format!("{}/session/ask", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_session_dismiss_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SESSION_DISMISS_URL")
        .unwrap_or_else(|_| format!("{}/session/dismiss", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_session_list_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SESSION_LIST_URL")
        .unwrap_or_else(|_| format!("{}/session/list", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_memory_write_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_MEMORY_WRITE_URL")
        .unwrap_or_else(|_| format!("{}/memory/write", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_memory_read_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_MEMORY_READ_URL")
        .unwrap_or_else(|_| format!("{}/memory/read", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_scratchpad_write_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SCRATCHPAD_WRITE_URL")
        .unwrap_or_else(|_| format!("{}/scratchpad/write", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_scratchpad_list_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SCRATCHPAD_LIST_URL")
        .unwrap_or_else(|_| format!("{}/scratchpad/list", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_outbox_recent_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_OUTBOX_RECENT_URL")
        .unwrap_or_else(|_| format!("{}/outbox/recent", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_fallback_list_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_FALLBACK_LIST_URL")
        .unwrap_or_else(|_| format!("{}/fallback/list", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_fallback_ack_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_FALLBACK_ACK_URL")
        .unwrap_or_else(|_| format!("{}/fallback/ack", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_fallback_gc_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_FALLBACK_GC_URL")
        .unwrap_or_else(|_| format!("{}/fallback/gc", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_ledger_query_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_LEDGER_QUERY_URL")
        .unwrap_or_else(|_| format!("{}/ledger/query", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_ledger_session_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_LEDGER_SESSION_URL")
        .unwrap_or_else(|_| format!("{}/ledger/session", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_ledger_record_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_LEDGER_RECORD_URL")
        .unwrap_or_else(|_| format!("{}/ledger/record", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_ledger_gc_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_LEDGER_GC_URL")
        .unwrap_or_else(|_| format!("{}/ledger/gc", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_lesson_add_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_LESSON_ADD_URL")
        .unwrap_or_else(|_| format!("{}/lesson/add", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_lesson_query_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_LESSON_QUERY_URL")
        .unwrap_or_else(|_| format!("{}/lesson/query", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_lesson_validate_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_LESSON_VALIDATE_URL")
        .unwrap_or_else(|_| format!("{}/lesson/validate", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn daemon_lesson_list_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_LESSON_LIST_URL")
        .unwrap_or_else(|_| format!("{}/lesson/list", daemon_base_url()))
}

#[instrument(skip_all)]
pub fn gemini_command() -> (String, Vec<String>) {
    resolve_connector_command("TRIUMVIRATE_GEMINI_BIN", "TRIUMVIRATE_GEMINI_ARGS", "gemini")
}

#[instrument(skip_all)]
pub fn codex_command() -> (String, Vec<String>) {
    resolve_connector_command("TRIUMVIRATE_CODEX_BIN", "TRIUMVIRATE_CODEX_ARGS", "codex")
}

#[instrument(skip_all)]
pub fn claude_command() -> (String, Vec<String>) {
    resolve_connector_command("TRIUMVIRATE_CLAUDE_BIN", "TRIUMVIRATE_CLAUDE_ARGS", "claude")
}

/// Backend that serves the public `gemini` agent. The public agent name stays
/// `gemini` (C3); this selects which CLI actually executes the request. REQ-001.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeminiBackend {
    /// Legacy `gemini` CLI (stream-json). Default and rollback target; kept until
    /// the binary stops serving on 2026-06-18.
    GeminiCli,
    /// Antigravity CLI (`agy`), single-turn plain text, subscription-OAuth only.
    Agy,
}

impl GeminiBackend {
    /// Stable label used in logs and the degraded-route surface (REQ-005).
    pub fn as_str(self) -> &'static str {
        match self {
            GeminiBackend::GeminiCli => "gemini-cli",
            GeminiBackend::Agy => "agy",
        }
    }
}

/// Select the gemini backend from `TRIUMVIRATE_GEMINI_BACKEND`. The value must be
/// exactly `agy` to select agy; unset or ANY other value is the legacy gemini-cli
/// path, so the default behavior is byte-for-byte the current path (REQ-001/002).
#[instrument(skip_all)]
pub fn gemini_backend() -> GeminiBackend {
    match std::env::var("TRIUMVIRATE_GEMINI_BACKEND").ok().as_deref() {
        Some("agy") => GeminiBackend::Agy,
        _ => GeminiBackend::GeminiCli,
    }
}

/// Resolve the agy binary path + extra args, mirroring the gemini/codex connector
/// resolution (`TRIUMVIRATE_AGY_BIN` default `agy`, `TRIUMVIRATE_AGY_ARGS`). REQ-010.
#[instrument(skip_all)]
pub fn agy_command() -> (String, Vec<String>) {
    resolve_connector_command("TRIUMVIRATE_AGY_BIN", "TRIUMVIRATE_AGY_ARGS", "agy")
}

/// Shadow-compare mode (Slice 6, opt-in via `TRIUMVIRATE_GEMINI_SHADOW`). When on,
/// every `gemini` request ALSO dispatches the other Gemini backend for comparison;
/// the primary still answers. Off by default — it doubles usage of the shared Google
/// quota pool, so it is a validation tool, not steady-state.
#[instrument(skip_all)]
pub fn gemini_shadow_enabled() -> bool {
    std::env::var("TRIUMVIRATE_GEMINI_SHADOW")
        .ok()
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "on" | "yes"))
        .unwrap_or(false)
}

impl GeminiBackend {
    /// The other Gemini backend — the one shadow-compare runs alongside this primary.
    pub fn shadow_counterpart(self) -> GeminiBackend {
        match self {
            GeminiBackend::GeminiCli => GeminiBackend::Agy,
            GeminiBackend::Agy => GeminiBackend::GeminiCli,
        }
    }
}

/// The verified-good agy version the backend expects (REQ-059). Defaults to the
/// last version verified against the live binary (1.0.2). On mismatch the backend
/// warns, or refuses under `agy_strict_version()`.
#[instrument(skip_all)]
pub fn agy_expected_version() -> String {
    std::env::var("TRIUMVIRATE_AGY_EXPECTED_VERSION").unwrap_or_else(|_| "1.0.2".to_string())
}

/// Whether an agy version mismatch refuses the backend (vs. warn only). REQ-059.
#[instrument(skip_all)]
pub fn agy_strict_version() -> bool {
    std::env::var("TRIUMVIRATE_AGY_STRICT_VERSION")
        .ok()
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "on" | "yes"))
        .unwrap_or(false)
}

#[instrument(skip_all)]
pub fn agent_verbosity() -> AgentVerbosity {
    let raw = std::env::var("TRIUMVIRATE_AGENT_VERBOSITY").ok();
    let parsed = AgentVerbosity::from_env(raw.as_deref());
    if let Some(value) = raw.as_deref() {
        let normalized = value.to_lowercase();
        let valid = matches!(
            normalized.as_str(),
            "quiet" | "minimal" | "standard" | "normal" | "detailed" | "verbose" | "raw" | "debug"
        );
        if !valid {
            tracing::warn!(
                "invalid TRIUMVIRATE_AGENT_VERBOSITY={value:?}, defaulting to standard"
            );
        }
    }
    parsed
}

#[instrument(skip_all)]
pub fn gemini_streaming_enabled() -> bool {
    std::env::var("TRIUMVIRATE_GEMINI_STREAMING")
        .ok()
        .map(|v| !matches!(v.to_lowercase().as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(true)
}

#[instrument(skip_all)]
pub fn codex_protocol() -> String {
    std::env::var("TRIUMVIRATE_CODEX_PROTOCOL")
        .ok()
        .map(|v| match v.to_lowercase().as_str() {
            "app-server" => "app-server".to_string(),
            _ => "exec".to_string(),
        })
        .unwrap_or_else(|| "exec".to_string())
}

fn resolve_connector_command(
    bin_env: &str,
    args_env: &str,
    default_bin: &str,
) -> (String, Vec<String>) {
    let bin = std::env::var(bin_env).unwrap_or_else(|_| default_bin.to_string());
    let args = std::env::var(args_env)
        .map(|v| v.split_whitespace().map(ToString::to_string).collect())
        .unwrap_or_else(|_| Vec::new());
    (bin, args)
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use agent_adapter::AgentVerbosity;
    use shared_types::AskAgentRequest;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn gemini_backend_defaults_to_gemini_cli_and_only_agy_selects_agy() {
        // REQ-001/002 + REQ-083 rollback config: unset or ANY non-"agy" value is the
        // legacy gemini-cli path; only exactly "agy" selects the agy backend.
        let _guard = env_lock().lock().expect("env lock poisoned");
        unsafe { std::env::remove_var("TRIUMVIRATE_GEMINI_BACKEND") };
        assert_eq!(super::gemini_backend(), super::GeminiBackend::GeminiCli);
        unsafe { std::env::set_var("TRIUMVIRATE_GEMINI_BACKEND", "agy") };
        assert_eq!(super::gemini_backend(), super::GeminiBackend::Agy);
        unsafe { std::env::set_var("TRIUMVIRATE_GEMINI_BACKEND", "gemini-cli") };
        assert_eq!(super::gemini_backend(), super::GeminiBackend::GeminiCli);
        unsafe { std::env::set_var("TRIUMVIRATE_GEMINI_BACKEND", "anything-else") };
        assert_eq!(super::gemini_backend(), super::GeminiBackend::GeminiCli);
        unsafe { std::env::remove_var("TRIUMVIRATE_GEMINI_BACKEND") };
    }

    #[test]
    fn shadow_counterpart_is_the_other_backend() {
        assert_eq!(
            super::GeminiBackend::GeminiCli.shadow_counterpart(),
            super::GeminiBackend::Agy
        );
        assert_eq!(
            super::GeminiBackend::Agy.shadow_counterpart(),
            super::GeminiBackend::GeminiCli
        );
    }

    #[test]
    fn gemini_shadow_is_opt_in() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        unsafe { std::env::remove_var("TRIUMVIRATE_GEMINI_SHADOW") };
        assert!(!super::gemini_shadow_enabled(), "off by default");
        unsafe { std::env::set_var("TRIUMVIRATE_GEMINI_SHADOW", "on") };
        assert!(super::gemini_shadow_enabled());
        unsafe { std::env::set_var("TRIUMVIRATE_GEMINI_SHADOW", "0") };
        assert!(!super::gemini_shadow_enabled());
        unsafe { std::env::remove_var("TRIUMVIRATE_GEMINI_SHADOW") };
    }

    #[test]
    fn default_name_is_stable() {
        let info = super::BridgeInfo::default();
        assert_eq!(info.name, "triumvirate-mcp-bridge");
    }

    #[test]
    fn supported_agent_validation_is_explicit() {
        assert!(super::is_supported_agent(&AskAgentRequest {
            agent: "gemini".to_string(),
            message: "x".to_string(),
            cwd: None,
            repo: None,
            branch: None,
            ..Default::default()
        }));
        assert!(super::is_supported_agent(&AskAgentRequest {
            agent: "claude".to_string(),
            message: "x".to_string(),
            cwd: None,
            repo: None,
            branch: None,
            ..Default::default()
        }));
        assert!(super::is_supported_agent_name("gemini"));
        assert!(super::is_supported_agent_name("claude"));
        assert!(super::is_supported_agent_name("agy"));
        assert!(!super::is_supported_agent_name("unknown"));
    }

    // T-001: deepseek joins the supported-agent set as a top-level name.
    // Stub guard: a function that always returns `false` fails the deepseek/gemini/codex
    // asserts; a function that always returns `true` fails the unknown/empty asserts.
    #[test]
    fn supports_deepseek_name() {
        assert!(super::is_supported_agent_name("deepseek"));
        assert!(super::is_supported_agent_name("DeepSeek"));   // case-insensitive (fn lower-cases)
        assert!(super::is_supported_agent_name("DEEPSEEK"));
        assert!(super::is_supported_agent_name("gemini"));     // regression
        assert!(super::is_supported_agent_name("codex"));      // regression
        assert!(super::is_supported_agent_name("claude"));     // regression
        assert!(super::is_supported_agent_name("agy"));        // regression
        assert!(!super::is_supported_agent_name(""));          // negative
        assert!(!super::is_supported_agent_name("deep"));      // not a prefix match
        assert!(!super::is_supported_agent_name("deepseek-v4-pro")); // model id ≠ agent name
    }

    #[test]
    fn normalize_maps_antigravity_aliases_to_gemini_key() {
        // antigravity/agy are the product/CLI aliases of the internal `gemini` key.
        assert_eq!(super::normalize_agent_name("antigravity"), "gemini");
        assert_eq!(super::normalize_agent_name("Antigravity"), "gemini");
        assert_eq!(super::normalize_agent_name("agy"), "gemini");
        assert_eq!(super::normalize_agent_name("AGY"), "gemini");
        // canonical + other agents pass through lowercased, unchanged
        assert_eq!(super::normalize_agent_name("gemini"), "gemini");
        assert_eq!(super::normalize_agent_name("Codex"), "codex");
        assert_eq!(super::normalize_agent_name("deepseek"), "deepseek");
        // idempotent: normalizing an already-canonical alias result is a no-op
        assert_eq!(
            super::normalize_agent_name(&super::normalize_agent_name("agy")),
            "gemini"
        );
        // aliases satisfy the supported-name allowlist via the canonical key
        assert!(super::is_supported_agent_name("antigravity"));
    }

    #[test]
    fn daemon_proxy_toggle_parser_is_stable() {
        assert!(super::should_use_daemon_proxy(Some("1")));
        assert!(super::should_use_daemon_proxy(Some("true")));
        assert!(!super::should_use_daemon_proxy(Some("false")));
        assert!(!super::should_use_daemon_proxy(None));
    }

    #[test]
    fn daemon_proxy_env_reader_respects_truthy_and_falsey() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON") };
        assert!(super::use_daemon_for_mcp_from_env());
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::set_var("TRIUMVIRATE_MCP_USE_DAEMON", "1") };
        assert!(super::use_daemon_for_mcp_from_env());
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::set_var("TRIUMVIRATE_MCP_USE_DAEMON", "false") };
        assert!(!super::use_daemon_for_mcp_from_env());
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::remove_var("TRIUMVIRATE_MCP_USE_DAEMON") };
    }

    #[test]
    fn daemon_autostart_toggle_defaults_true_and_respects_falsey_values() {
        assert!(super::daemon_autostart_enabled(None));
        assert!(super::daemon_autostart_enabled(Some("true")));
        assert!(!super::daemon_autostart_enabled(Some("false")));
        assert!(!super::daemon_autostart_enabled(Some("0")));
    }

    #[test]
    fn bearer_authorization_compares_expected_token() {
        assert!(super::is_bearer_authorized(Some("Bearer abc"), "abc"));
        assert!(!super::is_bearer_authorized(Some("Bearer xyz"), "abc"));
        assert!(!super::is_bearer_authorized(None, "abc"));
    }

    #[test]
    fn daemon_url_builders_default_and_override() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_DAEMON_BASE_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_BIND_ADDR");
            std::env::remove_var("TRIUMVIRATE_DAEMON_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_ASK_AGENT_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_LEDGER_GC_URL");
        }

        assert_eq!(super::daemon_base_url(), "http://127.0.0.1:8080");
        assert_eq!(super::daemon_health_url(), "http://127.0.0.1:8080/health");
        assert_eq!(super::daemon_status_url(), "http://127.0.0.1:8080/status");
        assert_eq!(
            super::daemon_ask_agent_url(),
            "http://127.0.0.1:8080/ask-agent"
        );
        assert_eq!(super::daemon_ledger_gc_url(), "http://127.0.0.1:8080/ledger/gc");

        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::set_var("TRIUMVIRATE_DAEMON_BASE_URL", "http://127.0.0.1:9000");
            std::env::set_var("TRIUMVIRATE_DAEMON_BIND_ADDR", "127.0.0.1:9005");
            std::env::set_var("TRIUMVIRATE_DAEMON_HEALTH_URL", "http://127.0.0.1:9001/health");
            std::env::set_var("TRIUMVIRATE_DAEMON_URL", "http://127.0.0.1:9001/status");
            std::env::set_var(
                "TRIUMVIRATE_DAEMON_ASK_AGENT_URL",
                "http://127.0.0.1:9002/ask-agent",
            );
            std::env::set_var(
                "TRIUMVIRATE_DAEMON_LEDGER_GC_URL",
                "http://127.0.0.1:9002/ledger/gc",
            );
        }

        assert_eq!(super::daemon_base_url(), "http://127.0.0.1:9000");
        assert_eq!(super::daemon_health_url(), "http://127.0.0.1:9001/health");
        assert_eq!(super::daemon_status_url(), "http://127.0.0.1:9001/status");
        assert_eq!(super::daemon_ask_agent_url(), "http://127.0.0.1:9002/ask-agent");
        assert_eq!(super::daemon_ledger_gc_url(), "http://127.0.0.1:9002/ledger/gc");

        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_DAEMON_BASE_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_BIND_ADDR");
            std::env::remove_var("TRIUMVIRATE_DAEMON_HEALTH_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_ASK_AGENT_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_LEDGER_GC_URL");
        }
    }

    #[test]
    fn daemon_base_url_falls_back_to_bind_addr_when_base_url_missing() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_DAEMON_BASE_URL");
            std::env::set_var("TRIUMVIRATE_DAEMON_BIND_ADDR", "0.0.0.0:8123");
        }
        assert_eq!(super::daemon_base_url(), "http://0.0.0.0:8123");
        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_DAEMON_BIND_ADDR");
        }
    }

    #[test]
    fn daemon_status_url_uses_bind_addr_when_only_bind_is_set() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_DAEMON_BASE_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_HEALTH_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_MEMORY_READ_URL");
            std::env::set_var("TRIUMVIRATE_DAEMON_BIND_ADDR", "127.0.0.1:8456");
        }
        assert_eq!(super::daemon_health_url(), "http://127.0.0.1:8456/health");
        assert_eq!(super::daemon_status_url(), "http://127.0.0.1:8456/status");
        assert_eq!(
            super::daemon_memory_read_url(),
            "http://127.0.0.1:8456/memory/read"
        );
        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_DAEMON_BIND_ADDR");
            std::env::remove_var("TRIUMVIRATE_DAEMON_MEMORY_READ_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_HEALTH_URL");
        }
    }

    #[test]
    fn connector_command_resolution_defaults_and_overrides() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
            std::env::remove_var("TRIUMVIRATE_CODEX_BIN");
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
        }
        assert_eq!(super::gemini_command().0, "gemini");
        assert_eq!(super::codex_command().0, "codex");

        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", "gemini-cli");
            std::env::set_var("TRIUMVIRATE_GEMINI_ARGS", "--model pro");
            std::env::set_var("TRIUMVIRATE_CODEX_BIN", "codex-cli");
            std::env::set_var("TRIUMVIRATE_CODEX_ARGS", "--reasoning high");
        }
        assert_eq!(super::gemini_command().0, "gemini-cli");
        assert_eq!(
            super::gemini_command().1,
            vec!["--model".to_string(), "pro".to_string()]
        );
        assert_eq!(super::codex_command().0, "codex-cli");
        assert_eq!(
            super::codex_command().1,
            vec!["--reasoning".to_string(), "high".to_string()]
        );

        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
            std::env::remove_var("TRIUMVIRATE_CODEX_BIN");
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
        }
    }

    #[test]
    fn agent_verbosity_reader_defaults_and_overrides() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::remove_var("TRIUMVIRATE_AGENT_VERBOSITY") };
        assert_eq!(super::agent_verbosity(), AgentVerbosity::Standard);
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::set_var("TRIUMVIRATE_AGENT_VERBOSITY", "quiet") };
        assert_eq!(super::agent_verbosity(), AgentVerbosity::Quiet);
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::set_var("TRIUMVIRATE_AGENT_VERBOSITY", "detailed") };
        assert_eq!(super::agent_verbosity(), AgentVerbosity::Detailed);
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::set_var("TRIUMVIRATE_AGENT_VERBOSITY", "raw") };
        assert_eq!(super::agent_verbosity(), AgentVerbosity::Raw);
    }

    #[test]
    fn gemini_streaming_toggle_defaults_true_and_respects_falsey() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::remove_var("TRIUMVIRATE_GEMINI_STREAMING") };
        assert!(super::gemini_streaming_enabled());
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::set_var("TRIUMVIRATE_GEMINI_STREAMING", "false") };
        assert!(!super::gemini_streaming_enabled());
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::set_var("TRIUMVIRATE_GEMINI_STREAMING", "1") };
        assert!(super::gemini_streaming_enabled());
    }

    #[test]
    fn codex_protocol_defaults_exec_and_allows_app_server() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::remove_var("TRIUMVIRATE_CODEX_PROTOCOL") };
        assert_eq!(super::codex_protocol(), "exec");
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::set_var("TRIUMVIRATE_CODEX_PROTOCOL", "app-server") };
        assert_eq!(super::codex_protocol(), "app-server");
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::set_var("TRIUMVIRATE_CODEX_PROTOCOL", "unknown") };
        assert_eq!(super::codex_protocol(), "exec");
    }

    #[test]
    fn caller_driver_identity_reads_from_env() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::remove_var("TRIUMVIRATE_DRIVER") };
        assert_eq!(super::caller_driver_identity(), None);
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::set_var("TRIUMVIRATE_DRIVER", "gemini") };
        assert_eq!(super::caller_driver_identity(), Some("gemini".to_string()));
        unsafe { std::env::set_var("TRIUMVIRATE_DRIVER", "claude") };
        assert_eq!(super::caller_driver_identity(), Some("claude".to_string()));
    }
}
