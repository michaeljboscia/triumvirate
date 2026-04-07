use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use triumvirate_proto::AgentId;

#[derive(Debug, Clone, Deserialize)]
pub struct PricingConfig {
    #[serde(default = "default_claude_input_per_mtok")]
    pub claude_input_per_mtok: f64,
    #[serde(default = "default_claude_output_per_mtok")]
    pub claude_output_per_mtok: f64,
    #[serde(default = "default_codex_input_per_mtok")]
    pub codex_input_per_mtok: f64,
    #[serde(default = "default_codex_output_per_mtok")]
    pub codex_output_per_mtok: f64,
    #[serde(default = "default_gemini_input_per_mtok")]
    pub gemini_input_per_mtok: f64,
    #[serde(default = "default_gemini_output_per_mtok")]
    pub gemini_output_per_mtok: f64,
}

impl Default for PricingConfig {
    fn default() -> Self {
        Self {
            claude_input_per_mtok: default_claude_input_per_mtok(),
            claude_output_per_mtok: default_claude_output_per_mtok(),
            codex_input_per_mtok: default_codex_input_per_mtok(),
            codex_output_per_mtok: default_codex_output_per_mtok(),
            gemini_input_per_mtok: default_gemini_input_per_mtok(),
            gemini_output_per_mtok: default_gemini_output_per_mtok(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentCostBreakdown {
    pub turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: f64,
}

pub fn estimate_cost_usd(agent: AgentId, input_tokens: u64, output_tokens: u64, pricing: &PricingConfig) -> f64 {
    let (input_rate, output_rate) = match agent {
        AgentId::Claude => (pricing.claude_input_per_mtok, pricing.claude_output_per_mtok),
        AgentId::Gemini => (pricing.gemini_input_per_mtok, pricing.gemini_output_per_mtok),
        AgentId::Codex => (pricing.codex_input_per_mtok, pricing.codex_output_per_mtok),
        AgentId::Human | AgentId::System => (0.0, 0.0),
    };
    ((input_tokens as f64 / 1_000_000.0) * input_rate)
        + ((output_tokens as f64 / 1_000_000.0) * output_rate)
}

pub fn estimate_costs_by_agent(
    token_totals: HashMap<AgentId, (u64, u64, u64)>,
    pricing: &PricingConfig,
) -> HashMap<AgentId, AgentCostBreakdown> {
    token_totals
        .into_iter()
        .map(|(agent, (turns, input_tokens, output_tokens))| {
            let estimated_cost_usd =
                estimate_cost_usd(agent, input_tokens, output_tokens, pricing);
            (
                agent,
                AgentCostBreakdown {
                    turns,
                    input_tokens,
                    output_tokens,
                    estimated_cost_usd,
                },
            )
        })
        .collect()
}

fn default_claude_input_per_mtok() -> f64 {
    5.0
}
fn default_claude_output_per_mtok() -> f64 {
    25.0
}
fn default_codex_input_per_mtok() -> f64 {
    5.0
}
fn default_codex_output_per_mtok() -> f64 {
    15.0
}
fn default_gemini_input_per_mtok() -> f64 {
    0.0
}
fn default_gemini_output_per_mtok() -> f64 {
    0.0
}
