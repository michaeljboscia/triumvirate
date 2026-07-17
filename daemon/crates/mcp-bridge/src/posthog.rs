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
//!   POSTHOG_HOST     e.g. https://posthog.e5btools.com   (unset -> this module is a no-op)
//!   POSTHOG_API_KEY  e.g. phc_...                        (unset -> no-op)
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
        }
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
            backend: self.backend,
            attempts: self.attempts,
            repo: self.repo.as_deref(),
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
fn capture(event: &str, properties: serde_json::Value) {
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
        "distinct_id": "triumvirate-daemon",
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
///   completed  - exited 0 AND produced a commit
///   no_commit  - exited 0 and produced NOTHING. The silent one. Codex reported success and
///                the repo is unchanged; without this it is indistinguishable from a real
///                success on every surface except a human going to look for the commit.
///   failed     - non-zero exit
///   timeout    - killed at the deadline
///   wait_error - we lost track of the child
///
/// No task_id and no prompt: both unbounded. This event is for counting and grouping ("how
/// often does codex silently produce nothing, and in which repo?"); the local tracker stays
/// the place to inspect one dispatch.
/// Emits exactly one `tv_codex_dispatch` on Drop, whichever way the monitor task exits.
///
/// Same reasoning as `CallTelemetry`, and for the same reason: `dispatch_codex_worktree`'s
/// monitor has EIGHT terminal arms (timeout, sentinel commit, head commit, wait error, two
/// no-commit paths, completed, failed). Hand-placing an emit in each is the pattern this
/// module's own history says fails — reviewers found three holes in exactly that approach on
/// the ask path. A guard cannot forget an arm, and `"unreported"` makes a new, unclassified
/// exit visible in the dashboard instead of silently absent.
pub struct CodexDispatchTelemetry {
    surface: &'static str,
    started: std::time::Instant,
    outcome: &'static str,
    repo: Option<String>,
    files_changed: Option<usize>,
    exit_code: Option<i32>,
}

impl CodexDispatchTelemetry {
    pub fn new(surface: &'static str, repo: Option<&str>) -> Self {
        Self::new_started_at(surface, repo, std::time::Instant::now())
    }

    /// Build with a caller-supplied origin, for monitors that begin observing AFTER the work
    /// started. ABE dispatches the child, then a separate task waits on it; a guard built in
    /// that task with `Instant::now()` would report the classification time, not the
    /// dispatch. Pass the dispatch's own start so the duration means what it says.
    pub fn new_started_at(
        surface: &'static str,
        repo: Option<&str>,
        started: std::time::Instant,
    ) -> Self {
        Self {
            surface,
            started,
            outcome: "unreported",
            repo: repo.map(|r| r.to_string()),
            files_changed: None,
            exit_code: None,
        }
    }

    /// Exited 0 AND produced a commit.
    pub fn completed(&mut self, files_changed: usize, exit_code: Option<i32>) {
        self.outcome = "completed";
        self.files_changed = Some(files_changed);
        self.exit_code = exit_code;
    }

    /// Exited 0 and changed NOTHING. The silent one.
    pub fn no_commit(&mut self, exit_code: Option<i32>) {
        self.outcome = "no_commit";
        self.files_changed = Some(0);
        self.exit_code = exit_code;
    }

    pub fn failed(&mut self, exit_code: Option<i32>) {
        self.outcome = "failed";
        self.exit_code = exit_code;
    }

    pub fn timeout(&mut self) {
        self.outcome = "timeout";
    }

    pub fn wait_error(&mut self) {
        self.outcome = "wait_error";
    }
}

impl Drop for CodexDispatchTelemetry {
    fn drop(&mut self) {
        record_codex_dispatch(
            self.surface,
            self.outcome,
            self.started.elapsed().as_millis() as u64,
            self.repo.as_deref(),
            self.files_changed,
            self.exit_code,
        );
    }
}

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

pub fn record_ai_generation(g: &AiGeneration<'_>) {
    let is_error = g.outcome != "success" && g.outcome != "degraded_success";

    capture(
        "$ai_generation",
        json!({
            // --- PostHog's LLM analytics schema ---
            "$ai_trace_id":       g.trace_id,
            "$ai_provider":       provider_for(g.agent),
            // CLI agents report no model id, so this falls back to the agent. It must fall
            // back to the DISPLAY name ("Antigravity"), never the internal key ("gemini"):
            // the key is a routing detail, and Gemini the product is retired. An operator
            // reading a chart should never see a name we no longer call anything.
            "$ai_model":          g.model.map(str::to_string)
                                    .unwrap_or_else(|| crate::display_agent_name(g.agent)),
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

    /// An unknown model must emit NO cost. A wrong number is worse than a missing one.
    #[test]
    fn unknown_model_emits_no_cost() {
        let (usd, billing) = cost_usd("deepseek", Some("deepseek-v9-unreleased"), 10, 10, Some(10));
        assert_eq!(billing, "unknown");
        assert_eq!(usd, None);
    }
}
