//! PostHog LLM analytics, emits one `$ai_generation` event per agent call.
//!
//! Lives in mcp-bridge, not in the top-level binary crate, because it is the ONLY telemetry
//! home every dispatcher can reach: `triumvirate` depends on `fleet`, so fleet can never
//! depend back on `triumvirate`, and `deepseek_resilience` (which needs to report 429s that
//! cost real money) lives here too. A telemetry module that only the top crate can call is
//! a telemetry module that silently omits every other caller.
//!
//! This is the Langfuse replacement. PostHog captures LLM observability as ordinary
//! analytics events (their docs: "All AI Observability events are captured as standard
//! PostHog events"), so there is no SDK and no new dependency, it is a plain POST on
//! `reqwest`, which this crate already carries.
//!
//! Opt-in via env, exactly like the existing OTEL exporter in `tracing_setup.rs`:
//!   POSTHOG_HOST     e.g. https://us.i.posthog.com or your self-hosted host (unset -> no-op)
//!   POSTHOG_API_KEY  your project's phc_ ingest key                        (unset -> no-op)
//!
//! Fire-and-forget by construction: the POST is spawned onto the runtime and every error
//! is swallowed. Telemetry must never be able to fail an agent call.

use serde_json::json;

/// Everything we know about one completed agent call.
pub struct AiGeneration<'a> {
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
    /// None when the model's price is unknown, we emit no cost rather than a fabricated one.
    pub cost_usd: Option<f64>,
    /// "metered" | "subscription" | "unknown". Never aggregate cost across these without it.
    pub billing: &'a str,
    /// Which backend actually served this call ("agy" | "gemini-cli"), None for agents
    /// that have only one. Both backends report the same agent and provider, so without
    /// this the two are indistinguishable in PostHog. That is not hypothetical: the daemon
    /// ran the dead gemini-cli for four days while the config said agy, and no chart could
    /// show it, because the only process that dispatches reads its OWN env and had never
    /// inherited TRIUMVIRATE_GEMINI_BACKEND. "gemini-cli" appears here only as the name of
    /// the retired backend we must be able to SEE if it ever serves traffic again.
    pub backend: Option<&'a str>,
    /// Attempts spent against the PRIMARY provider. gemini-cli runs a 4-model faildown
    /// chain, so one request can burn 4 calls of shared quota while reporting one event.
    /// Charting quota against event COUNT undercounts by up to 4x without this. Excludes
    /// the degraded cross-provider hop, hence 0 is possible on a degraded success.
    pub attempts: u32,
    /// Repo NAME (not path), so cost and quota can be sliced by project.
    ///
    /// The resolver hands back `git rev-parse --show-toplevel`, an absolute path. Sending
    /// that raw would mint a new property value per checkout, make every breakdown unusable,
    /// and ship the operator's home directory to a SaaS. The basename is bounded by the
    /// number of projects, is readable in a chart (unlike a hash), and answers the actual
    /// question: which project is burning the quota?
    pub repo: Option<&'a str>,
    /// The actual prompt and completion. See CallTelemetry::input/output.
    pub input: Option<&'a str>,
    pub output: Option<&'a str>,
}

/// Reduce an absolute repo path to its bounded, readable name. Anything that is already a
/// bare name passes through unchanged.
pub(crate) fn repo_name(repo: &str) -> String {
    std::path::Path::new(repo.trim_end_matches('/'))
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| repo.to_string())
}

/// Emits exactly one `$ai_generation` per agent call, on `Drop`, so it fires no matter
/// which of `execute_ask_agent`'s many exits is taken.
///
/// This exists because the sprinkle-a-call-at-each-exit pattern demonstrably does not work.
/// Reviewers found three separate holes in it: two early returns (unsupported agent, oversized
/// DeepSeek payload) emitted nothing; `degraded_success` emitted nothing; and worst, the success
/// event was emitted *before* mandatory peer review ran, so a peer-review failure reported
/// `success` and then returned `Err`. Every one of those is a class of bug that reappears the
/// moment someone adds a new `return Err(..)`, so the emit is no longer something a caller can
/// forget to do.
///
/// The default outcome is `"unreported"`. If that ever shows up in PostHog it means a new exit
/// path was added that never classified itself, the dashboard tells on us instead of going quiet.
pub struct CallTelemetry {
    agent: String,
    /// Only meaningful for metered agents (DeepSeek), where price varies by model.
    model: Option<String>,
    trace_id: String,
    started: std::time::Instant,
    outcome: &'static str,
    tokens: Option<agent_adapter::TokenUsage>,
    detail: Option<String>,
    /// Which backend served the call. None until the dispatcher resolves it.
    backend: Option<&'static str>,
    /// Repo NAME, derived from the resolved repo path. See AiGeneration::repo.
    repo: Option<String>,
    /// Attempts actually SPENT, not the schedule length: a call that succeeds first try
    /// spent 1, even though gemini-cli's schedule holds 4. Counting the schedule would
    /// report a 4x quota burn that never happened.
    attempts: u32,
    /// The actual prompt sent to the model and the completion it returned. This is the CONTENT
    /// of AI observability: token counts tell you a call happened, these tell you WHAT. Scrubbed
    /// (credentials/PII masked) and capped in record_ai_generation before they leave the process.
    input: Option<String>,
    output: Option<String>,
    /// True once the dispatch has actually begun talking to the provider (past every
    /// synchronous validation gate). It exists to tell two "no outcome was recorded" cases
    /// apart at Drop:
    ///   - `in_flight == true`  → the future was CANCELLED mid-await (the caller's client-side
    ///     `ask_agent` timeout fired, or the client disconnected) after we committed to the
    ///     call. That is a real terminal outcome — the metered DeepSeek path routinely runs
    ///     past the 180s client ceiling — so it emits `tv_outcome = "cancelled"`, not the
    ///     `unreported` sentinel, and stays visible to outcome-based dispatch monitoring.
    ///   - `in_flight == false` → a synchronous exit returned without classifying itself. That
    ///     is the original canary the `unreported` default was built to catch, so it is left
    ///     untouched.
    in_flight: bool,
}

impl CallTelemetry {
    pub fn new(agent: &str, trace_id: &str, model: Option<&str>) -> Self {
        Self {
            agent: agent.to_string(),
            model: model.map(str::to_string),
            trace_id: trace_id.to_string(),
            started: std::time::Instant::now(),
            outcome: "unreported",
            tokens: None,
            detail: None,
            backend: None,
            repo: None,
            attempts: 0,
            input: None,
            output: None,
            in_flight: false,
        }
    }

    /// Mark the call as in-flight: we have cleared every synchronous validation gate and are
    /// about to (or already) talk to the provider. From here on, a Drop that finds no recorded
    /// outcome means the future was cancelled mid-await (client timeout / disconnect), which is
    /// classified as `"cancelled"` rather than left as the `unreported` sentinel. Idempotent.
    pub fn begin_dispatch(&mut self) {
        self.in_flight = true;
    }

