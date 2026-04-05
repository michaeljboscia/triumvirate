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

pub fn daemon_base_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string())
}

pub fn daemon_status_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_URL")
        .unwrap_or_else(|_| format!("{}/status", daemon_base_url()))
}

pub fn daemon_ask_agent_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_ASK_AGENT_URL")
        .unwrap_or_else(|_| format!("{}/ask-agent", daemon_base_url()))
}

pub fn daemon_ask_twins_url() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_ASK_TWINS_URL")
        .unwrap_or_else(|_| format!("{}/ask-twins", daemon_base_url()))
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

#[cfg(test)]
mod tests {
    use shared_types::{AskAgentRequest, AskTwinsRequest};

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
    fn daemon_url_builders_default_and_override() {
        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_DAEMON_BASE_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_ASK_AGENT_URL");
        }

        assert_eq!(super::daemon_base_url(), "http://127.0.0.1:8080");
        assert_eq!(super::daemon_status_url(), "http://127.0.0.1:8080/status");
        assert_eq!(
            super::daemon_ask_agent_url(),
            "http://127.0.0.1:8080/ask-agent"
        );

        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::set_var("TRIUMVIRATE_DAEMON_BASE_URL", "http://127.0.0.1:9000");
            std::env::set_var("TRIUMVIRATE_DAEMON_URL", "http://127.0.0.1:9001/status");
            std::env::set_var(
                "TRIUMVIRATE_DAEMON_ASK_AGENT_URL",
                "http://127.0.0.1:9002/ask-agent",
            );
        }

        assert_eq!(super::daemon_base_url(), "http://127.0.0.1:9000");
        assert_eq!(super::daemon_status_url(), "http://127.0.0.1:9001/status");
        assert_eq!(super::daemon_ask_agent_url(), "http://127.0.0.1:9002/ask-agent");

        // SAFETY: test controls env var lifecycle in-process.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_DAEMON_BASE_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_URL");
            std::env::remove_var("TRIUMVIRATE_DAEMON_ASK_AGENT_URL");
        }
    }
}
