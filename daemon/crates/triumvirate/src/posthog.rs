//! PostHog LLM analytics — emits one `$ai_generation` event per agent call.
//!
//! This is the Langfuse replacement. PostHog captures LLM observability as ordinary
//! analytics events (their docs: "All AI Observability events are captured as standard
//! PostHog events"), so there is no SDK and no new dependency — it is a plain POST on
//! `reqwest`, which this crate already carries.
//!
//! Opt-in via env, exactly like the existing OTEL exporter in `tracing_setup.rs`:
//!   POSTHOG_HOST     e.g. https://posthog.e5btools.com   (unset -> this module is a no-op)
//!   POSTHOG_API_KEY  e.g. phc_...                        (unset -> no-op)
//!
//! Fire-and-forget by construction: the POST is spawned onto the runtime and every error
//! is swallowed. Telemetry must never be able to fail an agent call.

use serde_json::json;

/// Everything we know about one completed agent call.
pub(crate) struct AiGeneration<'a> {
    /// Normalized agent name: "gemini" | "codex" | "deepseek".
    pub agent: &'a str,
    /// The concrete model, when we know it (DeepSeek). CLI agents don't report one.
    pub model: Option<&'a str>,
    /// Triumvirate's own outcome taxonomy: "success" | "failure" | "degraded_success" | ...
    pub outcome: &'a str,
    /// Groups every sibling call in one consult into a single trace.
    pub trace_id: &'a str,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub thinking_tokens: Option<u64>,
    pub tool_calls: Option<u64>,
    pub duration_ms: u64,
    /// Real USD for metered agents; 0.0 for subscription agents (their marginal cost IS zero);
    /// None when the model's price is unknown — we emit no cost rather than a fabricated one.
    pub cost_usd: Option<f64>,
    /// "metered" | "subscription" | "unknown". Never aggregate cost across these without it.
    pub billing: &'a str,
}

/// Emits exactly one `$ai_generation` per agent call, on `Drop` — so it fires no matter
/// which of `execute_ask_agent`'s many exits is taken.
///
/// This exists because the sprinkle-a-call-at-each-exit pattern demonstrably does not work.
/// Reviewers found three separate holes in it: two early returns (unsupported agent, oversized
/// DeepSeek payload) emitted nothing; `degraded_success` emitted nothing; and worst, the success
/// event was emitted *before* mandatory peer review ran, so a peer-review failure reported
/// `success` and then returned `Err`. Every one of those is a class of bug that reappears the
/// moment someone adds a new `return Err(..)` — so the emit is no longer something a caller can
/// forget to do.
///
/// The default outcome is `"unreported"`. If that ever shows up in PostHog it means a new exit
/// path was added that never classified itself — the dashboard tells on us instead of going quiet.
pub(crate) struct CallTelemetry {
    agent: String,
    /// Only meaningful for metered agents (DeepSeek), where price varies by model.
    model: Option<String>,
    trace_id: String,
    started: std::time::Instant,
    outcome: &'static str,
    tokens: Option<agent_adapter::TokenUsage>,
    detail: Option<String>,
}

impl CallTelemetry {
    pub(crate) fn new(agent: &str, trace_id: &str, model: Option<&str>) -> Self {
        Self {
            agent: agent.to_string(),
            model: model.map(str::to_string),
            trace_id: trace_id.to_string(),
            started: std::time::Instant::now(),
            outcome: "unreported",
            tokens: None,
            detail: None,
        }
    }

    /// The agent name is normalized *after* the guard is built (the unsupported-agent check
    /// runs first), so allow it to be corrected once it is known.
    pub(crate) fn set_agent(&mut self, agent: &str) {
        self.agent = agent.to_string();
    }

    pub(crate) fn success(&mut self, tokens: Option<agent_adapter::TokenUsage>) {
        self.outcome = "success";
        self.tokens = tokens;
    }

    pub(crate) fn degraded_success(&mut self, tokens: Option<agent_adapter::TokenUsage>) {
        self.outcome = "degraded_success";
        self.tokens = tokens;
    }

    pub(crate) fn failure(&mut self, detail: impl Into<String>) {
        self.outcome = "failure";
        self.detail = Some(detail.into());
    }
}