    /// The `tv_outcome` string this call will actually emit on Drop. Split out so the
    /// cancelled-vs-unreported resolution is unit-testable without a live PostHog.
    ///
    /// `success` / `degraded_success` / `failure` always win — an explicitly recorded outcome
    /// is never overridden. Only the untouched `unreported` default is promoted, and only when
    /// the dispatch had begun (see `in_flight`): an in-flight drop is a cancellation; a
    /// not-yet-in-flight drop stays the `unreported` canary for a forgotten synchronous exit.
    pub(crate) fn effective_outcome(&self) -> &'static str {
        if self.outcome == "unreported" && self.in_flight {
            "cancelled"
        } else {
            self.outcome
        }
    }

    /// The prompt sent to the model, for `$ai_input`. Set at dispatch time.
    pub fn set_input(&mut self, input: &str) {
        self.input = Some(input.to_string());
    }

    /// The completion the model returned, for `$ai_output`. Set on success/degraded-success.
    pub fn set_output(&mut self, output: &str) {
        self.output = Some(output.to_string());
    }

    /// Record which backend actually served this call. Set as soon as the dispatcher
    /// resolves it, so it is present on every exit including early failures.
    pub fn set_backend(&mut self, backend: &'static str) {
        self.backend = Some(backend);
    }

    /// Record the CONCRETE model the client actually ran, once the response reveals it.
    /// Agy reports this at runtime ("Gemini 3.1 Pro (High)") and picks it itself, so it is
    /// knowable only after the call. Reporting the agent name as the model instead ("gemini")
    /// is useless: it cannot answer which model served the traffic, which is the entire
    /// question `$ai_model` exists to answer. The client is Agy; the model is a Gemini model.
    pub fn set_model(&mut self, model: &str) {
        self.model = Some(model.to_string());
    }

    /// Record which repo this call was made against, so cost and quota are sliceable by
    /// project. Stores the NAME only; the caller may pass a full path.
    pub fn set_repo(&mut self, repo: &str) {
        if !repo.trim().is_empty() {
            self.repo = Some(repo_name(repo));
        }
    }

    /// Count one dispatch attempt against the provider. Called per attempt, so a faildown
    /// chain reports the calls it really made against the shared quota pool.
    pub fn record_attempt(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
    }

    /// The agent name is normalized *after* the guard is built (the unsupported-agent check
    /// runs first), so allow it to be corrected once it is known.
    pub fn set_agent(&mut self, agent: &str) {
        self.agent = agent.to_string();
    }

    pub fn success(&mut self, tokens: Option<agent_adapter::TokenUsage>) {
        self.outcome = "success";
        self.tokens = tokens;
    }

    pub fn degraded_success(&mut self, tokens: Option<agent_adapter::TokenUsage>) {
        self.outcome = "degraded_success";
        self.tokens = tokens;
    }

    pub fn failure(&mut self, detail: impl Into<String>) {
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

        // Promote an in-flight drop with no recorded outcome to "cancelled" (see
        // effective_outcome). This is the fix for the DeepSeek dispatches that surfaced as
        // `unreported`: a metered call whose future the caller abandoned at the 180s client
        // ceiling never reached any classify() arm, so the guard used to emit the sentinel.
        let outcome = self.effective_outcome();

        record_ai_generation(&AiGeneration {
            agent: &self.agent,
            model: self.model.as_deref(),
            outcome,
            trace_id: &self.trace_id,
            input_tokens,
            output_tokens: output,
            cached_tokens,
            thinking_tokens: u.and_then(|x| x.thinking_tokens),
            tool_calls: u.and_then(|x| x.tool_calls),
            duration_ms: self.started.elapsed().as_millis() as u64,
            cost_usd: usd,
            billing,
            backend: self.backend,
            attempts: self.attempts,
            repo: self.repo.as_deref(),
            input: self.input.as_deref(),
            output: self.output.as_deref(),
        });

        // A failure is also an issue in error tracking. Only on a real failure, a
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
/// ~/.codex/auth.json) and Claude runs on a Max subscription, for both, the MARGINAL cost of one
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
/// listed here we return UnknownPrice and emit no cost, a wrong cost is worse than no cost.
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
/// - **deepseek**: `mcp-bridge/src/deepseek.rs::map_usage`, the file that calls itself "the single
///   source of truth", sets `input_tokens = prompt_cache_MISS_tokens` and
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
/// System-telemetry capture: all the `tv_*` / `$ai_generation` events belong to the daemon as
/// one actor, so they share a single stable distinct_id.
fn capture(event: &str, properties: serde_json::Value) {
    capture_as("triumvirate-daemon", event, properties);
}

/// Capture under a caller-supplied distinct_id. The MCP-analytics `$mcp_*` events use the MCP
/// SESSION id (not the daemon id), because PostHog's MCP Analytics dashboard is per-session:
/// it groups a client's tool calls into one session, which a shared daemon id would collapse.
fn capture_as(distinct_id: &str, event: &str, properties: serde_json::Value) {
    let (Ok(host), Ok(key)) = (
        std::env::var("POSTHOG_HOST"),
        std::env::var("POSTHOG_API_KEY"),
    ) else {
        return; // not configured, stay silent, exactly like the OTEL exporter
    };

    let url = format!("{}/i/v0/e/", host.trim_end_matches('/'));
    let body = json!({
        "api_key": key,
        "event": event,
        "distinct_id": distinct_id,
        "properties": properties,
    });

    // `tokio::spawn` PANICS outside a runtime. In the old home (a binary crate whose every
    // caller was async) that was safe by accident; here in mcp-bridge any sync helper can
    // call this, and a telemetry panic taking down a dispatcher is the worst possible trade.
    // Degrade to a warn instead: no runtime means no event, never a crash.
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        tracing::warn!(
            event = %event,
            "posthog event DROPPED: no tokio runtime on this thread (telemetry never panics a caller)"
        );
        return;
    };
    let event_name = event.to_string();
    handle.spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(event = %event_name, error = %e, "posthog client build failed");
                return;
            }
        };
        // Swallowing the RESULT (not just the failure) is how telemetry starts lying. A
        // dropped event and a delivered one looked identical from here, so "is this in
        // PostHog?" was unanswerable without a PostHog personal API key. Telemetry must
        // never fail the call, but it must never silently fail either.
        match client.post(&url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!(event = %event_name, status = %resp.status(), "posthog event accepted");
            }
            Ok(resp) => {
                let status = resp.status();
                let detail = resp.text().await.unwrap_or_default();
                tracing::warn!(
                    event = %event_name,
                    status = %status,
                    detail = %detail.chars().take(200).collect::<String>(),
                    "posthog REJECTED event"
                );
            }
            Err(e) => {
                tracing::warn!(event = %event_name, error = %e, "posthog POST failed");
            }
        }
    });
}

/// Report a failed agent call to PostHog **error tracking** (the `$exception` event).
///
/// PostHog's docs say never to hand-build `$exception` because "the exception event schema
/// is strict", and there is no Rust SDK to build it for us. So this is written against the
/// schema their ingestion service actually deserializes into, not against the docs:
/// `rust/cymbal/src/core/types/exception.rs`
///
/// ```text
/// pub struct Exception {
///     #[serde(rename = "type")]  pub exception_type: String,      // required
///     #[serde(rename = "value", default)] pub exception_message: String,
///     pub mechanism: Option<Mechanism>,   // optional
///     pub stacktrace: Option<Stacktrace>, // optional
/// }
/// ```
/// (Fenced as `text`: this is PostHog's struct quoted for reference, not ours. As an indented
/// block it was an implicit rust doctest — invisible while this module lived in a binary
/// crate, which does not run doctests, and a compile failure the moment it moved into a lib.)
///
/// `stacktrace` is optional, and we deliberately omit it: an agent failure is not a Rust
/// panic, so there is no meaningful frame list. PostHog then groups by type + message , 
/// which is the axis worth grouping on anyway ("how often does codex time out?").
pub fn record_exception(agent: &str, kind: &str, message: &str, trace_id: &str) {
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
            // message, a UUID, a duration, a byte count, a port, would then mint a brand-new
            // Issue for every single failure and bury the UI. Pin the grouping to the pair we
            // actually want to reason about ("how often does codex fail?") and let the message
            // stay detailed for the human reading the Issue.
            "$exception_fingerprint": [agent, kind],
            "tv_agent":     agent,
            "$ai_trace_id": trace_id,   // joins the exception to its $ai_generation
        }),
    );
}

