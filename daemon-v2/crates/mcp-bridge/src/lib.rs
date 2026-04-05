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
    let agent = req.agent.to_lowercase();
    agent == "gemini" || agent == "codex"
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
    }
}