impl Drop for CallTelemetry {
    fn drop(&mut self) {
        let u = self.tokens.as_ref();
        let output = u.and_then(|x| x.output);
        let raw_input = u.and_then(|x| x.input);
        let raw_cached = u.and_then(|x| x.cached);

        // Normalize the two adapters' incompatible conventions BEFORE anything is priced or
        // charted, so $ai_input_tokens means the same thing for every agent.
        let (total_prompt, hit, miss) = normalize_prompt(
            &self.agent,
            raw_input.unwrap_or(0),
            raw_cached.unwrap_or(0),
        );
        let (usd, billing) = cost_usd(&self.agent, self.model.as_deref(), hit, miss, output);
        // Preserve "we got no usage at all" (None) vs "we got zeros".
        let input_tokens = raw_input.map(|_| total_prompt);
        let cached_tokens = raw_cached.map(|_| hit);

        record_ai_generation(&AiGeneration {
            agent: &self.agent,
            model: self.model.as_deref(),
            outcome: self.outcome,
            trace_id: &self.trace_id,
            input_tokens,
            output_tokens: output,
            cached_tokens,
            thinking_tokens: u.and_then(|x| x.thinking_tokens),
            tool_calls: u.and_then(|x| x.tool_calls),
            duration_ms: self.started.elapsed().as_millis() as u64,
            cost_usd: usd,
            billing,
        });

        // A failure is also an issue in error tracking. Only on a real failure — a
        // degraded_success still produced an answer.
        if self.outcome == "failure" {
            let detail = self.detail.as_deref().unwrap_or("unknown error");
            record_exception(&self.agent, "AgentCallFailed", detail, &self.trace_id);
        }
    }
}

/// How an agent is actually paid for. This is the difference between a number on your card
/// statement and a hypothetical.
///
/// Only DeepSeek bills per token here: it authenticates with a raw API key against
/// api.deepseek.com. Codex authenticates with ChatGPT OAuth tokens (`OPENAI_API_KEY` is unset in
/// ~/.codex/auth.json) and Claude runs on a Max subscription — for both, the MARGINAL cost of one
/// more call is exactly $0. Reporting a dollar figure for those would describe a world we don't
/// live in; the scarce resource there is quota, not money, and quota is measured in tokens.
enum Billing {
    /// Fixed-price plan. Marginal cost of a call is zero; tokens are the scarce unit.
    Subscription,
    /// Real money, per token. USD per 1M tokens.
    Metered {
        cache_hit_in: f64,
        cache_miss_in: f64,
        output: f64,
    },
    /// Metered, but at a price we do not know. Report NO cost rather than a made-up one.
    UnknownPrice,
}

/// Prices are USD per 1M tokens, from DeepSeek's official pricing page
/// (https://api-docs.deepseek.com/quick_start/pricing/, checked 2026-07-12). If a model is not
/// listed here we return UnknownPrice and emit no cost — a wrong cost is worse than no cost.
fn billing_for(agent: &str, model: Option<&str>) -> Billing {
    match agent {
        // Subscription-backed: ChatGPT OAuth (codex), Max plan (claude), Google plan (gemini/agy).
        "codex" | "claude" | "gemini" => Billing::Subscription,
        "deepseek" => match model.unwrap_or("deepseek-v4-flash") {
            // deepseek-chat / deepseek-reasoner are the legacy aliases of v4-flash.
            "deepseek-v4-flash" | "deepseek-chat" | "deepseek-reasoner" => Billing::Metered {
                cache_hit_in: 0.0028,
                cache_miss_in: 0.14,
                output: 0.28,
            },
            "deepseek-v4-pro" => Billing::Metered {
                cache_hit_in: 0.003625,
                cache_miss_in: 0.435,
                output: 0.87,
            },
            _ => Billing::UnknownPrice,
        },
        _ => Billing::UnknownPrice,
    }
}

/// The adapters DISAGREE about what `input` means, and getting this wrong silently misprices
/// every metered call:
///
/// - **codex** (OpenAI convention): `input_tokens` is the TOTAL prompt and *includes*
///   `cached_input_tokens`. So `cached ⊆ input`.
/// - **deepseek**: `mcp-bridge/src/deepseek.rs::map_usage` — the file that calls itself "the single
///   source of truth" — sets `input_tokens = prompt_cache_MISS_tokens` and
///   `cached_tokens = prompt_cache_HIT_tokens`. They are **disjoint**; the total prompt is their sum.
///
/// Observed live and impossible under the subset assumption: `input=46, cached=256`.
///
/// Returns `(total_prompt, cache_hit, cache_miss)`, normalized.
fn normalize_prompt(agent: &str, input: u64, cached: u64) -> (u64, u64, u64) {
    if agent == "deepseek" {
        // disjoint: input IS the miss count
        (input + cached, cached, input)
    } else {
        // subset: cached is part of input
        let hit = cached.min(input);
        (input, hit, input - hit)
    }
}