/// Emit the config the daemon actually RESOLVED at startup (`tv_daemon_started`).
///
/// This exists because of a four-day outage that no surface could show. The daemon is the
/// only process that dispatches agents, and it resolves its backend from its OWN env. One
/// started without the MCP env block silently served the retired gemini-cli while every
/// client's config said agy. The config file was right, the dashboard was green, and the
/// logs were consistent, because all three described intent while the daemon acted on a
/// default nobody chose.
///
/// So: report the RESOLVED values, never the intended ones. `tv_backend` here is what
/// `gemini_backend()` actually returned in this process. If it ever reads "gemini-cli"
/// again, the drift is one chart away instead of four days deep.
pub fn record_daemon_started(backend: &str, agy_bin: &str, max_concurrent: usize, max_rpm: f64) {
    capture(
        "tv_daemon_started",
        json!({
            "tv_backend":            backend,
            "tv_agy_bin":            agy_bin,
            "tv_agy_max_concurrent": max_concurrent,
            "tv_agy_max_rpm":        max_rpm,
            "tv_version":            env!("CARGO_PKG_VERSION"),
            "tv_pid":                std::process::id(),
            // A dead backend is always a config defect. Chart this and the alert writes itself.
            "tv_backend_is_dead":    backend == "gemini-cli",
        }),
    );
}

/// Emit one `tv_fleet_task` per fleet agent run.
///
/// Fleet was completely dark: it dispatches agents against the same shared quota pool as the
/// ask path but emitted NOTHING to PostHog, so 429 counts undercounted reality and the
/// dashboard looked better the harder fleet was hammering. It was simultaneously the least
/// throttled path and the invisible one.
///
/// This is deliberately NOT an `$ai_generation`. Fleet launches a subprocess and reads only
/// its exit status, so it has no token usage and no model. Emitting a generation event with
/// zeroed tokens would let a chart silently sum fabricated data into the real totals; a
/// missing number is recoverable, a wrong one is not.
pub fn record_fleet_task(
    agent: &str,
    backend: Option<&str>,
    outcome: &str,
    duration_ms: u64,
    fleet_id: &str,
    task_id: &str,
) {
    capture(
        "tv_fleet_task",
        json!({
            "tv_agent":         agent,
            "tv_agent_display": crate::display_agent_name(agent),
            "tv_backend":       backend,
            // success | failed | degraded_success | timeout | skipped_breaker_open | launch_failed
            "tv_outcome":       outcome,
            "tv_duration_ms":   duration_ms,
            "tv_fleet_id":      fleet_id,
            "tv_task_id":       task_id,
            "tv_surface":       "fleet",
        }),
    );
}

/// Emit `tv_quota_breaker` when the agy circuit breaker changes state or blocks a call.
///
/// The breaker's whole job is to react to provider quota pressure, and it reported that
/// only to a `tracing::warn!` nobody was reading. Quota pressure is the single most
/// chartable signal this system has: it is the thing that actually rations the work.
/// `shed_count` is the number of calls refused during an OPEN epoch, reported as an
/// aggregate instead of one event per refused call. While the breaker is open EVERY request
/// is skipped, so per-call events would make event volume track traffic volume: the busier
/// the outage, the more it costs to observe, precisely when the signal is one bit ("still
/// open"). One event per epoch plus a count answers "how much work did quota pressure shed?"
/// without that.
pub fn record_breaker_event(event: &str, agent: &str, detail: &str, shed_count: Option<u64>) {
    capture(
        "tv_quota_breaker",
        json!({
            "tv_agent":         agent,
            "tv_agent_display": crate::display_agent_name(agent),
            // opened_shedding | half_open_probe | tripped_quota | tripped_other | recovered
            "tv_breaker_event": event,
            "tv_detail":        detail,
            "tv_shed_count":    shed_count,
        }),
    );
}

/// Emit `tv_rate_limit_wait` when OUR OWN limiter delays an agy call.
///
/// This is self-inflicted latency, and it is the early-warning signal for provider quota
/// pressure: the bucket starts shedding before the provider starts refusing. Only throttled
/// calls are reported, so any occurrence is meaningful.
///
/// The wait is BUCKETED, not raw. A raw millisecond value mints a distinct property value
/// per call, which is the high-cardinality garbage that makes a PostHog breakdown unusable.
/// Buckets answer the real question ("how bad, roughly?") and stay groupable forever.
pub fn record_rate_limit_wait(waited_ms: u64, configured_rpm: f64) {
    let bucket = match waited_ms {
        0..=99 => "<100ms",
        100..=499 => "100-500ms",
        500..=1_999 => "500ms-2s",
        2_000..=9_999 => "2-10s",
        10_000..=59_999 => "10-60s",
        _ => ">60s",
    };
    capture(
        "tv_rate_limit_wait",
        json!({
            "tv_agent":           "gemini",
            "tv_agent_display":   crate::display_agent_name("gemini"),
            "tv_wait_bucket":     bucket,
            "tv_configured_rpm":  configured_rpm,
            "tv_limiter":         "agy_rpm",
        }),
    );
}

/// Emit `tv_agy_health` for each health probe (REQ-056).
///
/// The probe already classifies the two failure modes that production traffic CANNOT tell
/// apart: a capture regression (exit 0 but empty stdout, indistinguishable from a legitimate
/// empty answer) versus a real backend failure. That classification existed only on the
/// `/health` endpoint, which means it was only ever seen by someone who already suspected a
/// problem. A probe result nobody polls is not monitoring.
pub fn record_health_probe(capture_health: &str, backend_health: &str, detail: &str) {
    capture(
        "tv_agy_health",
        json!({
            "tv_agent":          "gemini",
            "tv_agent_display":  crate::display_agent_name("gemini"),
            "tv_capture_health": capture_health,
            "tv_backend_health": backend_health,
            "tv_healthy":        capture_health == "ok" && backend_health == "ok",
            // Bounded: probe details are short and enumerable ("exit 2", "empty output").
            "tv_detail":         detail.chars().take(120).collect::<String>(),
        }),
    );
}

