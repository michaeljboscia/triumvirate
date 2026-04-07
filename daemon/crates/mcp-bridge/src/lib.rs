//! MCP bridge crate boundary.
//!
//! This crate will own tool routing and stdio-facing concerns as we continue
//! extracting logic from the monolithic `triumvirate` binary crate.

use shared_types::AskAgentRequest;
use agent_adapter::AgentVerbosity;
use tracing::instrument;

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

#[instrument(skip_all)]
pub fn is_supported_agent_name(agent: &str) -> bool {
    let agent = agent.to_lowercase();
    agent == "gemini" || agent == "codex"
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
        }));
        assert!(!super::is_supported_agent(&AskAgentRequest {
            agent: "claude".to_string(),
            message: "x".to_string(),
            cwd: None,
            repo: None,
            branch: None,
        }));
        assert!(super::is_supported_agent_name("gemini"));
        assert!(!super::is_supported_agent_name("claude"));
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
        assert_eq!(super::gemini_command().0, "mock-gemini");
        assert_eq!(super::codex_command().0, "mock-codex");

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
}