/// Cost of one call in USD, plus the label describing how it is billed.
fn cost_usd(
    agent: &str,
    model: Option<&str>,
    hit: u64,
    miss: u64,
    output: Option<u64>,
) -> (Option<f64>, &'static str) {
    match billing_for(agent, model) {
        // The honest number: one more call on a fixed plan costs nothing.
        Billing::Subscription => (Some(0.0), "subscription"),
        Billing::UnknownPrice => (None, "unknown"),
        Billing::Metered {
            cache_hit_in,
            cache_miss_in,
            output: out_price,
        } => {
            let usd = (hit as f64 / 1e6) * cache_hit_in
                + (miss as f64 / 1e6) * cache_miss_in
                + (output.unwrap_or(0) as f64 / 1e6) * out_price;
            (Some(usd), "metered")
        }
    }
}

/// Map an agent to the provider PostHog expects in `$ai_provider`.
fn provider_for(agent: &str) -> &'static str {
    match agent {
        "gemini" => "google",
        "codex" => "openai",
        "deepseek" => "deepseek",
        _ => "unknown",
    }
}

/// Emit a `$ai_generation` event. No-op unless POSTHOG_HOST and POSTHOG_API_KEY are both set.
/// POST one event to PostHog. Fire-and-forget; every error is swallowed, because a
/// telemetry failure must never be able to fail an agent call.
fn capture(event: &str, properties: serde_json::Value) {
    let (Ok(host), Ok(key)) = (
        std::env::var("POSTHOG_HOST"),
        std::env::var("POSTHOG_API_KEY"),
    ) else {
        return; // not configured — stay silent, exactly like the OTEL exporter
    };

    let url = format!("{}/i/v0/e/", host.trim_end_matches('/'));
    let body = json!({
        "api_key": key,
        "event": event,
        "distinct_id": "triumvirate-daemon",
        "properties": properties,
    });

    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        let _ = client.post(&url).json(&body).send().await;
    });
}

/// Report a failed agent call to PostHog **error tracking** (the `$exception` event).
///
/// PostHog's docs say never to hand-build `$exception` because "the exception event schema
/// is strict" — and there is no Rust SDK to build it for us. So this is written against the
/// schema their ingestion service actually deserializes into, not against the docs:
/// `rust/cymbal/src/core/types/exception.rs`
///
///     pub struct Exception {
///         #[serde(rename = "type")]  pub exception_type: String,      // required
///         #[serde(rename = "value", default)] pub exception_message: String,
///         pub mechanism: Option<Mechanism>,   // optional
///         pub stacktrace: Option<Stacktrace>, // optional
///     }
///
/// `stacktrace` is optional, and we deliberately omit it: an agent failure is not a Rust
/// panic, so there is no meaningful frame list. PostHog then groups by type + message —
/// which is the axis worth grouping on anyway ("how often does codex time out?").
pub(crate) fn record_exception(agent: &str, kind: &str, message: &str, trace_id: &str) {
    capture(
        "$exception",
        json!({
            "$exception_list": [{
                "type":  kind,        // -> Exception::exception_type
                "value": message,     // -> Exception::exception_message (full, unredacted)
                // `handled: true` is honest: this is a failure we caught and returned as an Err,
                // not an uncaught crash. Marking it unhandled would imply the daemon died.
                "mechanism": { "handled": true, "type": "generic" },
            }],
            // Without a stacktrace PostHog groups by type + MESSAGE. Any varying token in a
            // message — a UUID, a duration, a byte count, a port — would then mint a brand-new
            // Issue for every single failure and bury the UI. Pin the grouping to the pair we
            // actually want to reason about ("how often does codex fail?") and let the message
            // stay detailed for the human reading the Issue.
            "$exception_fingerprint": [agent, kind],
            "tv_agent":     agent,
            "$ai_trace_id": trace_id,   // joins the exception to its $ai_generation
        }),
    );
}

