use triumvirate_proto::AgentId;

/// Output of the routing engine for a single human input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingDecision {
    Agent { agent: AgentId, content: String },
    Debate { topic: String },
    Fleet { spec: String },
    Status,
}

/// Decide how to route a user message.
///
/// Rules:
/// - `@claude/@gemini/@codex` force-direct to a specific agent
/// - `/debate`, `/fleet`, `/status` become command decisions
/// - plain text defaults to a lead agent heuristic (GR1-D3)
pub fn decide_route(content: &str) -> RoutingDecision {
    let trimmed = content.trim();

    if let Some(rest) = trimmed.strip_prefix("@claude") {
        return RoutingDecision::Agent {
            agent: AgentId::Claude,
            content: rest.trim().to_string(),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("@gemini") {
        return RoutingDecision::Agent {
            agent: AgentId::Gemini,
            content: rest.trim().to_string(),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("@codex") {
        return RoutingDecision::Agent {
            agent: AgentId::Codex,
            content: rest.trim().to_string(),
        };
    }

    if let Some(rest) = trimmed.strip_prefix("/debate") {
        return RoutingDecision::Debate {
            topic: rest.trim().to_string(),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("/fleet") {
        return RoutingDecision::Fleet {
            spec: rest.trim().to_string(),
        };
    }
    if trimmed.eq_ignore_ascii_case("/status") {
        return RoutingDecision::Status;
    }

    let lower = trimmed.to_ascii_lowercase();

    // Lead-agent heuristic adapted from APP_FLOW.md and TEST_PLAN routing rules.
    if contains_any(&lower, &["research", "best practice", "compare", "analyze", "investigate"]) {
        return RoutingDecision::Agent {
            agent: AgentId::Gemini,
            content: trimmed.to_string(),
        };
    }
    if contains_any(&lower, &["implement", "write", "build", "refactor", "fix", "code"]) {
        return RoutingDecision::Agent {
            agent: AgentId::Codex,
            content: trimmed.to_string(),
        };
    }

    RoutingDecision::Agent {
        agent: AgentId::Claude,
        content: trimmed.to_string(),
    }
}

fn contains_any(input: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| input.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::{RoutingDecision, decide_route};
    use triumvirate_proto::AgentId;

    #[test]
    fn routes_at_mention_claude() {
        let decision = decide_route("@claude design the API");
        assert!(matches!(
            decision,
            RoutingDecision::Agent {
                agent: AgentId::Claude,
                ..
            }
        ));
    }

    #[test]
    fn routes_at_mention_codex() {
        let decision = decide_route("@codex implement auth.rs");
        assert!(matches!(
            decision,
            RoutingDecision::Agent {
                agent: AgentId::Codex,
                ..
            }
        ));
    }

    #[test]
    fn defaults_architecture_to_claude() {
        let decision = decide_route("How should we design the auth system?");
        assert!(matches!(
            decision,
            RoutingDecision::Agent {
                agent: AgentId::Claude,
                ..
            }
        ));
    }

    #[test]
    fn defaults_research_to_gemini() {
        let decision = decide_route("What are JWT best practices?");
        assert!(matches!(
            decision,
            RoutingDecision::Agent {
                agent: AgentId::Gemini,
                ..
            }
        ));
    }

    #[test]
    fn defaults_implementation_to_codex() {
        let decision = decide_route("Write the auth module");
        assert!(matches!(
            decision,
            RoutingDecision::Agent {
                agent: AgentId::Codex,
                ..
            }
        ));
    }

    #[test]
    fn parses_debate_command() {
        let decision = decide_route("/debate Redis vs Postgres");
        assert!(matches!(decision, RoutingDecision::Debate { .. }));
    }

    #[test]
    fn parses_fleet_command() {
        let decision = decide_route("/fleet 3 claude, 2 codex: build auth");
        assert!(matches!(decision, RoutingDecision::Fleet { .. }));
    }
}