/// Emit `tv_session_invalidated` when a cached worker session turns out to be stale.
///
/// The ONLY outbox status worth mirroring to PostHog. The rest of that lifecycle
/// (SPAWNED/WORKING/DONE/FAILED, and the ~72% that is per-tool-call WORKING_EVENT noise)
/// either restates what `$ai_generation` already reports or multiplies event volume ~12x to
/// answer questions already answerable. This one is different: nothing in PostHog represents
/// it, and it marks the failure mode that silently orphans a named session's transcript.
///
/// Deliberately carries no request_id and no raw error text: both are unbounded, and this
/// event exists to be COUNTED and grouped ("which agent/repo keeps handing us dead session
/// ids?"), not to reconstruct a single call. The outbox remains the place for that.
pub fn record_session_invalidated(agent: &str, backend: Option<&str>, repo: Option<&str>) {
    capture(
        "tv_session_invalidated",
        json!({
            "tv_agent":         agent,
            "tv_agent_display": crate::display_agent_name(agent),
            "tv_backend":       backend,
            "tv_repo":          repo.map(repo_name),
        }),
    );
}

/// Emit `tv_agy_version_mismatch` when the daemon dispatches against an agy binary whose version
/// differs from the pinned expected version. Drift proceeds warn-only (unless strict), which is
/// easy to lose in logs — PostHog once found 70+ such warnings in 3h. This turns it into a
/// first-class, dashboard-able DEFECT signal. Bounded properties (both are version strings). The
/// caller emits it ONCE per process, so the tile counts "daemons booted with drifted agy", not the
/// per-dispatch warning noise.
pub fn record_agy_version_mismatch(installed: &str, expected: &str) {
    capture(
        "tv_agy_version_mismatch",
        json!({
            "tv_agent":                  "gemini",
            "tv_agent_display":          "Antigravity",
            "tv_agy_installed_version":  installed,
            "tv_agy_expected_version":   expected,
        }),
    );
}

/// Emit `tv_codex_dispatch` for a completed ABE codex dispatch (`dispatch_codex` and
/// `dispatch_codex_worktree`).
///
/// This path was fully dark. It does not go through `execute_ask_agent`: ABE spawns codex
/// itself and returns a task id immediately, so `CallTelemetry` never sees it. That made the
/// one surface where an agent WRITES TO THE REPO the least observable thing in the system,
/// while `ask_agent` (which only ever returns text) was fully instrumented.
///
/// Not an `$ai_generation`: ABE reads an exit status and a git sha, never tokens or a model.
/// Zeroed tokens would let a chart sum fabricated data into real totals.
///
/// `outcome` is the taxonomy that matters here, and it is NOT just pass/fail:
///
///   - completed  - exited 0 AND produced a commit
///   - cancelled  - an external cancel_task won the terminal transition
///   - stuck      - the watchdog saw no filesystem activity
///   - setup_failed / spawn_failed - never got as far as running
///   - no_commit  - exited 0 and produced NOTHING. The silent one. Codex reported success and
///     the repo is unchanged; without this it is indistinguishable from a real success on
///     every surface except a human going to look for the commit.
///   - failed     - non-zero exit
///   - timeout    - killed at the deadline
///   - wait_error - we lost track of the child
///
/// No task_id and no prompt: both unbounded. This event is for counting and grouping ("how
/// often does codex silently produce nothing, and in which repo?"); the local tracker stays
/// the place to inspect one dispatch.
pub fn record_codex_dispatch(
    surface: &str,
    outcome: &str,
    duration_ms: u64,
    repo: Option<&str>,
    files_changed: Option<usize>,
    exit_code: Option<i32>,
) {
    capture(
        "tv_codex_dispatch",
        json!({
            "tv_agent":         "codex",
            "tv_agent_display": crate::display_agent_name("codex"),
            // dispatch_codex | dispatch_codex_worktree
            "tv_surface":       surface,
            "tv_outcome":       outcome,
            "tv_duration_ms":   duration_ms,
            "tv_repo":          repo.map(repo_name),
            // Count, never the file NAMES: names are unbounded and are the diff's job.
            "tv_files_changed": files_changed,
            "tv_exit_code":     exit_code,
            // The headline: exit 0 with an unchanged repo. Chart this one.
            "tv_silent_no_op":  outcome == "no_commit",
        }),
    );
}

/// Emit `tv_token_scan` for a token-economics scan that stored records.
///
/// The scanner watches agent telemetry FILES on disk and attributes spend that did not
/// necessarily flow through `execute_ask_agent` (a raw codex/gemini CLI run, a backfill).
/// The entire token-economics crate had zero PostHog coverage before this.
///
/// Scope and honesty:
///   - This is OBSERVABILITY, not the accounting record. The token DB is the source of truth;
///     it is written before this fires. This event rides the shared in-process bus and can be
///     dropped under load (broadcast lag), so it may UNDERCOUNT. Never reconcile money from it.
///   - Emitted once per scanned file that stored records (`source` distinguishes an
///     incremental file-change scan from a reconciliation sweep). A reconciliation over many
///     files produces many events, one per file, not one per run.
///   - Empty scans emit nothing: a file change that added no billable records is not news.
pub fn record_token_scan(source: &str, records: u64, tokens: u64, cost_usd: f64, duration_ms: u64) {
    capture(
        "tv_token_scan",
        json!({
            // "incremental" (a watched file changed) | "reconciliation" (startup/backfill sweep)
            "tv_scan_source":   source,
            "tv_records":       records,
            "tv_tokens":        tokens,
            "tv_cost_usd":      cost_usd,
            "tv_duration_ms":   duration_ms,
        }),
    );
}

/// Emit `tv_fleet_spawn` when a fleet is launched (or planned).
///
/// The per-task `tv_fleet_task` events only fire once a REAL fleet runs its agents, so
/// "someone launched a fleet" as an intent was invisible, and the default `dry_run=true`
/// planning path emitted nothing at all. This is the intent-level counterpart: how often
/// fleets are spawned, at what width, real vs dry-run.
pub fn record_fleet_spawn(state: &str, dry_run: bool, agent_count: usize, repo: Option<&str>) {
    capture(
        "tv_fleet_spawn",
        json!({
            // planned (dry_run) | running (wait) | spawning (background)
            "tv_state":        state,
            "tv_dry_run":      dry_run,
            "tv_agent_count":  agent_count,
            "tv_repo":         repo.map(repo_name),
            "tv_surface":      "fleet",
        }),
    );
}

/// Emit `tv_review_requested` when a peer review is assigned. A cross-agent workflow that
/// emitted nothing: who reviews whom, and how often, was invisible. Agent names are
/// normalized (they arrive as raw MCP-request strings); review_type is capped so a caller
/// cannot mint unbounded property values through it.
pub fn record_review_requested(reviewer_agent: &str, author_agent: &str, review_type: &str, ok: bool) {
    // review_type is low-cardinality in practice ("code", "design", ...) but is raw input;
    // cap its length so an abusive value cannot explode the PostHog breakdown.
    let review_type: String = review_type.chars().take(32).collect();
    // normalize_agent_name passes UNKNOWN input through verbatim, so a raw name would be an
    // unbounded property. Collapse anything not in the known agent set to "other" — for the
    // DISPLAY field too (display_agent_name only capitalizes an unknown string, so a raw one
    // would still leak; Antigravity).
    let bound_agent = |a: &str| -> (String, String) {
        if crate::is_supported_agent_name(a) {
            (crate::normalize_agent_name(a), crate::display_agent_name(a))
        } else {
            ("other".to_string(), "Other".to_string())
        }
    };
    let (reviewer_key, reviewer_display) = bound_agent(reviewer_agent);
    let (author_key, _) = bound_agent(author_agent);
    capture(
        "tv_review_requested",
        json!({
            "tv_reviewer_agent":         reviewer_key,
            "tv_reviewer_display":       reviewer_display,
            "tv_author_agent":           author_key,
            "tv_review_type":            review_type,
            // A review the engine FAILED to assign is invisible if we only emit on success —
            // the survivorship trap the Drop-guard on the ask path exists to avoid.
            "tv_ok":                     ok,
        }),
    );
}