pub(crate) fn record_ai_generation(g: &AiGeneration<'_>) {
    let is_error = g.outcome != "success" && g.outcome != "degraded_success";

    capture(
        "$ai_generation",
        json!({
            // --- PostHog's LLM analytics schema ---
            "$ai_trace_id":       g.trace_id,
            "$ai_provider":       provider_for(g.agent),
            "$ai_model":          g.model.unwrap_or(g.agent),  // CLI agents report no model id
            "$ai_input_tokens":   g.input_tokens.unwrap_or(0),
            "$ai_output_tokens":  g.output_tokens.unwrap_or(0),
            "$ai_latency":        (g.duration_ms as f64) / 1000.0,   // seconds
            "$ai_is_error":       is_error,
            // Real dollars ONLY for metered agents. 0.0 for a subscription is not a placeholder —
            // it is the true marginal cost of one more call on a fixed plan. Omitted entirely when
            // the price is unknown, so a chart can never silently sum a guess.
            "$ai_total_cost_usd": g.cost_usd,
            // --- Triumvirate-specific dimensions (what makes the dashboards useful) ---
            // tv_agent is the stable internal KEY ("gemini") — never rename it, or every chart
            // and saved insight built on historical rows silently splits in two. tv_agent_display
            // is the product name an operator should actually read ("Antigravity"). A dashboard
            // is a human surface, so it gets the human label; the key stays for continuity.
            "tv_agent":            g.agent,
            "tv_agent_display":    mcp_tools::display_agent_name(g.agent),
            "tv_outcome":          g.outcome,            // incl. "degraded_success"
            "tv_billing":          g.billing,            // metered | subscription | unknown
            "tv_cached_tokens":    g.cached_tokens.unwrap_or(0),
            "tv_thinking_tokens":  g.thinking_tokens.unwrap_or(0),
            "tv_tool_calls":       g.tool_calls.unwrap_or(0),
            "tv_duration_ms":      g.duration_ms,
            // The scarce unit on a subscription. Dollars can't move; this can.
            "tv_total_tokens":     g.input_tokens.unwrap_or(0) + g.output_tokens.unwrap_or(0),
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two adapters use OPPOSITE conventions for `input`. This test exists because the
    /// original cost function assumed the OpenAI one for both and silently underbilled DeepSeek.
    #[test]
    fn normalize_prompt_handles_both_adapter_conventions() {
        // deepseek: input IS the cache-miss count; cached is disjoint. Observed live: 46/256.
        assert_eq!(normalize_prompt("deepseek", 46, 256), (302, 256, 46));
        // codex/openai: cached is a subset of input.
        assert_eq!(
            normalize_prompt("codex", 189_930, 86_528),
            (189_930, 86_528, 103_402)
        );
    }

    /// Pins DeepSeek's published prices (api-docs.deepseek.com, 2026-07-12):
    /// v4-flash = $0.0028 hit / $0.14 miss / $0.28 out, per 1M tokens.
    #[test]
    fn deepseek_flash_is_priced_from_the_published_table() {
        let (usd, billing) = cost_usd("deepseek", None, 256, 46, Some(45));
        assert_eq!(billing, "metered");
        let expected =
            (256.0 / 1e6) * 0.0028 + (46.0 / 1e6) * 0.14 + (45.0 / 1e6) * 0.28;
        assert!((usd.unwrap() - expected).abs() < 1e-12);
    }

    /// A subscription call costs exactly nothing at the margin. Not a placeholder — the truth.
    #[test]
    fn subscription_agents_cost_zero_not_list_price() {
        for agent in ["codex", "claude", "gemini"] {
            let (usd, billing) = cost_usd(agent, None, 86_528, 103_402, Some(166));
            assert_eq!(billing, "subscription", "{agent}");
            assert_eq!(usd, Some(0.0), "{agent} must not be charged list price");
        }
    }

    /// An unknown model must emit NO cost. A wrong number is worse than a missing one.
    #[test]
    fn unknown_model_emits_no_cost() {
        let (usd, billing) = cost_usd("deepseek", Some("deepseek-v9-unreleased"), 10, 10, Some(10));
        assert_eq!(billing, "unknown");
        assert_eq!(usd, None);
    }
}
