//! Shared DTOs and enums for MCP bridge <-> daemon communication.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentKind {
    Gemini,
    Codex,
}

impl AgentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentKind::Gemini => "gemini",
            AgentKind::Codex => "codex",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleState {
    pub state: String,
    pub detail: String,
}

#[cfg(test)]
mod tests {
    #[test]
    fn agent_kind_strings_are_stable() {
        assert_eq!(super::AgentKind::Gemini.as_str(), "gemini");
        assert_eq!(super::AgentKind::Codex.as_str(), "codex");
    }

    #[test]
    fn lifecycle_state_holds_values() {
        let s = super::LifecycleState {
            state: "DONE".to_string(),
            detail: "ok".to_string(),
        };
        assert_eq!(s.state, "DONE");
        assert_eq!(s.detail, "ok");
    }
}