/// Emit `tv_review_verdict` when a peer review is submitted. The verdict is the high-signal
/// outcome and only ever reached local Prometheus before this. Comments (unbounded) are
/// omitted, and the verdict is NORMALIZED to a known set: it arrives as a raw MCP-request
/// String, so charting it verbatim would let a caller mint unbounded property values. An
/// unrecognized verdict collapses to "other" (with the raw kept out of PostHog).
pub fn record_review_verdict(verdict: &str, ok: bool) {
    let normalized = match verdict.trim().to_ascii_lowercase().as_str() {
        "clean" => "clean",
        "concerns" => "concerns",
        "regression" => "regression",
        "pass" => "pass",
        "fail" | "failure" => "fail",
        "skip" | "skipped" => "skip",
        _ => "other",
    };
    // An unknown verdict is LOGGED, not sent as a PostHog property. DeepSeek wanted the raw
    // signal preserved; Antigravity noted a capped raw is still unbounded cardinality (a
    // hallucinated 32-char string mints a new property every time). A warn keeps the signal
    // where humans and log search can see it without polluting PostHog's property index.
    if normalized == "other" {
        tracing::warn!(raw_verdict = %verdict.trim().chars().take(64).collect::<String>(),
            "peer review submitted an unrecognized verdict");
    }
    capture(
        "tv_review_verdict",
        json!({
            "tv_verdict": normalized,
            // A submit the engine rejected is invisible if we only emit on success.
            "tv_ok":      ok,
        }),
    );
}

/// Emit `tv_maintenance` for a background maintenance cycle (ledger sweep, temp-file sweep).
///
/// Deliberately NOT emitted on every quiet success: a sweep runs often and a clean no-op is
/// not news. Emit on FAILURE (the silent case, warn-only today) and on a non-trivial result
/// (files reaped = a leak signal). `count` is whatever the job counts (files removed, etc.).
pub fn record_maintenance(job: &str, outcome: &str, count: u64) {
    capture(
        "tv_maintenance",
        json!({
            "tv_job":     job,        // "ledger_sweep" | "temp_sweep"
            "tv_outcome": outcome,    // "ok" | "failed"
            "tv_count":   count,
        }),
    );
}

/// Emit `tv_deepseek_breaker` when the DeepSeek circuit breaker changes state.
///
/// The agy breaker had `tv_quota_breaker`; DeepSeek's had nothing, an asymmetry that mattered
/// because DeepSeek is the only METERED provider. The two states worth an alert:
///   - `hard_open_insufficient_balance`: HTTP 402, the account is OUT OF BALANCE. No automatic
///     recovery; an operator must refill or rotate the key. Completely invisible before this.
///   - `open_transient`: repeated 429/5xx tripped the breaker — the paid provider is throttling
///     or erroring, so paid traffic is being shed.
///
/// `to`/`from` are the state labels; both are a fixed low-cardinality set.
///
/// Note on recovery: `hard_open_insufficient_balance` is a PROCESS-LIFETIME LATCH — the breaker
/// has no reset() and try_acquire returns BlockHard, so record() never runs to move it out. It
/// clears only on a daemon restart (a fresh breaker starts Closed). So there is deliberately no
/// "recovered from out-of-balance" transition event; recovery shows up as the next
/// tv_daemon_started plus resumed successful deepseek $ai_generation, not here.
///
/// This is an EDGE trigger (fires once, on entry). If that one event drops under bus lag, a
/// persistent out-of-balance is still not silent: while latched, every deepseek call returns
/// BreakerOpen and emits a failing $ai_generation, continuously — a deepseek failure-rate alert
/// is the backstop for a missed entry event (Antigravity). deepseek is single-attempt
/// (REQ-DS-008), so a blocked call is ONE event, not a retry-loop storm.
pub fn record_deepseek_breaker(to_state: &str, from_state: &str) {
    capture(
        "tv_deepseek_breaker",
        json!({
            "tv_agent":         "deepseek",
            "tv_agent_display": crate::display_agent_name("deepseek"),
            "tv_breaker_to":    to_state,
            "tv_breaker_from":  from_state,
            // The one an alert should fire on: the metered account ran out of money.
            "tv_out_of_balance": to_state == "hard_open_insufficient_balance",
        }),
    );
}

// ---------------------------------------------------------------------------
// MCP Analytics ($mcp_* events)
//
// Mirrors PostHog's @posthog/mcp-analytics TypeScript SDK, which we cannot use because it
// wraps a JS/TS MCP server and Triumvirate's is Rust. The event names and property KEYS below
// are copied verbatim from PostHog's MCP Analytics events reference, because their purpose-built
// MCP Analytics dashboard queries these exact strings; a typo means the dashboard stays empty
// while the events 200-OK into the void. Triumvirate serves tools only (no resources/prompts),
// so only the tool-call / tools-list / initialize events apply.
// ---------------------------------------------------------------------------

/// The `$mcp_source` the dashboard filters on. Verbatim; do not "improve" it.
const MCP_SOURCE: &str = "posthog_mcp_analytics";
const MCP_SERVER_NAME: &str = "triumvirate";
/// Content capture, not a preview: the point of AI observability is seeing what was actually sent.
/// Per-string cap is generous (normal prompts/responses/args pass whole); it only bounds a
/// pathological single blob so one field can't dominate the event, with the byte cap as backstop.
/// NOTHING is masked (operator's explicit choice: everything raw and unmasked).
const MCP_PREVIEW_MAX_CHARS: usize = 16_384;

/// One string value from `$mcp_parameters`/`$mcp_response`: RAW, only size-capped. No credential/PII
/// masking, no path-basenaming — the operator wants the literal args every tool was called with.
/// The cap only stops one pathological blob from dominating the event.
fn mcp_preview(s: &str) -> String {
    let mut out: String = s.chars().take(MCP_PREVIEW_MAX_CHARS).collect();
    if s.chars().count() > MCP_PREVIEW_MAX_CHARS {
        out.push_str("...");
    }
    out
}

/// Bound the sanitized payload so `$mcp_parameters`/`$mcp_response` can never balloon: even with
/// every string capped, a deep/wide structure can exceed PostHog's 1MB event limit and get
/// silently dropped (Codex/DeepSeek). This is the ONLY thing the sanitizer still does — structural
/// size limiting to prevent data-loss, never masking.
const MCP_MAX_DEPTH: usize = 6;
const MCP_MAX_ENTRIES: usize = 40;

fn sanitize_mcp_json(v: &serde_json::Value) -> serde_json::Value {
    let sanitized = sanitize_mcp_json_inner(v, 0);
    // Total-byte cap on top of the structural caps: depth=6 x breadth=40 still admits a payload
    // far over PostHog's 1MB event limit, and oversize events are DROPPED SILENTLY (DeepSeek).
    // Keep each of params/response well under 1MB so the whole event always ingests.
    const MCP_MAX_BYTES: usize = 256 * 1024;
    let size = serde_json::to_string(&sanitized).map(|s| s.len()).unwrap_or(0);
    if size > MCP_MAX_BYTES {
        return serde_json::json!({ "<oversize_bytes>": size });
    }
    sanitized
}

