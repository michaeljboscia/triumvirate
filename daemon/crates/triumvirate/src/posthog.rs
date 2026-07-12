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
    trace_id: String,
    started: std::time::Instant,
    outcome: &'static str,
    tokens: Option<agent_adapter::TokenUsage>,
    detail: Option<String>,
}

impl CallTelemetry {
    pub(crate) fn new(agent: &str, trace_id: &str) -> Self {
        Self {
            agent: agent.to_string(),
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
        record_ai_generation(&AiGeneration {
            agent: &self.agent,
            outcome: self.outcome,
            trace_id: &self.trace_id,
            input_tokens: u.and_then(|x| x.input),
            output_tokens: u.and_then(|x| x.output),
            cached_tokens: u.and_then(|x| x.cached),
            thinking_tokens: u.and_then(|x| x.thinking_tokens),
            tool_calls: u.and_then(|x| x.tool_calls),
            duration_ms: self.started.elapsed().as_millis() as u64,
        });

        // A failure is also an issue in error tracking. Only on a real failure — a
        // degraded_success still produced an answer.
        if self.outcome == "failure" {
            let detail = self.detail.as_deref().unwrap_or("unknown error");
            record_exception(&self.agent, "AgentCallFailed", detail, &self.trace_id);
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
