//! MCP bridge crate boundary.
//!
//! This crate will own tool routing and stdio-facing concerns as we continue
//! extracting logic from the monolithic `triumvirate` binary crate.

use shared_types::{AskAgentRequest, AskTwinsRequest};

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

pub fn build_role_adapted_prompts(req: &AskTwinsRequest) -> (String, String) {
    let gemini_prompt = format!(
        "[Gemini role: research/analysis]\nQuestion: {}\nContext: cwd={:?} repo={:?} branch={:?}",
        req.message, req.cwd, req.repo, req.branch
    );
    let codex_prompt = format!(
        "[Codex role: implementation/testing]\nQuestion: {}\nContext: cwd={:?} repo={:?} branch={:?}",
        req.message, req.cwd, req.repo, req.branch
    );
    (gemini_prompt, codex_prompt)
}

pub fn is_supported_agent(req: &AskAgentRequest) -> bool {
    is_supported_agent_name(&req.agent)
}

pub fn is_supported_agent_name(agent: &str) -> bool {
    let agent = agent.to_lowercase();
    agent == "gemini" || agent == "codex"
}

pub fn should_use_daemon_proxy(var_value: Option<&str>) -> bool {
    var_value
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

pub fn use_daemon_for_mcp_from_env() -> bool {
    should_use_daemon_proxy(std::env::var("TRIUMVIRATE_MCP_USE_DAEMON").ok().as_deref())
}

pub fn daemon_autostart_enabled(var_value: Option<&str>) -> bool {
    var_value
        .map(|v| !matches!(v.to_lowercase().as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(true)
}

pub fn is_bearer_authorized(raw_auth_header: Option<&str>, token: &str) -> bool {
    let expected = format!("Bearer {token}");
    raw_auth_header.map(|v| v == expected).unwrap_or(false)
}

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

pub fn daemon_status_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_URL")
        .unwrap_or_else(|_| format!("{}/status", daemon_base_url()))
}

pub fn daemon_health_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_HEALTH_URL")
        .unwrap_or_else(|_| format!("{}/health", daemon_base_url()))
}

pub fn daemon_ask_agent_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_ASK_AGENT_URL")
        .unwrap_or_else(|_| format!("{}/ask-agent", daemon_base_url()))
}

pub fn daemon_ask_twins_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_ASK_TWINS_URL")
        .unwrap_or_else(|_| format!("{}/ask-twins", daemon_base_url()))
}

pub fn daemon_session_spawn_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SESSION_SPAWN_URL")
        .unwrap_or_else(|_| format!("{}/session/spawn", daemon_base_url()))
}

pub fn daemon_session_ask_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SESSION_ASK_URL")
        .unwrap_or_else(|_| format!("{}/session/ask", daemon_base_url()))
}

pub fn daemon_session_dismiss_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SESSION_DISMISS_URL")
        .unwrap_or_else(|_| format!("{}/session/dismiss", daemon_base_url()))
}

pub fn daemon_session_list_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SESSION_LIST_URL")
        .unwrap_or_else(|_| format!("{}/session/list", daemon_base_url()))
}

pub fn daemon_memory_write_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_MEMORY_WRITE_URL")
        .unwrap_or_else(|_| format!("{}/memory/write", daemon_base_url()))
}

pub fn daemon_memory_read_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_MEMORY_READ_URL")
        .unwrap_or_else(|_| format!("{}/memory/read", daemon_base_url()))
}

pub fn daemon_scratchpad_write_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SCRATCHPAD_WRITE_URL")
        .unwrap_or_else(|_| format!("{}/scratchpad/write", daemon_base_url()))
}

pub fn daemon_scratchpad_list_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_SCRATCHPAD_LIST_URL")
        .unwrap_or_else(|_| format!("{}/scratchpad/list", daemon_base_url()))
}

pub fn daemon_outbox_recent_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_OUTBOX_RECENT_URL")
        .unwrap_or_else(|_| format!("{}/outbox/recent", daemon_base_url()))
}

pub fn daemon_fallback_list_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_FALLBACK_LIST_URL")
        .unwrap_or_else(|_| format!("{}/fallback/list", daemon_base_url()))
}

pub fn daemon_fallback_ack_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_FALLBACK_ACK_URL")
        .unwrap_or_else(|_| format!("{}/fallback/ack", daemon_base_url()))
}

pub fn daemon_fallback_gc_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_FALLBACK_GC_URL")
        .unwrap_or_else(|_| format!("{}/fallback/gc", daemon_base_url()))
}

pub fn gemini_command() -> (String, Vec<String>) {
    resolve_connector_command("TRIUMVIRATE_GEMINI_BIN", "TRIUMVIRATE_GEMINI_ARGS", "mock-gemini")
}

pub fn codex_command() -> (String, Vec<String>) {
    resolve_connector_command("TRIUMVIRATE_CODEX_BIN", "TRIUMVIRATE_CODEX_ARGS", "mock-codex")
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

    use shared_types::{AskAgentRequest, AskTwinsRequest};

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
    fn prompt_builder_includes_role_labels() {
        let req = AskTwinsRequest {
            message: "Add auth".to_string(),
            cwd: Some("/tmp".to_string()),
            repo: Some("triumvirate".to_string()),
            branch: Some("feat/mcp-first".to_string()),
        };
        let (gemini, codex) = super::build_role_adapted_prompts(&req);
        assert!(gemini.contains("[Gemini role: research/analysis]"));
        assert!(codex.contains("[Codex role: implementation/testing]"));
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
        assert!(!super::use_daemon_for_mcp_from_env());
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
        }

        assert_eq!(super::daemon_base_url(), "http://127.0.0.1:8080");
        assert_eq!(super::daemon_health_url(), "http://127.0.0.1:8080/health");
        assert_eq!(super::daemon_status_url(), "http://127.0.0.1:8080/status");
        assert_eq!(
            super::daemon_ask_agent_url(),
            "http://127.0.0.1:8080/ask-agent"
        );

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
        }

        assert_eq!(super::daemon_base_url(), "http://127.0.0.1:9000");
        assert_eq!(super::daemon_health_url(), "http://127.0.0.1:9001/health");
        assert_eq!(super::daemon_status_url(), "http://127.0.0.1:9001/status");
        assert_eq!(super::daemon_ask_agent_url(), "http://127.0.0.1:9002/ask-agent");

        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_DAEMON_BASE_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_BIND_ADDR");
            std::env::remove_var("TRIUMVIRATE_DAEMON_HEALTH_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_ASK_AGENT_URL");
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
}