fn sanitize_mcp_json_inner(v: &serde_json::Value, depth: usize) -> serde_json::Value {
    if depth >= MCP_MAX_DEPTH {
        return serde_json::Value::String("<truncated: max depth>".to_string());
    }
    match v {
        serde_json::Value::String(s) => serde_json::Value::String(mcp_preview(s)),
        serde_json::Value::Array(a) => {
            let mut out: Vec<serde_json::Value> = a
                .iter()
                .take(MCP_MAX_ENTRIES)
                .map(|x| sanitize_mcp_json_inner(x, depth + 1))
                .collect();
            if a.len() > MCP_MAX_ENTRIES {
                out.push(serde_json::Value::String(format!(
                    "<+{} more>",
                    a.len() - MCP_MAX_ENTRIES
                )));
            }
            serde_json::Value::Array(out)
        }
        serde_json::Value::Object(o) => {
            let mut m = serde_json::Map::new();
            for (k, val) in o.iter().take(MCP_MAX_ENTRIES) {
                // Raw: keep every key and value verbatim (only structural depth/breadth/size
                // limiting applies). No key-name redaction — operator wants everything unmasked.
                m.insert(k.clone(), sanitize_mcp_json_inner(val, depth + 1));
            }
            if o.len() > MCP_MAX_ENTRIES {
                m.insert(
                    "<truncated>".to_string(),
                    serde_json::json!(o.len() - MCP_MAX_ENTRIES),
                );
            }
            serde_json::Value::Object(m)
        }
        other => other.clone(),
    }
}

/// Emit `$mcp_tool_call` for one MCP tool invocation (all 54 tools, from the single call_tool
/// choke point). distinct_id is the MCP session id, per PostHog's per-session dashboard model.
#[allow(clippy::too_many_arguments)]
pub fn record_mcp_tool_call(
    session_id: &str,
    tool_name: &str,
    duration_ms: u64,
    is_error: bool,
    error_type: Option<&str>,
    client_name: Option<&str>,
    client_version: Option<&str>,
    parameters: Option<&serde_json::Value>,
    response: Option<&serde_json::Value>,
    // Why the agent called this tool. `$mcp_intent_source` says where it came from:
    // "context_parameter" (the agent authored it via the injected `context` arg) or "inferred"
    // (our server-side fallback). Matches PostHog's MCP Analytics agent-intent schema so the
    // intent panels populate. Absent (both None) only if we chose to emit nothing.
    intent: Option<&str>,
    intent_source: Option<&str>,
) {
    // $mcp_error_status is deliberately ABSENT: PostHog defines it as an upstream HTTP status
    // (429/500...), and a JSON-RPC error code is not one — putting the JSON-RPC code there was a
    // typed-field lie (Codex). $mcp_error_type carries the classification instead.
    capture_as(
        session_id,
        "$mcp_tool_call",
        json!({
            "$session_id":              session_id,
            // Session-scoped telemetry, not a human user: keep it personless so a busy client
            // does not mint one anonymous PostHog person per MCP session (Codex / posthog-core).
            "$process_person_profile":  false,
            "$mcp_source":              MCP_SOURCE,
            "$mcp_server_name":         MCP_SERVER_NAME,
            "$mcp_server_version":      env!("CARGO_PKG_VERSION"),
            "$mcp_client_name":         client_name,
            "$mcp_client_version":      client_version,
            "$mcp_tool_name":           tool_name,
            "$mcp_duration_ms":         duration_ms,
            "$mcp_is_error":            is_error,
            "$mcp_error_type":          error_type,
            // Agent intent (the closest thing to agent reasoning in the telemetry), RAW like the
            // rest of the content. `$mcp_intent_source` distinguishes an agent-authored intent from
            // our inferred fallback so a dashboard can weight them.
            "$mcp_intent":              intent,
            "$mcp_intent_source":       intent_source,
            "$mcp_parameters":          parameters.map(sanitize_mcp_json),
            "$mcp_response":            response.map(sanitize_mcp_json),
        }),
    );
}

/// Emit `$mcp_tools_list` (the advertised-vs-called signal: which of the 54 tools exist).
pub fn record_mcp_tools_list(
    session_id: &str,
    tool_names: &[String],
    client_name: Option<&str>,
    client_version: Option<&str>,
) {
    capture_as(
        session_id,
        "$mcp_tools_list",
        json!({
            "$session_id":              session_id,
            "$process_person_profile":  false,
            "$mcp_source":              MCP_SOURCE,
            "$mcp_server_name":         MCP_SERVER_NAME,
            "$mcp_server_version":      env!("CARGO_PKG_VERSION"),
            "$mcp_client_name":         client_name,
            "$mcp_client_version":      client_version,
            "$mcp_listed_tool_names":   tool_names,
        }),
    );
}

/// Emit `$mcp_initialize` for a client/server handshake (once per session).
pub fn record_mcp_initialize(session_id: &str, client_name: &str, client_version: &str) {
    capture_as(
        session_id,
        "$mcp_initialize",
        json!({
            "$session_id":              session_id,
            "$process_person_profile":  false,
            "$mcp_source":              MCP_SOURCE,
            "$mcp_server_name":         MCP_SERVER_NAME,
            "$mcp_server_version":      env!("CARGO_PKG_VERSION"),
            "$mcp_client_name":         client_name,
            "$mcp_client_version":      client_version,
        }),
    );
}

pub fn record_ai_generation(g: &AiGeneration<'_>) {
    capture("$ai_generation", ai_generation_props(g));
}

/// Emit a `$ai_generation` for a DISPATCHED codex worker so its "told" (the prompt/briefing it was
/// given) and "produced" (the diff it committed + its stdout, or the failure diagnosis) become
/// visible in PostHog's LLM Traces, the same surface `ask_agent` uses. Dispatch was previously a
/// black box: `tv_codex_dispatch` carried only the outcome, never the content.
///
/// Content is scrubbed + byte-capped by `mask_and_cap_content` (via `record_ai_generation`). MUST be
/// called ONLY by the winner of the terminal transition (caller gates on the `mark_*` bool), so a
/// cancel race cannot produce a second, contradictory trace. `surface` (dispatch_codex |
/// dispatch_codex_worktree) rides in `$ai_backend`/tv_backend to distinguish this from an ask_agent
/// generation. `trace_id` should be the Pantheon root/parent session id when present so the trace
/// nests under the parent agent, falling back to the task id.
pub fn record_dispatch_generation(
    trace_id: &str,
    surface: &'static str,
    repo: Option<&str>,
    told: &str,
    produced: &str,
    is_error: bool,
    duration_ms: u64,
) {
    // Basename the repo so tv_repo matches tv_codex_dispatch (bounded, readable, no home-dir leak),
    // whether the caller passed a name or a full path.
    let repo_basename = repo.map(repo_name);
    record_ai_generation(&AiGeneration {
        agent: "codex",
        model: None,
        outcome: if is_error { "failure" } else { "success" },
        trace_id,
        input_tokens: None,
        output_tokens: None,
        cached_tokens: None,
        thinking_tokens: None,
        tool_calls: None,
        duration_ms,
        cost_usd: None,
        billing: "subscription",
        backend: Some(surface),
        attempts: 0,
        repo: repo_basename.as_deref(),
        input: Some(told),
        output: Some(produced),
    });
}

