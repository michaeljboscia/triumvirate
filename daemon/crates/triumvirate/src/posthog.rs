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
                "type":  kind,        // -> Exception::exception_type   (the grouping key)
                "value": message,     // -> Exception::exception_message
                "mechanism": { "handled": true, "type": "generic" },
            }],
            "tv_agent":    agent,
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
            "$ai_model":          g.agent,               // CLI agents don't report an exact model id
            "$ai_input_tokens":   g.input_tokens.unwrap_or(0),
            "$ai_output_tokens":  g.output_tokens.unwrap_or(0),
            "$ai_latency":        (g.duration_ms as f64) / 1000.0,   // seconds
            "$ai_is_error":       is_error,
            // --- Triumvirate-specific dimensions (what makes the dashboards useful) ---
            "tv_agent":            g.agent,
            "tv_outcome":          g.outcome,            // incl. "degraded_success"
            "tv_cached_tokens":    g.cached_tokens.unwrap_or(0),
            "tv_thinking_tokens":  g.thinking_tokens.unwrap_or(0),
            "tv_tool_calls":       g.tool_calls.unwrap_or(0),
            "tv_duration_ms":      g.duration_ms,
        }),
    );
}