/// Build the `$ai_generation` property bag. Split out from `record_ai_generation` so the exact
/// property shape (especially the structured `$ai_input`/`$ai_output_choices` the LLM UI keys on)
/// is unit-testable without a live PostHog.
fn ai_generation_props(g: &AiGeneration<'_>) -> serde_json::Value {
    let is_error = g.outcome != "success" && g.outcome != "degraded_success";

    let mut props = json!({
            // --- PostHog's LLM analytics schema ---
            "$ai_trace_id":       g.trace_id,
            "$ai_provider":       provider_for(g.agent),
            // "unknown" when we genuinely do not know, NEVER the agent's name.
            //
            // This first fell back to the internal key ("gemini"), which charted every
            // Antigravity call as the model "gemini" and answered nothing. The fix was to
            // fall back to the DISPLAY name instead, which was the same bug wearing better
            // clothes: "Antigravity" is a CLIENT, not a model, and putting it in $ai_model
            // still contaminates model analytics with a client name. The client is Agy; the
            // model is a Gemini model. When the connector does not tell us which, the honest
            // answer is that we do not know, and an "unknown" slice on the dashboard is the
            // signal that our model parsing has regressed.
            "$ai_model":          g.model.map(str::to_string)
                                    .unwrap_or_else(|| "unknown".to_string()),
            "$ai_input_tokens":   g.input_tokens.unwrap_or(0),
            "$ai_output_tokens":  g.output_tokens.unwrap_or(0),
            "$ai_latency":        (g.duration_ms as f64) / 1000.0,   // seconds
            "$ai_is_error":       is_error,
            // Real dollars ONLY for metered agents. 0.0 for a subscription is not a placeholder , 
            // it is the true marginal cost of one more call on a fixed plan. Omitted entirely when
            // the price is unknown, so a chart can never silently sum a guess.
            "$ai_total_cost_usd": g.cost_usd,
            // --- Triumvirate-specific dimensions (what makes the dashboards useful) ---
            // tv_agent is the stable internal KEY ("gemini"), never rename it, or every chart
            // and saved insight built on historical rows silently splits in two. tv_agent_display
            // is the product name an operator should actually read ("Antigravity"). A dashboard
            // is a human surface, so it gets the human label; the key stays for continuity.
            "tv_agent":            g.agent,
            "tv_agent_display":    crate::display_agent_name(g.agent),
            "tv_outcome":          g.outcome,            // incl. "degraded_success"
            "tv_billing":          g.billing,            // metered | subscription | unknown
            "tv_cached_tokens":    g.cached_tokens.unwrap_or(0),
            "tv_thinking_tokens":  g.thinking_tokens.unwrap_or(0),
            "tv_tool_calls":       g.tool_calls.unwrap_or(0),
            "tv_duration_ms":      g.duration_ms,
            // The scarce unit on a subscription. Dollars can't move; this can.
            "tv_total_tokens":     g.input_tokens.unwrap_or(0) + g.output_tokens.unwrap_or(0),
            // WHICH backend served this. agy and the retired gemini-cli report the same
            // agent and provider, so every chart treated them as one thing while the daemon
            // quietly ran gemini-cli for four days against a config that said agy. Absent
            // (null) when the agent has only one backend, never invented.
            "tv_backend":          g.backend,
            // Attempts spent against the PRIMARY provider only. gemini-cli's faildown chain
            // can burn 4 per request, so quota-vs-events is a 4x lie without this. Named
            // "primary" deliberately: the degraded cross-provider hop (agy -> codex) is NOT
            // counted here, so a degraded success legitimately reports 0 primary attempts.
            // Calling it "tv_attempts" would read as "no agent call happened", which is false.
            "tv_primary_attempts": g.attempts,
            // Which project burned this. Name only, never the absolute path.
            "tv_repo":             g.repo,
            // The CONTENT of the call: the actual prompt and completion, which is the whole point
            // of AI observability (token counts say a call happened; these say WHAT). Scrubbed
            // (credentials/PII masked) and capped so a huge completion cannot push the event past
            // PostHog's 1MB drop limit.
            //
            // These MUST be structured message/choice arrays, not flat strings. PostHog's LLM
            // trace UI keys on the {role, content} shape; a flat string ingests fine but the chat
            // bubbles never render, so you get a valid-but-invisible event, exactly the failure the
            // whole content-capture effort exists to avoid (Antigravity). $ai_input is a message
            // array; the completion goes in $ai_output_choices (NOT $ai_output). See below: these
            // two are inserted AFTER the literal, not here.
        });
    // Content goes in as explicit object keys. Doing `g.input.map(|s| json!([...]))` inside the
    // json! literal above yields an Option<Value> in value position, and json! DROPS that key from
    // the object entirely when it is Some(Value) (verified live: set_input ran, the tel held the
    // input at Drop, yet $ai_input never reached PostHog). Inserting into the Value::Object is
    // unambiguous: the key is present exactly when we have content.
    if let Some(s) = g.input {
        props["$ai_input"] = json!([{ "role": "user", "content": mask_and_cap_content(s) }]);
    }
    if let Some(s) = g.output {
        props["$ai_output_choices"] = json!([{ "role": "assistant", "content": mask_and_cap_content(s) }]);
    }
    props
}

/// Prepare captured prompt/completion text for PostHog: cap by BYTES only, NO scrubbing. The
/// operator's explicit choice (2026-07-22): they want to SEE the literal text of the call — the
/// whole point of AI observability — and the credential/PII masking was destroying exactly that
/// (it masked commit SHAs and authors in diffs as `***`). This is a private, single-operator
/// PostHog Cloud project; the raw text is the deliverable. The only guardrail kept is the size cap,
/// because PostHog HARD-DROPS events over 1MB (that's data loss, not privacy). Byte cap (not char)
/// so multibyte content cannot silently blow past the limit.
fn mask_and_cap_content(s: &str) -> String {
    // 60KB keeps each field under PostHog's per-string-property limit and the event comfortably
    // under 1MB (input + output + the rest of the event).
    const AI_CONTENT_MAX_BYTES: usize = 60 * 1024;
    if s.len() <= AI_CONTENT_MAX_BYTES {
        return s.to_string();
    }
    // Truncate on a char boundary so we never split a UTF-8 sequence.
    let mut end = AI_CONTENT_MAX_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push_str("...<truncated>");
    out
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

    /// A subscription call costs exactly nothing at the margin. Not a placeholder, the truth.
    #[test]
    fn subscription_agents_cost_zero_not_list_price() {
        for agent in ["codex", "claude", "gemini"] {
            let (usd, billing) = cost_usd(agent, None, 86_528, 103_402, Some(166));
            assert_eq!(billing, "subscription", "{agent}");
            assert_eq!(usd, Some(0.0), "{agent} must not be charged list price");
        }
    }

    /// The resolver hands back an absolute toplevel path. Shipping it raw would be
    /// cardinality garbage AND would leak the operator's home directory to a SaaS.
    #[test]
    fn repo_name_reduces_absolute_paths_to_a_bounded_label() {
        assert_eq!(repo_name("/Users/someone/projects/triumvirate"), "triumvirate");
        assert_eq!(repo_name("/Users/someone/projects/triumvirate/"), "triumvirate");
        // Already bare: unchanged.
        assert_eq!(repo_name("triumvirate"), "triumvirate");
        // Degenerate input must never panic or invent a value.
        assert_eq!(repo_name("/"), "/");
        assert_eq!(repo_name(""), "");
    }

    /// Rung 1 sanitization is the privacy/security floor for $mcp_parameters. It must mask
    /// live credentials (a key in the first 160 chars would otherwise ship to a SaaS), basename
    /// absolute paths, and truncate long free-text, while preserving structure.
    #[test]
    fn mcp_sanitizer_is_raw_and_only_size_caps() {
        // Nothing is masked: the literal string passes through verbatim, only size-capped
        // (operator's explicit choice — everything raw and unmasked).
        assert_eq!(
            mcp_preview("use key phc_abcdef0123456789 now"),
            "use key phc_abcdef0123456789 now",
            "content is raw, unmasked"
        );
        // Absolute paths are kept verbatim (no basenaming anymore).
        assert_eq!(
            mcp_preview("/Users/mike/projects/triumvirate/daemon"),
            "/Users/mike/projects/triumvirate/daemon"
        );
        // Normal-length content passes whole; only a pathological blob is capped.
        let normal = "x".repeat(500);
        assert_eq!(mcp_preview(&normal), normal, "normal content is not truncated");
        let huge = "x".repeat(MCP_PREVIEW_MAX_CHARS + 1000);
        let capped = mcp_preview(&huge);
        assert!(capped.chars().count() <= MCP_PREVIEW_MAX_CHARS + 3, "pathological blob capped");
        assert!(capped.ends_with("..."), "cap marked");

        // Structure AND values preserved verbatim; only structural limiting applies.
        let v = serde_json::json!({"agent":"gemini","message":"/etc/passwd","n":3});
        let s = sanitize_mcp_json(&v);
        assert_eq!(s["agent"], serde_json::json!("gemini"));
        assert_eq!(s["message"], serde_json::json!("/etc/passwd"), "path kept raw");
        assert_eq!(s["n"], serde_json::json!(3));

        // Sensitive-NAMED keys are NO LONGER redacted: values kept raw.
        let sensitive = serde_json::json!({
            "password": "hunter2",
            "authorization": "Bearer opaque",
            "api_key": "AIzaKey",
        });
        let r = sanitize_mcp_json(&sensitive);
        assert_eq!(r["password"], serde_json::json!("hunter2"));
        assert_eq!(r["authorization"], serde_json::json!("Bearer opaque"));
        assert_eq!(r["api_key"], serde_json::json!("AIzaKey"));

        // Content is captured as PostHog's structured LLM shape, not flat strings, or the trace UI
        // renders nothing (Antigravity). $ai_input is a message array, completion is
        // $ai_output_choices; both RAW (no scrubbing).
        let g = AiGeneration {
            agent: "gemini",
            model: None,
            outcome: "success",
            trace_id: "t1",
            input_tokens: Some(1),
            output_tokens: Some(1),
            cached_tokens: None,
            thinking_tokens: None,
            tool_calls: None,
            duration_ms: 10,
            cost_usd: None,
            billing: "subscription",
            backend: None,
            attempts: 1,
            repo: None,
            input: Some("my api_key: sk-livedeadbeef0123456789"),
            output: Some("done"),
        };
        let ev = ai_generation_props(&g);
        assert_eq!(ev["$ai_input"][0]["role"], serde_json::json!("user"));
        // Content is captured RAW (operator's explicit choice): the literal text of the call, no
        // masking. Only the size cap remains. See mask_and_cap_content.
        assert_eq!(ev["$ai_input"][0]["content"],
                   serde_json::json!("my api_key: sk-livedeadbeef0123456789"),
                   "captured input must be the literal text, unscrubbed: {}", ev["$ai_input"]);
        assert_eq!(ev["$ai_output_choices"][0]["role"], serde_json::json!("assistant"));
        assert_eq!(ev["$ai_output_choices"][0]["content"], serde_json::json!("done"));
        assert!(ev.get("$ai_output").is_none(), "flat $ai_output must not be emitted");
    }

    /// An unknown model must emit NO cost. A wrong number is worse than a missing one.
    #[test]
    fn unknown_model_emits_no_cost() {
        let (usd, billing) = cost_usd("deepseek", Some("deepseek-v9-unreleased"), 10, 10, Some(10));
        assert_eq!(billing, "unknown");
        assert_eq!(usd, None);
    }

    /// Regression for the three DeepSeek dispatches that reached PostHog as `tv_outcome =
    /// "unreported"` (180s / 68s errors, model=unknown, one primary attempt, metered). Their
    /// futures were cancelled by the caller's 180s `ask_agent` ceiling before any classify()
    /// arm ran, so they fell outside outcome-based dispatch monitoring. Once a dispatch is
    /// in-flight, an unrecorded drop must resolve to the terminal `"cancelled"`, not the
    /// sentinel.
    #[test]
    fn cancelled_deepseek_dispatch_emits_terminal_outcome_not_unreported() {
        // A metered DeepSeek call with no model resolved yet — exactly the shape observed:
        // agent=deepseek, $ai_model=unknown. We reproduce the cancellation by arming the
        // dispatch and then dropping without ever calling success/degraded_success/failure.
        let mut tel = CallTelemetry::new("deepseek", "trace-cancelled", None);
        tel.record_attempt();
        tel.begin_dispatch();
        assert_eq!(
            tel.effective_outcome(),
            "cancelled",
            "an in-flight drop with no recorded outcome is a cancellation, not `unreported`"
        );

        // And a cancellation IS an error, so it lands in error/outcome-based monitoring.
        let g = AiGeneration {
            agent: "deepseek",
            model: None,
            outcome: tel.effective_outcome(),
            trace_id: "trace-cancelled",
            input_tokens: None,
            output_tokens: None,
            cached_tokens: None,
            thinking_tokens: None,
            tool_calls: None,
            duration_ms: 180_001,
            cost_usd: None,
            billing: "metered",
            backend: None,
            attempts: 1,
            repo: None,
            input: None,
            output: None,
        };
        let ev = ai_generation_props(&g);
        assert_eq!(ev["tv_outcome"], serde_json::json!("cancelled"));
        assert_eq!(ev["$ai_is_error"], serde_json::json!(true));
        assert_eq!(ev["$ai_model"], serde_json::json!("unknown"));
    }

    /// An explicitly recorded outcome is never overridden by the cancellation promotion:
    /// a DeepSeek call that fails typed (timeout, hard provider, ...) still reads `"failure"`.
    #[test]
    fn recorded_failure_wins_over_cancellation_promotion() {
        let mut tel = CallTelemetry::new("deepseek", "trace-failed", None);
        tel.begin_dispatch();
        tel.failure("deepseek absolute SLA timeout exceeded");
        assert_eq!(tel.effective_outcome(), "failure");
    }

    /// The `unreported` canary is preserved: a drop that never reached `begin_dispatch` (a
    /// synchronous exit that forgot to classify itself) must still surface as `unreported` so
    /// the dashboard tells on a genuinely new unclassified code path — the reason the sentinel
    /// exists at all.
    #[test]
    fn synchronous_unclassified_exit_stays_unreported_canary() {
        let tel = CallTelemetry::new("deepseek", "trace-sync", None);
        assert_eq!(tel.effective_outcome(), "unreported");
    }
}
