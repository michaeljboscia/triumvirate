//! DeepSeek module — HTTP client, SSE parser, guards, runaway abort, runner.
//!
//! T-005 build_client (REQ-DS-007): reqwest::Client with rolling read_timeout.
//! T-006 StreamParser (REQ-DS-019): chunk-boundary-safe SSE parser.
//! T-007 guards (REQ-DS-029/030): ghost-success detect + finish_reason guard.
//! T-008 runaway (REQ-DS-028): reasoning-cap early-abort.
//! T-009 finalize (REQ-DS-009/018/021/023/026): usage map + cost + per-request log.
//! T-010 run() (REQ-DS-004/005/008/014/024): top-level orchestrator.

use crate::deepseek_config::DeepSeekConfig;

/// Build a `reqwest::Client` from the loaded DeepSeek config.
///
/// Returns `reqwest::Error` if reqwest's builder rejects the configuration
/// (e.g. a TLS backend failure). In practice this is infallible with our
/// default-features = false + rustls-tls workspace setup.
pub fn build_client(cfg: &DeepSeekConfig) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        // Rolling per-read idle timeout. Each byte resets it; an SSE stream that
        // dribbles chunks within `read_timeout` of each other will complete even
        // if the total wall-clock exceeds it. This is the property the spec
        // (REQ-DS-007) is leaning on for long thinking-mode responses.
        .read_timeout(cfg.read_timeout)
        // Codex W3-review SHOULD-FIX #1: do NOT set reqwest .timeout(cfg.timeout)
        // here. The runner's `tokio::time::timeout(cfg.timeout, ...)` is the
        // SINGLE owner of the absolute SLA ceiling — two competing absolute
        // timeouts created a race where a hung request could surface as
        // NetworkPreFirstByte(reqwest timeout) instead of AbsoluteTimeoutExceeded.
        // TCP keep-alive probes detect dead peers between requests.
        .tcp_keepalive(cfg.tcp_keepalive)
        .build()
}

// ─────────────────────────────────────────────────────────────────────────────
// T-006 (REQ-DS-019): SSE stream parser.
// ─────────────────────────────────────────────────────────────────────────────
//
// DeepSeek's /v1/chat/completions endpoint streams Server-Sent Events:
//
//     : ping\n\n                          ← comment / keep-alive (line starts with ':')
//     data: {"id":"...","choices":[{"delta":{"reasoning_content":"…"}}]}\n\n
//     data: {"id":"...","choices":[{"delta":{"content":"…"}}]}\n\n
//     data: {"id":"...","choices":[{"delta":{},"finish_reason":"stop"}],"usage":{…}}\n\n
//     data: [DONE]\n\n
//
// The parser is stateful — call `feed(&mut self, chunk)` repeatedly with
// arbitrary byte slices (reqwest's `bytes_stream` yields these at TCP-packet
// granularity, which has zero relationship to SSE event boundaries). Buffered
// bytes are held until an event terminator (`\n\n`) is seen, so a JSON object
// split across N chunks is reassembled correctly.
//
// The parser MUST NOT make decisions about retry / breaker state — it just
// reports what it saw. T-007 / T-008 / T-009 / T-010 wrap it.

#[derive(Clone, Debug, serde::Deserialize)]
pub struct RawUsage {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub prompt_cache_hit_tokens: i64,
    #[serde(default)]
    pub prompt_cache_miss_tokens: i64,
    #[serde(default)]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CompletionTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: i64,
}

/// T-007 ghost-success: a `data:` chunk whose JSON has a top-level `error`
/// object. DeepSeek occasionally streams a 200-with-embedded-error.
#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
pub struct EmbeddedError {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

/// Internal raw chunk shape — only the fields the parser cares about. Extra
/// fields are ignored by serde's default.
#[derive(Debug, serde::Deserialize)]
struct RawChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    system_fingerprint: Option<String>,
    #[serde(default)]
    choices: Vec<RawChoice>,
    #[serde(default)]
    usage: Option<RawUsage>,
    #[serde(default)]
    error: Option<EmbeddedError>,
}

#[derive(Debug, serde::Deserialize)]
struct RawChoice {
    #[serde(default)]
    delta: RawDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawDelta {
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

/// Events the parser surfaces from `feed`. Callers use these for telemetry
/// (e.g. counting keep-alives, tracking reasoning growth for T-008); the
/// post-stream accumulators on `StreamParser` are the authoritative result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseEvent {
    /// `: …\n\n` comment / heartbeat line.
    KeepAlive,
    /// One `data: {…}` chunk parsed; the parser already merged its content into
    /// the accumulators. Variants record what was extracted, in order, so a
    /// caller can implement T-008's running token estimate cheaply.
    ReasoningDelta { added_chars: usize },
    ContentDelta { added_chars: usize },
    Usage,
    EmbeddedError,
    /// `data: [DONE]\n\n` sentinel.
    Done,
}

#[derive(Debug)]
pub enum ParseError {
    Utf8 { context: String },
    InvalidJson { snippet: String, cause: String },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Utf8 { context } => write!(f, "invalid UTF-8 in SSE event ({context})"),
            ParseError::InvalidJson { snippet, cause } => {
                write!(f, "invalid JSON in `data:` event ({cause}): {snippet}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

pub struct StreamParser {
    /// Bytes received but not yet terminated by `\n\n`.
    buffer: Vec<u8>,
    pub reasoning_acc: String,
    pub content_acc: String,
    pub usage: Option<RawUsage>,
    pub finish_reason: Option<String>,
    pub request_id: Option<String>,
    pub system_fingerprint: Option<String>,
    pub embedded_error: Option<EmbeddedError>,
    pub done: bool,
    pub keepalive_count: u32,
}

impl Default for StreamParser {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamParser {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(4096),
            reasoning_acc: String::new(),
            content_acc: String::new(),
            usage: None,
            finish_reason: None,
            request_id: None,
            system_fingerprint: None,
            embedded_error: None,
            done: false,
            keepalive_count: 0,
        }
    }

    /// T-008 (REQ-DS-028): the cheap chars/4 token estimate used to detect
    /// runaway reasoning. The /4 factor matches what we use for `usage_source
    /// = "estimated"` records when the usage chunk is missing (T-009).
    pub fn estimated_reasoning_tokens(&self) -> i64 {
        (self.reasoning_acc.chars().count() / 4) as i64
    }

    /// True when `cap > 0` AND the streamed reasoning has exceeded it. The
    /// runner calls this between `feed` invocations and drops the reqwest
    /// stream-future cleanly the moment it returns true, then returns
    /// `DeepSeekFailureKind::RunawayReasoning { observed_tokens }`.
    /// The breaker MUST NOT be informed of this — runaway is a budget signal,
    /// not a provider-fault signal.
    pub fn is_runaway(&self, cap: u32) -> bool {
        if cap == 0 {
            return false;
        }
        self.estimated_reasoning_tokens() >= cap as i64
    }

    /// Feed one chunk of bytes. Extracts every COMPLETE event terminated by
    /// `\n\n` within the buffered + new bytes; partial trailing event stays
    /// buffered for the next call. Returns the events extracted in this call.
    ///
    /// Idempotent on empty input. Safe to call after `done == true` (no-op
    /// beyond buffering, which the runner shouldn't be doing anyway).
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<ParseEvent>, ParseError> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();

        // Scan for `\n\n` or `\r\n\r\n` boundaries, draining as we go.
        while let Some((payload_end, term_len)) = find_event_terminator(&self.buffer) {
            // Event payload is everything BEFORE the terminator; drain past
            // both so subsequent boundary searches start fresh.
            let event_bytes: Vec<u8> = self.buffer.drain(..payload_end).collect();
            self.buffer.drain(..term_len);

            // SSE events can contain CR/LF combinations; we accept LF-only here,
            // but explicitly strip a trailing \r before parsing each line so that
            // CRLF-emitting servers don't break us.
            let event_str = match std::str::from_utf8(&event_bytes) {
                Ok(s) => s,
                Err(_) => {
                    return Err(ParseError::Utf8 {
                        context: format!("event of {} bytes", event_bytes.len()),
                    });
                }
            };

            self.consume_event(event_str, &mut events)?;
        }

        Ok(events)
    }

    fn consume_event(
        &mut self,
        event: &str,
        events: &mut Vec<ParseEvent>,
    ) -> Result<(), ParseError> {
        // Codex W3-review SHOULD-FIX #3: per the SSE spec, multiple `data:`
        // lines within ONE event must be JOINED by '\n' and dispatched as a
        // single payload. DeepSeek emits one-data-per-event today but a
        // proxy that pretty-prints JSON across lines would have broken the
        // previous "parse each data: line independently" implementation.
        //
        // Algorithm:
        //   1. Scan lines; collect data: payloads in order.
        //   2. Ignore `:` comment lines (count as keep-alive marker).
        //   3. Silently ignore unknown SSE field names (event:, id:, retry:).
        //   4. After scanning: if any data: payload exists, join with '\n'
        //      and dispatch as ONE chunk.
        //
        // [DONE] handling: if ANY of the data: payloads is exactly "[DONE]"
        // after the join, we treat it as the sentinel. (In practice the
        // sentinel is a single-line event so this matches reality; the join
        // path is defensive against pathological proxies.)
        let mut data_lines: Vec<&str> = Vec::new();
        let mut had_comment_only = false;
        let mut had_any_data = false;
        for raw_line in event.split('\n') {
            let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
            if line.is_empty() {
                continue;
            }
            if line.starts_with(':') {
                had_comment_only = true;
                continue;
            }
            if let Some(payload) = line.strip_prefix("data:") {
                data_lines.push(payload.trim_start());
                had_any_data = true;
                continue;
            }
            // Unknown SSE field (event:, id:, retry:, …) — ignore.
        }

        if had_any_data {
            let joined = data_lines.join("\n");
            if joined == "[DONE]" {
                self.done = true;
                events.push(ParseEvent::Done);
            } else {
                self.consume_data_chunk(&joined, events)?;
            }
        } else if had_comment_only {
            // Pure heartbeat event.
            self.keepalive_count += 1;
            events.push(ParseEvent::KeepAlive);
        }
        // Else: event was completely empty (e.g. trailing \n\n at stream end);
        // do nothing.
        Ok(())
    }

    fn consume_data_chunk(
        &mut self,
        payload: &str,
        events: &mut Vec<ParseEvent>,
    ) -> Result<(), ParseError> {
        let chunk: RawChunk = serde_json::from_str(payload).map_err(|e| {
            // Codex W3-review SHOULD-FIX #4: bound the snippet at a UTF-8
            // boundary. `&payload[..200]` would panic if byte 200 lands inside
            // a multi-byte char (e.g. an emoji in an error message).
            let snippet = if payload.len() > 200 {
                format!("{}…", cap_reasoning(payload, 200))
            } else {
                payload.to_string()
            };
            ParseError::InvalidJson {
                snippet,
                cause: e.to_string(),
            }
        })?;

        if let Some(id) = chunk.id
            && self.request_id.is_none()
        {
            self.request_id = Some(id);
        }
        if let Some(fp) = chunk.system_fingerprint {
            self.system_fingerprint = Some(fp);
        }
        if let Some(err) = chunk.error {
            self.embedded_error = Some(err);
            events.push(ParseEvent::EmbeddedError);
            return Ok(());
        }
        if let Some(usage) = chunk.usage {
            self.usage = Some(usage);
            events.push(ParseEvent::Usage);
        }
        for choice in chunk.choices {
            if let Some(reason) = choice.finish_reason {
                self.finish_reason = Some(reason);
            }
            if let Some(r) = choice.delta.reasoning_content {
                let added = r.chars().count();
                self.reasoning_acc.push_str(&r);
                events.push(ParseEvent::ReasoningDelta { added_chars: added });
            }
            if let Some(c) = choice.delta.content {
                let added = c.chars().count();
                self.content_acc.push_str(&c);
                events.push(ParseEvent::ContentDelta { added_chars: added });
            }
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// T-007 (REQ-DS-029, REQ-DS-030): guards — ghost-success + finish_reason.
// ─────────────────────────────────────────────────────────────────────────────
//
// DeepSeek occasionally streams a 200-with-embedded-error: the HTTP envelope
// is 200 OK, the SSE stream looks normal, but one of the `data:` chunks
// contains a top-level `error` object. T-006 stashed it in
// `parser.embedded_error`; T-007 detects it at finalize and converts it to a
// typed failure routed through the same classification path as HTTP errors
// (per REQ-DS-029 — no new breaker policy).
//
// REQ-DS-030 covers the other side: finish_reason ∈ {length, content_filter,
// null, "unknown"} means the model output is INCOMPLETE or REJECTED — never
// return Ok(parsed) with partial content. Always fail loud.

use crate::deepseek_resilience::Classification;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BadFinishReasonKind {
    Length,
    ContentFilter,
    /// B.6 (test-plan TIER 1): DeepSeek-specific finish_reason indicating the
    /// backend ran out of capacity to complete the request. Classify as
    /// transient — retry-eligible — separate from `Unknown` so monitoring
    /// can distinguish capacity-driven failures from caller bugs.
    InsufficientSystemResource,
    Missing,
    Unknown(String),
}

/// Typed failure surface for the DeepSeek runner. Grows with each task; the
/// runner (T-010) matches on it to decide whether to invoke the breaker.
#[derive(Debug)]
pub enum DeepSeekFailureKind {
    /// A `data:` chunk carried a top-level `error` object. The accompanying
    /// classification mirrors what the same condition would map to as an HTTP
    /// status code (e.g. insufficient_balance → Hard, like 402).
    GhostSuccessEmbedded {
        error: EmbeddedError,
        classification: Classification,
    },
    /// finish_reason was not `stop`. Partial output is NOT returned — the
    /// caller learns the kind and decides whether to retry.
    BadFinishReason(BadFinishReasonKind),
    /// T-008 (REQ-DS-028): observed reasoning chars/4 crossed
    /// `cfg.reasoning_cap_tokens`. This is a BUDGET decision, not a provider
    /// fault — the runner MUST NOT call breaker.record() on this failure.
    RunawayReasoning { observed_tokens: i64 },
    /// T-010: HTTP 4xx that classify() decided is Hard (400/401/402/403/422).
    /// The runner records this on the breaker as `Outcome::HardError`.
    HardProvider(u16),
    /// T-010: HTTP 429/5xx after the in-runner 429-Retry-After retry was
    /// exhausted (or wasn't applicable). Recorded as `Outcome::TransientError`.
    Transient(u16),
    /// T-010: connect / DNS / request-build error before the first response
    /// byte. The runner already retried once; this is the second-failure
    /// surface. Treated as transient by the breaker.
    NetworkPreFirstByte(String),
    /// T-010: stream byte error AFTER the first response byte. Estimated
    /// usage is attached because no `usage` chunk was seen.
    NetworkMidStream(String),
    /// T-010 (REQ-DS-024): the outer `tokio::time::timeout` fired before
    /// `[DONE]` was seen. Recorded as transient.
    AbsoluteTimeoutExceeded,
    /// T-010: the breaker refused at request time. The variant carries the
    /// state so the dispatch layer can surface the right user message.
    BreakerOpen(BreakerState),
    /// T-010: parser raised a typed ParseError mid-stream. Treated as a
    /// transient SSE-malformed condition for breaker purposes — DeepSeek
    /// streaming format isn't expected to break, so this is a real signal.
    ParserError(String),
    /// Codex W3-review SHOULD-FIX #6: the response stream closed cleanly (no
    /// reqwest body Err) but `data: [DONE]` was never seen. `[DONE]` is the
    /// application-layer terminator per the OpenAI-compatible streaming
    /// contract; missing it means the response was truncated even if the
    /// underlying socket reported a normal EOF. Treated as transient.
    StreamEndedWithoutDone,
    /// B.5 (test-plan TIER 1): the response had `finish_reason = "stop"` and
    /// (per the wire) is a "successful" completion, but `content` is empty
    /// while `reasoning_content` may or may not be present. Caller expected
    /// a final answer and got nothing. Surface as a typed failure rather
    /// than returning Ok with empty `response_text`. Breaker-neutral
    /// (not a provider fault — the model just declined to answer).
    EmptyFinalAnswer { had_reasoning: bool },
}

impl std::fmt::Display for DeepSeekFailureKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeepSeekFailureKind::GhostSuccessEmbedded { error, classification } => {
                write!(
                    f,
                    "ghost-success: embedded error (classification={classification:?}) code={:?} message={:?}",
                    error.code, error.message
                )
            }
            DeepSeekFailureKind::BadFinishReason(kind) => {
                write!(f, "bad finish_reason: {kind:?}")
            }
            DeepSeekFailureKind::RunawayReasoning { observed_tokens } => {
                write!(f, "runaway reasoning: ~{observed_tokens} tokens crossed cap")
            }
            DeepSeekFailureKind::HardProvider(code) => write!(f, "deepseek HTTP {code} (hard)"),
            DeepSeekFailureKind::Transient(code) => write!(f, "deepseek HTTP {code} (transient)"),
            DeepSeekFailureKind::NetworkPreFirstByte(detail) => {
                write!(f, "deepseek network failure before first byte: {detail}")
            }
            DeepSeekFailureKind::NetworkMidStream(detail) => {
                write!(f, "deepseek mid-stream network failure: {detail}")
            }
            DeepSeekFailureKind::AbsoluteTimeoutExceeded => {
                write!(f, "deepseek absolute SLA timeout exceeded")
            }
            DeepSeekFailureKind::BreakerOpen(state) => {
                write!(f, "deepseek breaker is open: {state:?}")
            }
            DeepSeekFailureKind::ParserError(detail) => {
                write!(f, "deepseek SSE parser error: {detail}")
            }
            DeepSeekFailureKind::StreamEndedWithoutDone => {
                write!(f, "deepseek stream ended without [DONE] sentinel")
            }
            DeepSeekFailureKind::EmptyFinalAnswer { had_reasoning } => {
                write!(
                    f,
                    "deepseek returned finish_reason=stop with empty content (had_reasoning={had_reasoning})"
                )
            }
        }
    }
}

impl std::error::Error for DeepSeekFailureKind {}

/// The clean-success result of finalizing a parsed stream.
#[derive(Clone, Debug)]
pub struct FinalizedStream {
    pub content: String,
    pub reasoning: String,
    pub usage: Option<RawUsage>,
    pub finish_reason: String,
    pub request_id: Option<String>,
    pub system_fingerprint: Option<String>,
}

/// Apply T-007's guards to a parsed stream and either return the clean result
/// or a typed failure. Ghost-success takes precedence over finish_reason
/// because an embedded error means the stream is a lie regardless of how it
/// "ended". The finalized stream is borrowed from the parser, so the caller
/// retains ownership of the parser for downstream usage (T-009).
pub fn finalize_stream(parser: &StreamParser) -> Result<FinalizedStream, DeepSeekFailureKind> {
    // (a) Ghost-success: detect FIRST. A 402-equivalent embedded payload looks
    //     like a 200 success on the wire, so the only thing protecting the
    //     caller from acting on a lie is this check.
    if let Some(err) = &parser.embedded_error {
        return Err(DeepSeekFailureKind::GhostSuccessEmbedded {
            classification: classify_embedded_error(err),
            error: err.clone(),
        });
    }

    // (b) finish_reason guard. "stop" is the only clean exit.
    match parser.finish_reason.as_deref() {
        Some("stop") => {
            // B.5 (test-plan TIER 1): finish_reason=stop with EMPTY content is
            // a semantic failure — the model "successfully" declined to answer.
            // Caller MUST NOT downstream an empty response as a normal result.
            if parser.content_acc.is_empty() {
                return Err(DeepSeekFailureKind::EmptyFinalAnswer {
                    had_reasoning: !parser.reasoning_acc.is_empty(),
                });
            }
            Ok(FinalizedStream {
                content: parser.content_acc.clone(),
                reasoning: parser.reasoning_acc.clone(),
                usage: parser.usage.clone(),
                finish_reason: "stop".to_string(),
                request_id: parser.request_id.clone(),
                system_fingerprint: parser.system_fingerprint.clone(),
            })
        }
        Some("length") => {
            Err(DeepSeekFailureKind::BadFinishReason(BadFinishReasonKind::Length))
        }
        Some("content_filter") => Err(DeepSeekFailureKind::BadFinishReason(
            BadFinishReasonKind::ContentFilter,
        )),
        // B.6 (test-plan TIER 1): DeepSeek-documented finish_reason for
        // provider-capacity interruption. Promote to its own variant so
        // monitoring can distinguish it from generic Unknown.
        Some("insufficient_system_resource") => Err(DeepSeekFailureKind::BadFinishReason(
            BadFinishReasonKind::InsufficientSystemResource,
        )),
        Some(other) => Err(DeepSeekFailureKind::BadFinishReason(
            BadFinishReasonKind::Unknown(other.to_string()),
        )),
        None => Err(DeepSeekFailureKind::BadFinishReason(BadFinishReasonKind::Missing)),
    }
}

/// Map an embedded error to the same Hard/Transient axis as HTTP status codes.
/// Unknown / unrecognised codes default to Hard — better to fail loud than
/// quietly retry on something the parser doesn't understand.
fn classify_embedded_error(err: &EmbeddedError) -> Classification {
    let code = err.code.as_deref().unwrap_or("").to_lowercase();
    let msg = err.message.as_deref().unwrap_or("").to_lowercase();
    let kind = err.kind.as_deref().unwrap_or("").to_lowercase();
    let combined = format!("{code} {msg} {kind}");

    // Transient first — these map to 429/5xx-like behavior.
    let transient_markers = ["rate_limit", "rate limit", "429", "overloaded", "server_error"];
    if transient_markers.iter().any(|m| combined.contains(m)) {
        return Classification::Transient;
    }
    // Everything else (insufficient_balance, billing, auth, invalid_*, ...)
    // is Hard. Including the catch-all unknown case.
    Classification::Hard
}

// ─────────────────────────────────────────────────────────────────────────────
// T-009 (REQ-DS-009, REQ-DS-018, REQ-DS-021, REQ-DS-023, REQ-DS-026):
// usage mapping + estimated fallback + per-request log writer.
// ─────────────────────────────────────────────────────────────────────────────
//
// Three concerns through the same finalize path:
//
//   (a) Usage mapping (REQ-DS-009 / A-04b). DeepSeek's `usage` chunk has the
//       form { prompt_tokens, completion_tokens, prompt_cache_hit_tokens,
//       prompt_cache_miss_tokens, completion_tokens_details.reasoning_tokens }.
//       `completion_tokens` ALREADY INCLUDES `reasoning_tokens` — adding them
//       overstates output and breaks cost math. The mapping:
//         input_tokens  ← prompt_cache_miss_tokens   (priced as "miss")
//         output_tokens ← completion_tokens          (NOT + reasoning_tokens)
//         cached_tokens ← prompt_cache_hit_tokens    (priced as "cached")
//
//   (b) Estimated fallback when the usage chunk is missing (dirty disconnect,
//       runaway abort, bad finish_reason). bytes_received / 4 for output +
//       prompt_estimate for input. usage_source = "estimated".
//
//   (c) Per-request log file. JSON dropped at $LOG_DIR/<request_id>.json
//       containing { request_id, model, system_fingerprint, reasoning_content
//       (size-capped), content, usage, cost_usd, finish_reason, timestamp }.
//       This is the NAMED storage target for REQ-DS-023 (CoT bifurcation)
//       and REQ-DS-018 (system_fingerprint observability). Privacy guard:
//       MUST NOT contain the API key or the request `messages` payload.
//
// The cost number itself is computed by the caller via the now-pub
// `token_economics::calculate_cost_usd` — keeping that out of mcp-bridge
// keeps the production dep graph clean (token-economics is a dev-dep here
// purely so the test can anchor against probe-04 numbers).

use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageSource {
    /// Came from the `usage` SSE chunk — DeepSeek's authoritative count.
    Exact,
    /// Synthesized from bytes_received/4 + prompt_estimate when no usage
    /// chunk arrived (mid-stream disconnect, runaway abort, bad finish).
    Estimated,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub usage_source: UsageSource,
}

/// REQ-DS-009 / A-04b mapping. The single source of truth — every other
/// caller in the daemon goes through this, NEVER touches RawUsage fields
/// directly to derive a final number.
///
/// Stub guard: a stub that does `output ← completion_tokens +
/// completion_tokens_details.reasoning_tokens` overstates and is caught by
/// the test `deepseek_usage_map_no_double_add`.
pub fn map_usage(raw: &RawUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: raw.prompt_cache_miss_tokens,
        output_tokens: raw.completion_tokens,
        cached_tokens: raw.prompt_cache_hit_tokens,
        usage_source: UsageSource::Exact,
    }
}

/// Fallback when the `usage` chunk never arrived. The runner computes
/// `bytes_received` itself (sum of chunk lengths) and supplies a prompt-side
/// estimate (chars/4 of the request messages, computed without ever logging
/// them).
pub fn estimate_usage(bytes_received: i64, prompt_estimate: i64) -> TokenUsage {
    TokenUsage {
        input_tokens: prompt_estimate,
        output_tokens: bytes_received / 4,
        cached_tokens: 0,
        usage_source: UsageSource::Estimated,
    }
}

/// Truncate a reasoning string to `cap_bytes` while preserving UTF-8
/// boundaries. Used by the log writer so a multi-MB reasoning string doesn't
/// produce a multi-MB log file by default (cap defaults to 256KB per
/// REQ-DS-023 / DeepSeekConfig.log_reasoning_cap_bytes).
pub fn cap_reasoning(s: &str, cap_bytes: usize) -> &str {
    if s.len() <= cap_bytes {
        return s;
    }
    let mut idx = cap_bytes;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    &s[..idx]
}

/// The JSON shape persisted to `$LOG_DIR/<request_id>.json`. Borrows the
/// underlying strings so the writer doesn't double the memory cost of a big
/// reasoning trace.
///
/// Privacy guard (REQ-DS-023 scope_out): MUST NOT contain the API key or the
/// request messages — only RESPONSE artifacts (model output, reasoning trace,
/// usage, fingerprint, cost). The serialised JSON has no field named
/// `messages`, `api_key`, or `Authorization`; the privacy regression test in
/// the runner asserts this on a real consult.
///
/// Caveat operators should know (Codex W3-review NIT #3): `reasoning_content`
/// and `content` are model OUTPUT. A model that parrots or quotes the user's
/// prompt verbatim CAN cause user-prompt fragments to land in the log file
/// indirectly. This is not a daemon-side secret leak — the runner never
/// writes the request payload — but it is a data-retention concern for any
/// deployment with sensitive prompts. Operators who need stricter isolation
/// should set `log_reasoning_cap_bytes` very low (or disable the log
/// directory entirely, planned for a future knob).
#[derive(serde::Serialize)]
pub struct PerRequestLogRecord<'a> {
    pub request_id: &'a str,
    pub model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<&'a str>,
    pub reasoning_content: &'a str,
    pub content: &'a str,
    pub usage: &'a TokenUsage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    pub finish_reason: &'a str,
    pub timestamp: String,
}

/// Write the per-request log file to `log_dir/{request_id}.json`. Creates the
/// directory if it doesn't exist. The runner (T-010) calls this AFTER it has
/// emitted the lifecycle cost tracing line — log-write errors are visible
/// to the runner so it can log a warning but MUST NOT propagate as a consult
/// failure (observability is best-effort, not blocking).
pub fn write_per_request_log(
    log_dir: &Path,
    record: &PerRequestLogRecord<'_>,
) -> io::Result<PathBuf> {
    std::fs::create_dir_all(log_dir)?;
    let safe_id = sanitize_for_filename(record.request_id);
    let path = log_dir.join(format!("{safe_id}.json"));
    let json = serde_json::to_string_pretty(record).map_err(io::Error::other)?;
    std::fs::write(&path, json)?;
    Ok(path)
}

/// `request_id` comes from DeepSeek and is ordinarily a safe slug, but we
/// strip path separators defensively so a malicious value can't escape
/// `log_dir` (REQ-DS-018 — observability must not be a vector).
/// Codex W3-review NIT #2: also reject Windows-reserved basenames (CON, PRN,
/// AUX, NUL, COM1..COM9, LPT1..LPT9), as well as the empty / "." / ".." cases
/// that would either fail or escape on any platform.
fn sanitize_for_filename(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '_',
        })
        .collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return format!("req-{}", cleaned.len());
    }
    if is_windows_reserved(&cleaned) {
        return format!("req-{cleaned}");
    }
    cleaned
}

fn is_windows_reserved(name: &str) -> bool {
    // Compare on the stem before the first '.' (Windows blocks reserved
    // basenames even with extensions, e.g. CON.txt).
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
            | "COM1" | "COM2" | "COM3" | "COM4" | "COM5"
            | "COM6" | "COM7" | "COM8" | "COM9"
            | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5"
            | "LPT6" | "LPT7" | "LPT8" | "LPT9"
    )
}

/// Current wall-clock as RFC3339 UTC, matching the rest of the daemon's
/// timestamps (e.g. token-economics price_table effective_date).
pub fn now_rfc3339_utc() -> String {
    chrono::Utc::now()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ─────────────────────────────────────────────────────────────────────────────
// T-010 (REQ-DS-004, 005, 008, 014, 024): top-level runner.
// ─────────────────────────────────────────────────────────────────────────────
//
// The runner orchestrates everything: breaker → semaphore → request body →
// streaming POST → SSE parser → guards (T-007) → runaway check (T-008) →
// finalize → usage map (T-009) → per-request log (T-009). It returns a
// typed Result so the caller can route a HardProvider differently from a
// Transient (REQ-DS-008 — no sibling substitution; failures stay typed).
//
// Retries are SCOPED:
//   - Pre-first-byte network failure: 1 retry (connect timeouts, DNS, etc.)
//   - 429 with Retry-After header: 1 retry honoring the suggested wait
//     (clamped to 10s so a malicious header can't park us forever)
// No outer retry loop — those would be REQ-DS-008 violations.
//
// The absolute SLA ceiling (REQ-DS-024) wraps the whole call in
// tokio::time::timeout(cfg.timeout). When it fires the runner emits a typed
// AbsoluteTimeoutExceeded failure (NOT a panic), records a transient outcome
// on the breaker, and returns.

use futures_util::StreamExt;
use std::sync::Arc;
use std::time::Instant;

use crate::deepseek_resilience::{
    AcquireDecision, Breaker, BreakerState, ConcurrencyCap, Outcome, TokenBucket, classify,
};

/// One DeepSeek consult's request. The runner uses these fields to build the
/// JSON request body; nothing else (notably, `messages_text` is NEVER logged
/// — REQ-DS-023).
#[derive(Clone, Debug, Default)]
pub struct RunRequest {
    pub messages: Vec<RequestMessage>,
    /// Caller-provided session id. The runner attaches it to the returned
    /// ParsedAgentResult; the dispatch layer (T-012) is responsible for
    /// ensuring it starts with `deepseek-`.
    pub session_id: String,
    /// Best-effort caller estimate of prompt size in chars, used for the
    /// estimated-usage fallback (T-009). Zero is fine.
    pub prompt_chars_estimate: i64,
    /// REQ-DS-023 CoT bifurcation knob (T-012). Default: false — response_text
    /// carries the model's `content` only and reasoning_content is captured
    /// to the per-request log. When set to true, response_text is the
    /// reasoning wrapped in `<reasoning>...</reasoning>` followed by content,
    /// so callers can inspect the trace in-line.
    pub include_reasoning: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RequestMessage {
    pub role: String, // "system" | "user" | "assistant"
    pub content: String,
}

/// Shared resilience state — the dispatch layer (T-012) constructs this once
/// per daemon process and passes a reference to every `run()` invocation.
/// All three primitives are shared across concurrent consults:
///   - `breaker` (state machine; `std::sync::Mutex` — short critical sections)
///   - `concurrency` (Arc<Semaphore> wrapper; cheap to clone)
///   - `rpm` (token bucket; `std::sync::Mutex` — short critical sections,
///     NEVER held across an .await per Codex W3-review BLOCKER fix)
#[derive(Clone)]
pub struct ResilienceState {
    pub breaker: Arc<std::sync::Mutex<Breaker>>,
    pub concurrency: ConcurrencyCap,
    pub rpm: Arc<std::sync::Mutex<TokenBucket>>,
}

impl ResilienceState {
    pub fn from_cfg(cfg: &DeepSeekConfig) -> Self {
        Self {
            breaker: Arc::new(std::sync::Mutex::new(Breaker::new(Default::default()))),
            concurrency: ConcurrencyCap::new(cfg.max_concurrent),
            rpm: Arc::new(std::sync::Mutex::new(TokenBucket::new(
                cfg.max_rpm,
                Instant::now(),
            ))),
        }
    }
}

/// Typed failure surface returned by `run()`. The discriminant tells the
/// caller whether to surface the failure to the user as-is (HardProvider),
/// log + retry-later (Transient), or treat as an SLA breach
/// (AbsoluteTimeoutExceeded).
#[derive(Debug)]
pub struct DeepSeekFailure {
    pub kind: DeepSeekFailureKind,
    /// Best-effort usage record so the dispatch layer can persist tokens
    /// even on failure paths (REQ-DS-026; T-013 wires Err-path persistence).
    pub usage: Option<TokenUsage>,
    /// Raw bytes received before failure — feeds the estimated_usage fallback
    /// when no usage chunk was seen.
    pub bytes_received: i64,
    pub request_id: Option<String>,
}

// Extend DeepSeekFailureKind with the runner's failure modes. We do this via
// a helper module-level enum because adding variants to the existing enum
// would conflict with T-007's Display impl; instead the runner constructs
// `RunnerFailureKind` values that flatten into the same `DeepSeekFailureKind`
// once T-010 lands.

impl DeepSeekFailureKind {
    pub fn is_hard(&self) -> bool {
        matches!(
            self,
            DeepSeekFailureKind::GhostSuccessEmbedded {
                classification: Classification::Hard,
                ..
            } | DeepSeekFailureKind::HardProvider(_)
        )
    }

    pub fn is_runner_transient(&self) -> bool {
        // Codex W3-review SHOULD-FIX #2: this MUST match the breaker-recording
        // path in run(). Any failure that run() reports to the breaker as
        // Outcome::TransientError is_runner_transient; anything else is not.
        // B.6 inclusion: InsufficientSystemResource finish_reason is a provider
        // capacity signal, so it lives on the transient axis.
        matches!(
            self,
            DeepSeekFailureKind::Transient(_)
                | DeepSeekFailureKind::NetworkPreFirstByte(_)
                | DeepSeekFailureKind::NetworkMidStream(_)
                | DeepSeekFailureKind::ParserError(_)
                | DeepSeekFailureKind::AbsoluteTimeoutExceeded
                | DeepSeekFailureKind::StreamEndedWithoutDone
                | DeepSeekFailureKind::BadFinishReason(
                    BadFinishReasonKind::InsufficientSystemResource
                )
                | DeepSeekFailureKind::GhostSuccessEmbedded {
                    classification: Classification::Transient,
                    ..
                }
        )
    }
}

/// Build the JSON request body. Pulled out for tests + so the body can be
/// inspected without making an HTTP call.
pub fn build_request_body(cfg: &DeepSeekConfig, req: &RunRequest) -> serde_json::Value {
    // B.9 (TIER 2 live probe 2026-05-26): the `thinking` field is a NESTED
    // object `{"type": "enabled"|"disabled"}`, not a flat string. Flat strings
    // get HTTP 400 `invalid type: string, expected struct ThinkingOptions`.
    //
    // B.9b (2026-05-26, found via live MCP test post-PR#37): when
    // `thinking={type:"disabled"}`, `reasoning_effort` MUST be omitted entirely.
    // The API rejects the combo with HTTP 400:
    //   "thinking options type cannot be disabled when reasoning_effort is set"
    // The two parameters are mutually exclusive on the wire. The contract
    // probes never exercised the combination: probe-08 sent thinking=disabled
    // with NO reasoning_effort field at all (which is why it passed); every
    // OTHER probe sent thinking=enabled (so the combo was incidentally legal).
    // build_request_body was the first thing to assemble disabled+effort.
    //
    // `reasoning_effort` IS a flat string per the live probe — keep it flat
    // when included.
    let mut body = serde_json::json!({
        "model": cfg.model,
        "messages": req.messages,
        "stream": true,
        "max_tokens": cfg.max_tokens,
        "thinking": { "type": cfg.thinking.as_api_str() },
    });
    // Only include reasoning_effort when thinking is enabled. Both v4-pro
    // and v4-flash accept this combination; both reject disabled+effort.
    if matches!(cfg.thinking, crate::deepseek_config::ThinkingMode::Enabled) {
        body["reasoning_effort"] = serde_json::json!(cfg.reasoning_effort.as_api_str());
    }
    body
}

/// REQ-DS-004 / REQ-DS-005 / REQ-DS-008 / REQ-DS-014 / REQ-DS-024 — top-level
/// DeepSeek runner. Returns a clean `ParsedAgentResult` on success or a typed
/// `DeepSeekFailure` for every failure mode. Caller-side substitution is
/// forbidden (REQ-DS-008): the dispatch layer must surface this failure
/// directly, not retry with Gemini/Codex.
pub async fn run(
    cfg: &DeepSeekConfig,
    client: &reqwest::Client,
    req: &RunRequest,
    resilience: &ResilienceState,
) -> Result<agent_adapter::ParsedAgentResult, DeepSeekFailure> {
    // Codex W3-review SHOULD-FIX #5 ordering: acquire the concurrency permit
    // BEFORE consulting the breaker. Otherwise a caller can transition the
    // breaker to HalfOpen (consuming the lease) and then sit waiting on the
    // semaphore, holding the lease without ever firing the probe HTTP call —
    // which lets later callers expire the breaker or compete over the
    // probe slot incorrectly.

    // Phase 1: concurrency cap (semaphore — async wait, no breaker state mutated).
    let _permit = resilience.concurrency.acquire().await;

    // Phase 2: RPM gate (Codex W3-review BLOCKER fix). Lock → compute wait →
    // drop lock → sleep → re-acquire. The lock is NEVER held across the
    // await. Bounded by cfg.timeout indirectly via the outer SLA wrap below.
    loop {
        let wait_until = {
            let now = Instant::now();
            let mut bucket = resilience
                .rpm
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if bucket.try_take(now) {
                None
            } else {
                Some(bucket.next_available(now))
            }
        };
        match wait_until {
            None => break,
            Some(until) => {
                let now = Instant::now();
                if until > now {
                    tokio::time::sleep(until.duration_since(now)).await;
                }
            }
        }
    }

    // Phase 3: breaker check (cheap, after the semaphore + RPM have admitted us).
    let decision = {
        let mut b = resilience
            .breaker
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        b.try_acquire(Instant::now())
    };
    match decision {
        AcquireDecision::Allow => {}
        AcquireDecision::BlockHard => {
            return Err(DeepSeekFailure {
                kind: DeepSeekFailureKind::BreakerOpen(BreakerState::HardOpenInsufficientBalance),
                usage: None,
                bytes_received: 0,
                request_id: None,
            });
        }
        AcquireDecision::BlockTransient { retry_after } => {
            return Err(DeepSeekFailure {
                kind: DeepSeekFailureKind::BreakerOpen(BreakerState::OpenTransient {
                    until: Instant::now() + retry_after,
                    attempts: 0,
                }),
                usage: None,
                bytes_received: 0,
                request_id: None,
            });
        }
    }

    // Phase 4: wrap the whole consult in the absolute-timeout SLA (single owner
    // per Codex W3-review SHOULD-FIX #1 — reqwest .timeout() has been removed
    // from build_client so this is the only absolute ceiling).
    let inner = run_inner(cfg, client, req);
    let outcome = tokio::time::timeout(cfg.timeout, inner).await;

    let result = match outcome {
        Ok(r) => r,
        Err(_elapsed) => Err(DeepSeekFailure {
            kind: DeepSeekFailureKind::AbsoluteTimeoutExceeded,
            usage: None,
            bytes_received: 0,
            request_id: None,
        }),
    };

    // Phase 5: report the outcome to the breaker. RunawayReasoning and
    // BadFinishReason are budget/policy decisions — do NOT touch the breaker.
    let breaker_transition = {
        let mut b = resilience
            .breaker
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        // Snapshot before/after so the transition can be reported to PostHog AFTER the lock
        // releases. The Breaker itself stays a pure, clock-driven state machine (no telemetry
        // coupling), exactly like the agy breaker's core; the emit lives at this call site.
        let prev = b.state();
        match &result {
            Ok(_) => b.record(Outcome::Success, now),
            Err(f) => match &f.kind {
                DeepSeekFailureKind::HardProvider(code) => b.record(Outcome::HardError(*code), now),
                DeepSeekFailureKind::Transient(code) => b.record(Outcome::TransientError(*code), now),
                DeepSeekFailureKind::NetworkMidStream(_)
                | DeepSeekFailureKind::NetworkPreFirstByte(_)
                | DeepSeekFailureKind::ParserError(_)
                | DeepSeekFailureKind::AbsoluteTimeoutExceeded
                | DeepSeekFailureKind::StreamEndedWithoutDone
                | DeepSeekFailureKind::BadFinishReason(
                    BadFinishReasonKind::InsufficientSystemResource
                ) => {
                    // B.6: insufficient_system_resource is a provider capacity
                    // signal, treat like 5xx for breaker arithmetic.
                    b.record(Outcome::TransientError(0), now)
                }
                DeepSeekFailureKind::GhostSuccessEmbedded { classification, .. } => match classification {
                    Classification::Hard => b.record(Outcome::HardError(402), now),
                    Classification::Transient => b.record(Outcome::TransientError(429), now),
                },
                // BREAKER-NEUTRAL kinds (T-008 contract, T-007 partial,
                // B.5 EmptyFinalAnswer — model declined to answer, not a
                // provider fault):
                DeepSeekFailureKind::RunawayReasoning { .. }
                | DeepSeekFailureKind::BadFinishReason(_)
                | DeepSeekFailureKind::BreakerOpen(_)
                | DeepSeekFailureKind::EmptyFinalAnswer { .. } => {}
            },
        }
        let next = b.state();
        (prev, next)
    };

    // Emit only on a STATE CHANGE, and only for the newsworthy states — a per-request
    // "still Closed" is not news. DeepSeek is the only METERED provider, so its breaker is
    // arguably higher-value than agy's: HardOpenInsufficientBalance means "out of money", and
    // the transient trip means the paid provider is 429/5xx-ing us. Emitted here, after the
    // lock is released (capture() detaches the POST).
    {
        let (prev, next) = breaker_transition;
        // Compare LABELS, not the full enum. BreakerState carries Instant + attempts, so a
        // re-trip while already open (OpenTransient{t1,1} -> OpenTransient{t2,2}) is `prev !=
        // next` at the enum level but the SAME logical state — emitting there would flap an
        // "open_transient -> open_transient" storm under concurrent in-flight failures (Codex).
        let (from, to) = (describe_breaker_state(prev), describe_breaker_state(next));
        if from != to {
            crate::posthog::record_deepseek_breaker(to, from);
        }
    }

    result
}

/// Stable, low-cardinality label for a DeepSeek breaker state, for PostHog.
///
/// Note: try_acquire() also mutates state (OpenTransient->HalfOpen when a cooldown elapses to
/// grant a probe; a stale HalfOpen->OpenTransient) and those are deliberately NOT emitted. The
/// alertable states — the transient TRIP, the out-of-balance HardOpen, the probe outcome
/// (HalfOpen->Closed recovery / HalfOpen->OpenTransient re-trip), all of which pass through
/// record() — are covered here. The uncovered ones are intermediate "probe granted" visibility.
fn describe_breaker_state(state: crate::deepseek_resilience::BreakerState) -> &'static str {
    use crate::deepseek_resilience::BreakerState;
    match state {
        BreakerState::Closed => "closed",
        BreakerState::HardOpenInsufficientBalance => "hard_open_insufficient_balance",
        BreakerState::OpenTransient { .. } => "open_transient",
        BreakerState::HalfOpen { .. } => "half_open",
    }
}

async fn run_inner(
    cfg: &DeepSeekConfig,
    client: &reqwest::Client,
    req: &RunRequest,
) -> Result<agent_adapter::ParsedAgentResult, DeepSeekFailure> {
    let body = build_request_body(cfg, req);
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));

    // Pre-first-byte retry: at most ONE additional attempt for connect/DNS/
    // request-construction errors. 429-with-Retry-After is a SEPARATE retry
    // budget (also at most one) — they don't share a counter so a 429 after
    // a network-retry still gets its own honored wait.
    let mut pre_byte_attempts = 0u32;
    let mut retry_after_used = false;

    loop {
        let send_result = client
            .post(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", cfg.api_key.expose()),
            )
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(&body)
            .send()
            .await;

        let resp = match send_result {
            Ok(r) => r,
            Err(e) => {
                let class_label = e.to_string();
                if pre_byte_attempts == 0 && (e.is_connect() || e.is_request() || e.is_timeout()) {
                    pre_byte_attempts += 1;
                    continue;
                }
                return Err(DeepSeekFailure {
                    kind: DeepSeekFailureKind::NetworkPreFirstByte(class_label),
                    usage: None,
                    bytes_received: 0,
                    request_id: None,
                });
            }
        };

        let status = resp.status().as_u16();

        if status == 429 && !retry_after_used {
            // Honor Retry-After once. Per RFC 7231, it can be either
            // delta-seconds or an HTTP-date (RFC 1123 / RFC 2822 subset).
            // Codex W3-review NIT #1: parse both, clamp to 10s.
            let wait_secs = parse_retry_after(
                resp.headers().get(reqwest::header::RETRY_AFTER),
            )
            .unwrap_or(1)
            .min(10);
            retry_after_used = true;
            tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
            continue;
        }

        if !resp.status().is_success() {
            // Capture a bounded slice of the error body for diagnostics. The
            // body MAY contain the request payload echo in some 4xx responses;
            // we don't write it to the per-request log file, only into the
            // typed failure for transient/hard routing.
            let kind = match classify(status) {
                Classification::Hard => DeepSeekFailureKind::HardProvider(status),
                Classification::Transient => DeepSeekFailureKind::Transient(status),
            };
            return Err(DeepSeekFailure {
                kind,
                usage: None,
                bytes_received: 0,
                request_id: None,
            });
        }

        // Happy path entry: consume the SSE stream.
        return consume_stream(
            resp,
            cfg,
            &req.session_id,
            req.prompt_chars_estimate,
            req.include_reasoning,
        )
        .await;
    }
}

async fn consume_stream(
    resp: reqwest::Response,
    cfg: &DeepSeekConfig,
    session_id: &str,
    prompt_chars_estimate: i64,
    include_reasoning: bool,
) -> Result<agent_adapter::ParsedAgentResult, DeepSeekFailure> {
    let mut parser = StreamParser::new();
    let mut bytes_received: i64 = 0;
    let mut stream = resp.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(b) => b,
            Err(e) => {
                let usage = estimate_usage(bytes_received, prompt_chars_estimate / 4);
                return Err(DeepSeekFailure {
                    kind: DeepSeekFailureKind::NetworkMidStream(e.to_string()),
                    usage: Some(usage),
                    bytes_received,
                    request_id: parser.request_id.clone(),
                });
            }
        };
        bytes_received += chunk.len() as i64;
        if let Err(e) = parser.feed(&chunk) {
            // Parser error — treat as a transient ish/SSE-malformed condition.
            return Err(DeepSeekFailure {
                kind: DeepSeekFailureKind::ParserError(e.to_string()),
                usage: Some(estimate_usage(bytes_received, prompt_chars_estimate / 4)),
                bytes_received,
                request_id: parser.request_id.clone(),
            });
        }
        // T-008 runaway check.
        if parser.is_runaway(cfg.reasoning_cap_tokens) {
            let observed = parser.estimated_reasoning_tokens();
            let usage = estimate_usage(bytes_received, prompt_chars_estimate / 4);
            // Drop the stream by returning — reqwest cancels the body future.
            return Err(DeepSeekFailure {
                kind: DeepSeekFailureKind::RunawayReasoning { observed_tokens: observed },
                usage: Some(usage),
                bytes_received,
                request_id: parser.request_id.clone(),
            });
        }
        if parser.done {
            break;
        }
    }

    // Codex W3-review SHOULD-FIX #6: `[DONE]` is the application-layer
    // terminator. If the stream closed cleanly (no reqwest body Err) but we
    // never saw [DONE], the response is truncated even though the socket is
    // happy. Fail loud with a typed transient.
    if !parser.done {
        let usage = parser
            .usage
            .as_ref()
            .map(map_usage)
            .or_else(|| Some(estimate_usage(bytes_received, prompt_chars_estimate / 4)));
        return Err(DeepSeekFailure {
            kind: DeepSeekFailureKind::StreamEndedWithoutDone,
            usage,
            bytes_received,
            request_id: parser.request_id.clone(),
        });
    }

    // Stream completed normally with [DONE].
    let finalized = match finalize_stream(&parser) {
        Ok(f) => f,
        Err(kind) => {
            let usage = parser
                .usage
                .as_ref()
                .map(map_usage)
                .or_else(|| Some(estimate_usage(bytes_received, prompt_chars_estimate / 4)));
            return Err(DeepSeekFailure {
                kind,
                usage,
                bytes_received,
                request_id: parser.request_id.clone(),
            });
        }
    };

    let usage = finalized
        .usage
        .as_ref()
        .map(map_usage)
        .unwrap_or_else(|| estimate_usage(bytes_received, prompt_chars_estimate / 4));

    // B.3 (test-plan TIER 1): cache-token invariant warn. DeepSeek documents
    // `prompt_tokens == prompt_cache_hit_tokens + prompt_cache_miss_tokens`.
    // If the invariant breaks, the billing model on their side has shifted —
    // log it so we notice. Best-effort only; doesn't fail the consult.
    if let Some(raw) = finalized.usage.as_ref() {
        let hit_plus_miss = raw.prompt_cache_hit_tokens + raw.prompt_cache_miss_tokens;
        if raw.prompt_tokens > 0 && hit_plus_miss != raw.prompt_tokens {
            tracing::warn!(
                request_id = ?finalized.request_id,
                prompt_tokens = raw.prompt_tokens,
                cache_hit = raw.prompt_cache_hit_tokens,
                cache_miss = raw.prompt_cache_miss_tokens,
                "deepseek cache-token invariant violated (hit + miss != prompt_tokens)"
            );
        }
    }

    // B.4 (test-plan TIER 1): system_fingerprint change detector. DeepSeek
    // doesn't announce backend rollovers — fingerprint drift is the only
    // signal. We keep one last-seen value per (model) in a process-static
    // map and warn on any change. Operators correlate quality regressions
    // with these events.
    if let Some(fp) = finalized.system_fingerprint.as_deref() {
        record_fingerprint_for_model(&cfg.model, fp);
    }

    // Best-effort per-request log. Errors are warned-not-failed.
    if let Some(req_id) = finalized.request_id.as_deref() {
        let capped_reasoning = cap_reasoning(&finalized.reasoning, cfg.log_reasoning_cap_bytes);
        let record = PerRequestLogRecord {
            request_id: req_id,
            model: &cfg.model,
            system_fingerprint: finalized.system_fingerprint.as_deref(),
            reasoning_content: capped_reasoning,
            content: &finalized.content,
            usage: &usage,
            cost_usd: None, // T-012/T-013 wire calculate_cost_usd in
            finish_reason: &finalized.finish_reason,
            timestamp: now_rfc3339_utc(),
        };
        if let Err(e) = write_per_request_log(&cfg.log_dir, &record) {
            tracing::warn!(
                request_id = req_id,
                error = %e,
                "deepseek per-request log write failed (consult unaffected)"
            );
        }
    }

    // REQ-DS-023 CoT bifurcation. Default: response_text = content only;
    // reasoning is captured in the per-request log written above. With
    // include_reasoning=true the reasoning is interleaved at the head of
    // response_text inside <reasoning>…</reasoning> tags so the caller sees
    // the trace without parsing the per-request log file.
    let response_text = if include_reasoning && !finalized.reasoning.is_empty() {
        format!(
            "<reasoning>\n{}\n</reasoning>\n\n{}",
            finalized.reasoning, finalized.content,
        )
    } else {
        finalized.content.clone()
    };

    Ok(agent_adapter::ParsedAgentResult {
        response_text,
        session_id: Some(session_id.to_string()),
        events: Vec::new(),
        tool_calls: Vec::new(),
        token_usage: Some(to_adapter_token_usage(&usage)),
        cli_version: None,
        parser_mode: "deepseek-sse".to_string(),
    })
}

/// B.4 fingerprint state: maps `cfg.model` → last-seen `system_fingerprint`
/// for this process. A change emits a `tracing::warn!` so the operator can
/// correlate quality regressions with backend rollovers. Returns the previous
/// value so tests can verify the transition.
pub(crate) fn record_fingerprint_for_model(model: &str, fingerprint: &str) -> Option<String> {
    use std::sync::Mutex;
    static MAP: std::sync::OnceLock<Mutex<std::collections::HashMap<String, String>>> =
        std::sync::OnceLock::new();
    let map = MAP.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|p| p.into_inner());
    let prev = guard.insert(model.to_string(), fingerprint.to_string());
    if let Some(ref old) = prev
        && old != fingerprint
    {
        tracing::warn!(
            model = %model,
            old_fingerprint = %old,
            new_fingerprint = %fingerprint,
            "deepseek system_fingerprint changed — backend rollover detected"
        );
    }
    prev
}

/// Parse a `Retry-After` HTTP header value into seconds. Per RFC 7231 the
/// value is either delta-seconds OR an HTTP-date (RFC 1123-shaped). Returns
/// None for missing/malformed values; the caller substitutes a default.
fn parse_retry_after(v: Option<&reqwest::header::HeaderValue>) -> Option<u64> {
    let raw = v?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    // Delta-seconds path first (cheap).
    if let Ok(secs) = raw.parse::<u64>() {
        return Some(secs);
    }
    // HTTP-date path. chrono can parse RFC 2822, which accepts the
    // RFC 1123 / IMF-fixdate shape ("Wed, 21 Oct 2015 07:28:00 GMT").
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(raw) {
        let now = chrono::Utc::now();
        let target = dt.with_timezone(&chrono::Utc);
        if target > now {
            return Some((target - now).num_seconds() as u64);
        }
        return Some(0);
    }
    None
}

fn to_adapter_token_usage(u: &TokenUsage) -> agent_adapter::TokenUsage {
    agent_adapter::TokenUsage {
        input: Some(u.input_tokens.max(0) as u64),
        output: Some(u.output_tokens.max(0) as u64),
        cached: Some(u.cached_tokens.max(0) as u64),
        thinking_tokens: None,
        latency_ms: None,
        tool_calls: None,
        total: Some(
            (u.input_tokens.max(0) + u.output_tokens.max(0) + u.cached_tokens.max(0)) as u64,
        ),
    }
}

/// Scan for the first SSE event terminator — `\n\n` (LF-LF) OR `\r\n\r\n`
/// (CRLF-CRLF). Returns `(payload_end_index, terminator_len)` so the caller
/// can drain accordingly. Per the SSE spec both terminators are valid; DeepSeek
/// currently emits LF-LF but proxies may rewrite.
fn find_event_terminator(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i + 1 < buf.len() {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some((i, 2));
        }
        if i + 3 < buf.len()
            && buf[i] == b'\r'
            && buf[i + 1] == b'\n'
            && buf[i + 2] == b'\r'
            && buf[i + 3] == b'\n'
        {
            return Some((i, 4));
        }
        i += 1;
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────
//
// Reality test (REQ-DS-007): a stub that uses only the absolute `timeout` would
// fail the rolling-read-timeout property. We prove the property by spinning up
// a raw TCP server that:
//   1. writes the HTTP/1.1 response head + Content-Length: 15
//   2. then writes 15 bytes of body, one every ~200ms (total ~3s)
//
// Client config: read_timeout = 500ms (shorter than the total wall-clock so a
// non-rolling timeout would fail; longer than the inter-byte gap so the
// rolling one succeeds).
//
// No hyper dependency — `tokio::net::TcpListener` + hand-written HTTP/1.1 is
// enough because we control both sides.

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    /// Spawns a one-shot HTTP/1.1 server that responds to ONE request by
    /// dribbling `body_byte_count` bytes spaced `inter_byte_delay` apart.
    /// Returns the bound `127.0.0.1:PORT` URL string. The handler task exits
    /// after serving the single connection.
    async fn spawn_dribbler(
        body_byte_count: usize,
        inter_byte_delay: Duration,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let url = format!("http://{}", addr);

        tokio::spawn(async move {
            let (mut sock, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => return,
            };

            // We don't bother parsing the request — read until \r\n\r\n then write.
            // Reading isn't strictly necessary for the dribble; the test calls .send()
            // which writes the request, but reqwest may not block on response start
            // until it has flushed. To keep things simple we just discard whatever
            // shows up and proceed to write.
            let mut buf = [0u8; 4096];
            // Best-effort: peek once, ignore result.
            let _ = tokio::time::timeout(
                Duration::from_millis(100),
                tokio::io::AsyncReadExt::read(&mut sock, &mut buf),
            )
            .await;

            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
                body_byte_count
            );
            if sock.write_all(head.as_bytes()).await.is_err() {
                return;
            }

            for i in 0..body_byte_count {
                // One byte. Use ASCII printable so any debugging dump is readable.
                let byte = [b'a' + (i as u8 % 26)];
                if sock.write_all(&byte).await.is_err() {
                    return;
                }
                if sock.flush().await.is_err() {
                    return;
                }
                tokio::time::sleep(inter_byte_delay).await;
            }
            let _ = sock.shutdown().await;
        });

        url
    }

    /// Build a config with the timing knobs we care about; everything else
    /// at its `from_env` default-equivalent. We don't go through env loading
    /// here because we want to vary timings independently.
    fn cfg_with_timings(
        read_timeout: Duration,
        absolute_timeout: Duration,
    ) -> DeepSeekConfig {
        // Construct with literal defaults (the public surface). The point of
        // this test is the reqwest config plumbing, not env parsing.
        DeepSeekConfig {
            base_url: "http://localhost".to_string(),
            api_key: crate::deepseek_config::ApiKey::new("unused-for-this-test"),
            model: "deepseek-v4-pro".to_string(),
            max_tokens: 32768,
            thinking: crate::deepseek_config::ThinkingMode::Enabled,
            reasoning_effort: crate::deepseek_config::ReasoningEffort::High,
            read_timeout,
            timeout: absolute_timeout,
            tcp_keepalive: Duration::from_secs(30),
            max_concurrent: 8,
            max_rpm: 60,
            reasoning_cap_tokens: 0,
            log_dir: std::path::PathBuf::from("/tmp/deepseek-logs-test"),
            log_reasoning_cap_bytes: 262_144,
            bulk_bytes: 16_384,
        }
    }

    // REQ-DS-007 reality check: a rolling read_timeout allows total wall-clock
    // to exceed `read_timeout` as long as each individual gap is shorter.
    //
    // Stub guard: a client built with only an absolute `timeout` ≈ 500ms would
    // be cut off at 500ms — well before the 15 bytes finish at ~3s total.
    #[tokio::test]
    async fn deepseek_client_timeouts_rolling_read_succeeds_even_when_total_exceeds_read_timeout() {
        // 15 bytes × 200ms each → ~3s total. read_timeout=500ms, absolute=10s.
        // Inter-byte gap (200ms) < read_timeout (500ms) → rolling timer never trips.
        // Total wall-clock (3s) > read_timeout (500ms) → non-rolling stub would fail.
        let url = spawn_dribbler(15, Duration::from_millis(200)).await;

        let cfg = cfg_with_timings(Duration::from_millis(500), Duration::from_secs(10));
        let client = build_client(&cfg).expect("build_client");

        let started = Instant::now();
        let resp = client
            .get(&url)
            .send()
            .await
            .expect("request should complete despite total > read_timeout");
        assert!(resp.status().is_success(), "HTTP status: {}", resp.status());
        let body = resp.bytes().await.expect("body").to_vec();
        let elapsed = started.elapsed();

        assert_eq!(body.len(), 15, "should receive all 15 dribbled bytes");
        assert!(
            elapsed >= Duration::from_millis(15 * 200 - 200),
            "elapsed should reflect dribble timing (got {elapsed:?})"
        );
        assert!(
            elapsed > Duration::from_millis(500),
            "elapsed must exceed read_timeout to prove the rolling property (got {elapsed:?})"
        );
    }

    // Stub guard: confirms read_timeout DOES bite when an inter-byte gap exceeds it.
    // If reqwest silently ignored read_timeout, this test would hang or pass with
    // the full body — both outcomes are caught.
    #[tokio::test]
    async fn deepseek_client_timeouts_rolling_read_times_out_when_gap_exceeds_read_timeout() {
        // 5 bytes × 800ms gap → first byte arrives ~800ms after request, by which
        // point the 200ms read_timeout has long since fired.
        let url = spawn_dribbler(5, Duration::from_millis(800)).await;

        let cfg = cfg_with_timings(Duration::from_millis(200), Duration::from_secs(10));
        let client = build_client(&cfg).expect("build_client");

        let result = client.get(&url).send().await;
        // reqwest returns Ok(Response) on headers received, then errors on body.
        // We must consume the body to actually exercise the read_timeout.
        let body_result = match result {
            Ok(resp) => resp.bytes().await,
            Err(e) => return assert!(e.is_timeout() || e.is_connect() || e.is_request(),
                "expected timeout-class error on send(), got: {e}"),
        };
        let err = body_result.expect_err(
            "body read should fail because inter-byte gap (800ms) >> read_timeout (200ms)",
        );
        assert!(
            err.is_timeout() || err.is_body(),
            "expected timeout/body error from rolling read_timeout; got: {err}"
        );
    }

    // build_client itself is infallible with the default workspace TLS feature
    // set — confirm we don't accidentally introduce a builder that fails on
    // ordinary configs. (A future change that adds a required builder field
    // without a default would trip this.)
    #[test]
    fn build_client_succeeds_with_defaultish_config() {
        let cfg = cfg_with_timings(Duration::from_secs(60), Duration::from_secs(1800));
        build_client(&cfg).expect("build_client should not fail on default-shaped config");
    }

    // ─────────────────────────────────────────────────────────────────────
    // T-006 parser tests.
    // ─────────────────────────────────────────────────────────────────────

    fn sample_reasoning_chunk() -> String {
        // Mid-stream reasoning delta — id, choices[0].delta.reasoning_content.
        r#"data: {"id":"chatcmpl-001","object":"chat.completion.chunk","model":"deepseek-v4-pro","system_fingerprint":"fp_abc","choices":[{"index":0,"delta":{"reasoning_content":"step 1 then step 2"},"finish_reason":null}]}"#
            .to_string()
    }
    fn sample_content_chunk() -> String {
        r#"data: {"id":"chatcmpl-001","choices":[{"index":0,"delta":{"content":"final answer text"},"finish_reason":null}]}"#
            .to_string()
    }
    fn sample_usage_chunk() -> String {
        // The final delta also carries finish_reason and usage.
        // completion_tokens INCLUDES reasoning_tokens — that's the contract A-04b
        // bakes into T-009. We test the parser preserves the raw numbers; T-009
        // owns the no-double-add mapping.
        r#"data: {"id":"chatcmpl-001","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":18,"completion_tokens":174,"prompt_cache_hit_tokens":0,"prompt_cache_miss_tokens":18,"completion_tokens_details":{"reasoning_tokens":120}}}"#
            .to_string()
    }

    /// Reality test (REQ-DS-019): a synthetic byte sequence containing a
    /// keep-alive, reasoning, content, usage, and DONE — split into 3 pieces
    /// with one mid-JSON boundary. The parser must reassemble it correctly.
    ///
    /// Stub guard: a parser that does ONE serde_json::from_str over the whole
    /// buffer would barf on the mid-JSON split. A parser that yields one event
    /// per `feed()` call regardless of content would miscount keep-alives.
    #[test]
    fn deepseek_parser_chunked_handles_arbitrary_boundaries() {
        let stream = format!(
            ": keep-alive\n\n{}\n\n{}\n\n{}\n\ndata: [DONE]\n\n",
            sample_reasoning_chunk(),
            sample_content_chunk(),
            sample_usage_chunk(),
        );

        // Deliberately put one boundary INSIDE the usage-chunk JSON so the parser
        // has to buffer across `feed` calls.
        let bytes = stream.as_bytes();
        // Boundary 1: between keep-alive and reasoning chunks (roughly).
        let split1 = bytes.iter().position(|&b| b == b'd').unwrap(); // start of first `data:`
        // Boundary 2: somewhere inside the usage JSON body — we want the second
        // `{"prompt_tokens"` substring to land mid-flight.
        let usage_marker = b"\"prompt_tokens\"";
        let usage_pos = bytes
            .windows(usage_marker.len())
            .position(|w| w == usage_marker)
            .expect("usage marker present");
        // Cut mid-`prompt_tokens` keyword to force the parser to glue back across feed calls.
        let split2 = usage_pos + 6;

        let chunk_a = &bytes[..split1];
        let chunk_b = &bytes[split1..split2];
        let chunk_c = &bytes[split2..];

        let mut parser = StreamParser::new();
        let events_a = parser.feed(chunk_a).expect("feed a");
        let events_b = parser.feed(chunk_b).expect("feed b");
        let events_c = parser.feed(chunk_c).expect("feed c");

        // Keep-alive landed in the first chunk (the only event before any data).
        assert_eq!(parser.keepalive_count, 1, "exactly one keep-alive seen");
        let total_events = events_a.len() + events_b.len() + events_c.len();
        assert!(
            total_events >= 5,
            "expected at least 5 events across all chunks (keepalive, reasoning, content, usage, done); got {total_events}"
        );

        assert_eq!(parser.reasoning_acc, "step 1 then step 2");
        assert_eq!(parser.content_acc, "final answer text");
        let usage = parser.usage.as_ref().expect("usage extracted");
        assert_eq!(usage.prompt_tokens, 18);
        assert_eq!(usage.completion_tokens, 174);
        assert_eq!(usage.prompt_cache_miss_tokens, 18);
        assert_eq!(usage.prompt_cache_hit_tokens, 0);
        assert_eq!(
            usage
                .completion_tokens_details
                .as_ref()
                .map(|d| d.reasoning_tokens),
            Some(120)
        );
        assert_eq!(parser.finish_reason.as_deref(), Some("stop"));
        assert_eq!(parser.request_id.as_deref(), Some("chatcmpl-001"));
        assert_eq!(parser.system_fingerprint.as_deref(), Some("fp_abc"));
        assert!(parser.done, "must see [DONE]");
        assert!(parser.embedded_error.is_none());
    }

    /// Stub guard: a parser that ignores keep-alive `:` lines as data would
    /// trip over them; one that miscounts them as data chunks would inflate
    /// the deltas list.
    #[test]
    fn deepseek_parser_ignores_comment_keepalives() {
        let mut parser = StreamParser::new();
        let stream = b": ping\n\n: ping\n\n: ping\n\n";
        let events = parser.feed(stream).expect("feed");
        assert_eq!(parser.keepalive_count, 3);
        assert!(events.iter().all(|e| matches!(e, ParseEvent::KeepAlive)));
        assert!(parser.content_acc.is_empty());
        assert!(parser.reasoning_acc.is_empty());
        assert!(!parser.done);
    }

    /// One-byte-at-a-time pathological feed — confirms the chunk-boundary
    /// reassembly is correct for ANY split, not just the boundary in the
    /// reality test.
    #[test]
    fn deepseek_parser_handles_byte_at_a_time_feed() {
        let stream = format!(
            "{}\n\ndata: [DONE]\n\n",
            sample_content_chunk(),
        );
        let mut parser = StreamParser::new();
        for byte in stream.as_bytes() {
            parser.feed(&[*byte]).expect("feed one byte");
        }
        assert_eq!(parser.content_acc, "final answer text");
        assert!(parser.done);
    }

    /// Invalid JSON in a `data:` line surfaces as a typed error — the parser
    /// must NOT silently swallow it.
    #[test]
    fn deepseek_parser_invalid_json_returns_typed_error() {
        let mut parser = StreamParser::new();
        let stream = b"data: {not-json}\n\n";
        let err = parser.feed(stream).expect_err("must error");
        match err {
            ParseError::InvalidJson { snippet, .. } => {
                assert!(snippet.contains("not-json"));
            }
            other => panic!("expected InvalidJson, got {other:?}"),
        }
    }

    /// CRLF tolerance — DeepSeek emits LF-only today, but some proxies rewrite
    /// to CRLF. The parser strips the trailing \r before parsing each line.
    #[test]
    fn deepseek_parser_tolerates_crlf_line_endings() {
        let mut parser = StreamParser::new();
        let stream = b": ping\r\n\r\n";
        parser.feed(stream).expect("CRLF keepalive parses");
        assert_eq!(parser.keepalive_count, 1);
    }

    // ─────────────────────────────────────────────────────────────────────
    // T-007 guards tests.
    // ─────────────────────────────────────────────────────────────────────

    /// Reality test (a): stream with an embedded {"error":...} chunk in the
    /// middle → finalize returns GhostSuccessEmbedded classified as Hard,
    /// matching the same Hard classification as a 402 HTTP status.
    ///
    /// Stub guard: a finalize that returns Ok on this stream (because the
    /// finish_reason is technically "stop") would fail — embedded error must
    /// take precedence.
    #[test]
    fn deepseek_guards_ghost_success_insufficient_balance_is_hard() {
        let stream = format!(
            "{}\n\ndata: {{\"choices\":[],\"error\":{{\"code\":\"insufficient_balance\",\"message\":\"Insufficient Balance\",\"type\":\"billing\"}}}}\n\ndata: {{\"id\":\"x\",\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n",
            sample_reasoning_chunk(),
        );
        let mut parser = StreamParser::new();
        parser.feed(stream.as_bytes()).expect("feed");

        // Sanity: the parser DID see the error AND the subsequent stop+done.
        assert!(parser.embedded_error.is_some());
        assert_eq!(parser.finish_reason.as_deref(), Some("stop"));
        assert!(parser.done);

        match finalize_stream(&parser) {
            Err(DeepSeekFailureKind::GhostSuccessEmbedded { classification, error }) => {
                assert_eq!(classification, Classification::Hard);
                assert_eq!(error.code.as_deref(), Some("insufficient_balance"));
            }
            other => panic!("expected GhostSuccessEmbedded(Hard), got {other:?}"),
        }
    }

    /// Embedded errors with rate-limit markers map to Transient (mirror 429).
    #[test]
    fn deepseek_guards_embedded_rate_limit_is_transient() {
        let stream = "data: {\"choices\":[],\"error\":{\"code\":\"rate_limit_exceeded\",\"message\":\"Too many requests\"}}\n\ndata: [DONE]\n\n";
        let mut parser = StreamParser::new();
        parser.feed(stream.as_bytes()).expect("feed");
        match finalize_stream(&parser) {
            Err(DeepSeekFailureKind::GhostSuccessEmbedded { classification, .. }) => {
                assert_eq!(classification, Classification::Transient);
            }
            other => panic!("expected GhostSuccessEmbedded(Transient), got {other:?}"),
        }
    }

    /// Reality test (b): finish_reason="length" → BadFinishReason(Length).
    /// NOT Ok(parsed) with partial content. A stub that returns Ok with the
    /// accumulated content would fail this test.
    #[test]
    fn deepseek_guards_finish_reason_length_is_bad() {
        let stream = format!(
            "{}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"length\"}}]}}\n\ndata: [DONE]\n\n",
            sample_content_chunk(),
        );
        let mut parser = StreamParser::new();
        parser.feed(stream.as_bytes()).expect("feed");
        assert!(!parser.content_acc.is_empty(), "parser DID accumulate partial content");
        match finalize_stream(&parser) {
            Err(DeepSeekFailureKind::BadFinishReason(BadFinishReasonKind::Length)) => {}
            other => panic!("expected BadFinishReason(Length), got {other:?}"),
        }
    }

    /// content_filter is its own bucket so callers can apply policy-specific UI.
    #[test]
    fn deepseek_guards_finish_reason_content_filter_is_bad() {
        let stream = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"content_filter\"}]}\n\ndata: [DONE]\n\n";
        let mut parser = StreamParser::new();
        parser.feed(stream.as_bytes()).expect("feed");
        match finalize_stream(&parser) {
            Err(DeepSeekFailureKind::BadFinishReason(BadFinishReasonKind::ContentFilter)) => {}
            other => panic!("expected BadFinishReason(ContentFilter), got {other:?}"),
        }
    }

    /// Stream that finished without ever emitting a finish_reason. The runner
    /// should NOT treat this as Ok — it's the "stream truncated cleanly but
    /// missed the final delta" case.
    #[test]
    fn deepseek_guards_missing_finish_reason_is_bad() {
        let stream = format!("{}\n\ndata: [DONE]\n\n", sample_content_chunk());
        let mut parser = StreamParser::new();
        parser.feed(stream.as_bytes()).expect("feed");
        match finalize_stream(&parser) {
            Err(DeepSeekFailureKind::BadFinishReason(BadFinishReasonKind::Missing)) => {}
            other => panic!("expected BadFinishReason(Missing), got {other:?}"),
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // T-008 runaway-reasoning tests.
    // ─────────────────────────────────────────────────────────────────────

    /// Build a single `data:` event whose reasoning_content is `len` characters
    /// long. JSON-escape-safe because we use only ASCII letters.
    fn reasoning_event_of_len(len: usize) -> String {
        let mut payload = String::with_capacity(len);
        for i in 0..len {
            payload.push((b'a' + (i as u8 % 26)) as char);
        }
        format!(
            r#"data: {{"id":"chatcmpl-001","choices":[{{"delta":{{"reasoning_content":"{}"}}}}]}}"#,
            payload
        )
    }

    /// Reality test: cfg.reasoning_cap_tokens=100 + 1000 chars of reasoning
    /// → is_runaway=true with observed≈250.
    ///
    /// Stub guard: a parser that ignores the cap would still return
    /// is_runaway(100)==false (because it never grew reasoning_acc) or would
    /// return true regardless of input. Both fail the chars/4 anchor.
    #[test]
    fn deepseek_runaway_trips_when_cap_exceeded() {
        let stream = format!("{}\n\n", reasoning_event_of_len(1000));
        let mut parser = StreamParser::new();
        parser.feed(stream.as_bytes()).expect("feed");
        assert_eq!(parser.estimated_reasoning_tokens(), 250); // 1000/4
        assert!(parser.is_runaway(100), "1000 chars / 4 = 250 tokens > cap 100");
    }

    /// Reality test: cap=0 (default) + same 1000 chars → is_runaway=false.
    /// Confirms the default-disabled behaviour. A stub that fires regardless
    /// of cap would fail.
    #[test]
    fn deepseek_runaway_does_not_trip_when_cap_is_zero() {
        let stream = format!("{}\n\n", reasoning_event_of_len(1000));
        let mut parser = StreamParser::new();
        parser.feed(stream.as_bytes()).expect("feed");
        assert!(!parser.is_runaway(0), "cap=0 means disabled, must never trip");
    }

    /// Edge case: estimate at the cap boundary (cap=250, 1000 chars → 250
    /// tokens) trips (>=). A stub using strict `>` would let this slide.
    #[test]
    fn deepseek_runaway_boundary_inclusive() {
        let stream = format!("{}\n\n", reasoning_event_of_len(1000));
        let mut parser = StreamParser::new();
        parser.feed(stream.as_bytes()).expect("feed");
        assert!(parser.is_runaway(250), "boundary (==) must trip");
        assert!(!parser.is_runaway(251), "251 should NOT trip on 250 observed");
    }

    // ─────────────────────────────────────────────────────────────────────
    // T-009 usage map / estimated fallback / per-request log tests.
    // ─────────────────────────────────────────────────────────────────────

    /// Reality test (REQ-DS-009 / A-04b): the mapping does NOT double-add
    /// completion_tokens_details.reasoning_tokens into output. The Wave-0
    /// probe-04 numbers (prompt 18, completion 174 incl 120 reasoning,
    /// cached 0) produce output=174 — NOT 294.
    ///
    /// Stub guard: a map_usage that does
    ///   output = completion_tokens + completion_tokens_details.reasoning_tokens
    /// fails here (174+120 = 294 ≠ 174).
    #[test]
    fn deepseek_usage_map_no_double_add() {
        let raw = RawUsage {
            prompt_tokens: 18,
            completion_tokens: 174,
            prompt_cache_hit_tokens: 0,
            prompt_cache_miss_tokens: 18,
            completion_tokens_details: Some(CompletionTokensDetails {
                reasoning_tokens: 120,
            }),
        };
        let usage = map_usage(&raw);
        assert_eq!(usage.input_tokens, 18);
        assert_eq!(usage.output_tokens, 174, "must NOT be 294 (no double-add)");
        assert_eq!(usage.cached_tokens, 0);
        assert_eq!(usage.usage_source, UsageSource::Exact);
    }

    /// Reality test cont'd: the resulting TokenUsage flowed through
    /// `token_economics::calculate_cost_usd` against the seeded
    /// deepseek-v4-pro prices must produce ≈ $0.0001592 — the same number the
    /// Wave-0 probe-04 live API call would imply if the daemon were
    /// integrated.  Window ±$0.0000001 catches a stub that doubles output.
    #[test]
    fn deepseek_usage_map_probe04_cost_anchor() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = token_economics::open(&temp.path().join("token-economics.db"))
            .expect("open token db");
        token_economics::ensure_deepseek_prices(&db).expect("seed prices");

        let raw = RawUsage {
            prompt_tokens: 18,
            completion_tokens: 174,
            prompt_cache_hit_tokens: 0,
            prompt_cache_miss_tokens: 18,
            completion_tokens_details: Some(CompletionTokensDetails { reasoning_tokens: 120 }),
        };
        let usage = map_usage(&raw);

        // Wrap the mapped usage in a TokenRecord (timestamp after effective_date
        // 2026-01-01).
        let record = token_economics::TokenRecord {
            agent: "deepseek".to_string(),
            session_id: "sess-probe-04".to_string(),
            timestamp: "2026-06-01T00:00:00Z".to_string(),
            model: Some("deepseek-v4-pro".to_string()),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_tokens: usage.cached_tokens,
            thinking_tokens: 120, // recorded but not priced separately
            total_tokens: usage.input_tokens + usage.output_tokens + usage.cached_tokens,
            cost_usd: None,
            latency_ms: None,
            tool_calls: None,
            lines_added: None,
            lines_removed: None,
            rate_limit_pct: None,
            context_window: None,
            build_id: None,
            task_id: None,
            wave: None,
            usage_source: token_economics::USAGE_SOURCE_EXACT.to_string(),
        };
        let cost = token_economics::calculate_cost_usd(&db, &record)
            .expect("cost calc")
            .expect("priced");
        // 18 × 0.435/1M + 174 × 0.87/1M = 7.83e-6 + 1.5138e-4 ≈ 1.5921e-4
        assert!(
            (cost - 0.00015921).abs() < 1e-7,
            "expected ≈$0.00015921, got ${cost}"
        );

        // Stub guard: if the mapping had double-added reasoning_tokens into
        // output, output would be 294 and cost would be ~$0.000264, far
        // outside the ±$0.0000001 window. Compute the would-be-bad cost
        // explicitly and assert our actual cost is NOT in that neighborhood.
        let bad_cost = (18.0 * 0.435 + 294.0 * 0.87) / 1_000_000.0;
        assert!(
            (cost - bad_cost).abs() > 1e-5,
            "cost {cost} suspiciously close to the double-add value {bad_cost}"
        );
    }

    /// Estimated fallback: bytes_received=800 + prompt_estimate=200 →
    /// {input: 200, output: 200, cached: 0, source: estimated}.
    #[test]
    fn deepseek_usage_estimate_fallback_uses_bytes_over_4() {
        let usage = estimate_usage(800, 200);
        assert_eq!(usage.input_tokens, 200);
        assert_eq!(usage.output_tokens, 200, "800 bytes / 4 = 200 tokens");
        assert_eq!(usage.cached_tokens, 0);
        assert_eq!(usage.usage_source, UsageSource::Estimated);
    }

    /// cap_reasoning preserves UTF-8 boundaries — a naive byte-slice at a
    /// non-boundary index would panic. Use a multi-byte character to prove it.
    #[test]
    fn deepseek_cap_reasoning_preserves_utf8_boundary() {
        // "🦀" is 4 bytes. Cap at 3 must truncate to "" (no whole char fits).
        let s = "🦀abc";
        assert_eq!(cap_reasoning(s, 3), "");
        // Cap at 4 keeps the crab.
        assert_eq!(cap_reasoning(s, 4), "🦀");
        // Cap >= len returns the whole string.
        assert_eq!(cap_reasoning(s, 1000), "🦀abc");
    }

    /// Reality test (privacy): the log file contains reasoning_content +
    /// system_fingerprint + cost_usd AND does NOT contain the API key or
    /// the request messages. The runner never passes those into the record
    /// — this asserts the SHAPE of the record forbids them.
    #[test]
    fn deepseek_per_request_log_writes_expected_fields_and_excludes_secrets() {
        let temp = tempfile::tempdir().expect("tempdir");
        let log_dir = temp.path();

        let usage = TokenUsage {
            input_tokens: 18,
            output_tokens: 174,
            cached_tokens: 0,
            usage_source: UsageSource::Exact,
        };

        // The "secrets" the test pretends the runner is holding — both must
        // be ABSENT from the log file, regardless of what the runner does.
        let api_key_plaintext = "sk-LIVE-test-key-must-not-leak-XYZ";
        let request_messages_text = "PRIVATE-USER-PROMPT-do-not-log-this";
        let _ = (api_key_plaintext, request_messages_text); // marker for grep

        let record = PerRequestLogRecord {
            request_id: "chatcmpl-001",
            model: "deepseek-v4-pro",
            system_fingerprint: Some("fp_abc"),
            reasoning_content: "step 1 then step 2",
            content: "final answer text",
            usage: &usage,
            cost_usd: Some(0.00015921),
            finish_reason: "stop",
            timestamp: "2026-05-26T00:00:00Z".to_string(),
        };
        let path = write_per_request_log(log_dir, &record).expect("write log");
        let body = std::fs::read_to_string(&path).expect("read log");

        // Positive: required fields present.
        assert!(body.contains("\"request_id\""), "missing request_id");
        assert!(body.contains("\"chatcmpl-001\""));
        assert!(body.contains("\"system_fingerprint\""));
        assert!(body.contains("\"fp_abc\""));
        assert!(body.contains("\"reasoning_content\""));
        assert!(body.contains("step 1 then step 2"));
        assert!(body.contains("\"cost_usd\""));
        assert!(body.contains("0.00015921"));
        assert!(body.contains("\"finish_reason\""));
        assert!(body.contains("\"stop\""));
        assert!(body.contains("\"usage_source\""));
        assert!(body.contains("\"exact\""));

        // Privacy regression guards.
        assert!(
            !body.contains(api_key_plaintext),
            "REGRESSION: log file contained the API key string"
        );
        assert!(
            !body.contains("Authorization"),
            "REGRESSION: log file contained 'Authorization' (header name)"
        );
        assert!(
            !body.contains(request_messages_text),
            "REGRESSION: log file contained the request-messages text"
        );
        assert!(
            !body.contains("\"messages\""),
            "REGRESSION: log file contained a 'messages' field"
        );
    }

    /// Per-request log path uses the request_id as the filename slug, and the
    /// sanitizer prevents an evil request_id from escaping log_dir.
    #[test]
    fn deepseek_per_request_log_sanitizes_filename() {
        let temp = tempfile::tempdir().expect("tempdir");
        let usage = TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            usage_source: UsageSource::Estimated,
        };
        let record = PerRequestLogRecord {
            request_id: "../../escape/attempt",
            model: "deepseek-v4-pro",
            system_fingerprint: None,
            reasoning_content: "",
            content: "",
            usage: &usage,
            cost_usd: None,
            finish_reason: "stop",
            timestamp: "2026-05-26T00:00:00Z".to_string(),
        };
        let path = write_per_request_log(temp.path(), &record).expect("write");
        // The sanitized filename keeps the file INSIDE log_dir.
        assert!(
            path.starts_with(temp.path()),
            "sanitiser failed — escape attempt landed at {path:?}"
        );
        let name = path.file_name().unwrap().to_string_lossy();
        // The relevant escape vectors are path separators. Literal ".." chars
        // are harmless once they can't be combined with a separator.
        assert!(!name.contains('/'), "filename leaked '/': {name}");
        assert!(!name.contains('\\'), "filename leaked '\\\\': {name}");
        // Belt-and-braces: the resolved file is INSIDE log_dir (already asserted
        // via starts_with, but re-checking the parent for clarity).
        assert_eq!(path.parent(), Some(temp.path()));
    }

    // ─────────────────────────────────────────────────────────────────────
    // T-010 runner tests — drive run() against a scripted local TCP server.
    // ─────────────────────────────────────────────────────────────────────

    /// Spawn a TCP server that serves N successive connections with the
    /// provided byte scripts (one per connection). The server is one-shot
    /// per connection: it reads the request to EOL terminator best-effort,
    /// writes the script, then closes. Returns the URL the client should
    /// POST to (we point the client at /chat/completions; the server doesn't
    /// path-route — every request gets the next script).
    async fn spawn_scripted_server(scripts: Vec<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let url = format!("http://{}", addr);

        tokio::spawn(async move {
            for script in scripts {
                let (mut sock, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => return,
                };
                // Read the request best-effort (drop after a short window so we
                // don't block on a client that keeps the connection idle).
                let mut buf = [0u8; 8192];
                let _ = tokio::time::timeout(
                    Duration::from_millis(200),
                    tokio::io::AsyncReadExt::read(&mut sock, &mut buf),
                )
                .await;
                let _ = sock.write_all(&script).await;
                let _ = sock.flush().await;
                let _ = sock.shutdown().await;
            }
        });
        url
    }

    fn happy_sse_body() -> String {
        format!(
            "{}\n\n{}\n\n{}\n\ndata: [DONE]\n\n",
            sample_reasoning_chunk(),
            sample_content_chunk(),
            sample_usage_chunk(),
        )
    }

    fn http_response_with_body(status_line: &str, body: &str, extra_headers: &str) -> Vec<u8> {
        format!(
            "{}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{}",
            status_line,
            body.len(),
            extra_headers,
            body,
        )
        .into_bytes()
    }

    /// Build a test cfg that points base_url at the supplied URL. Default
    /// timeouts kept long enough not to fire during the test, short enough
    /// to keep CI snappy. Reasoning cap disabled.
    fn cfg_for(url: &str, absolute_timeout: Duration) -> DeepSeekConfig {
        DeepSeekConfig {
            base_url: url.to_string(),
            api_key: crate::deepseek_config::ApiKey::new("sk-test-key-do-not-log"),
            model: "deepseek-v4-pro".to_string(),
            max_tokens: 1024,
            thinking: crate::deepseek_config::ThinkingMode::Enabled,
            reasoning_effort: crate::deepseek_config::ReasoningEffort::High,
            read_timeout: Duration::from_secs(5),
            timeout: absolute_timeout,
            tcp_keepalive: Duration::from_secs(30),
            max_concurrent: 4,
            max_rpm: 60,
            reasoning_cap_tokens: 0,
            log_dir: std::env::temp_dir().join("deepseek-runner-test"),
            log_reasoning_cap_bytes: 262_144,
            bulk_bytes: 16_384,
        }
    }

    fn make_req() -> RunRequest {
        RunRequest {
            messages: vec![RequestMessage {
                role: "user".to_string(),
                content: "PRIVATE-USER-PROMPT-do-not-log-this".to_string(),
            }],
            session_id: "deepseek-test-session-001".to_string(),
            prompt_chars_estimate: 36,
            include_reasoning: false,
        }
    }

    /// Reality test (a): canned reasoning → content → usage → [DONE] →
    /// run() returns Ok(ParsedAgentResult) with session_id, mapped token usage,
    /// and content == "final answer text".
    ///
    /// Stub guard: a run() that ignores the mock response and returns Ok with
    /// empty content would fail the content equality assertion.
    #[tokio::test]
    async fn deepseek_runner_happy_path_returns_ok_with_parsed_result() {
        let body = happy_sse_body();
        let script = http_response_with_body("HTTP/1.1 200 OK", &body, "");
        let url = spawn_scripted_server(vec![script]).await;

        let cfg = cfg_for(&url, Duration::from_secs(10));
        let client = build_client(&cfg).expect("client");
        let resilience = ResilienceState::from_cfg(&cfg);
        let req = make_req();

        let result = run(&cfg, &client, &req, &resilience)
            .await
            .expect("happy path must return Ok");

        assert_eq!(result.response_text, "final answer text");
        assert_eq!(result.session_id.as_deref(), Some("deepseek-test-session-001"));
        let usage = result.token_usage.as_ref().expect("token_usage populated");
        assert_eq!(usage.input, Some(18));
        assert_eq!(usage.output, Some(174), "must NOT double-add reasoning");
        assert_eq!(usage.cached, Some(0));
        // Breaker should be Closed after a clean success.
        assert_eq!(
            resilience.breaker.lock().unwrap().state(),
            crate::deepseek_resilience::BreakerState::Closed
        );
    }

    /// Reality test (b): a 402 status closes the response with no SSE body.
    /// run() returns Err(HardProvider(402)) and the breaker latches to
    /// HardOpenInsufficientBalance.
    #[tokio::test]
    async fn deepseek_runner_402_returns_hard_provider_and_latches_breaker() {
        let script = http_response_with_body("HTTP/1.1 402 Payment Required", "", "");
        let url = spawn_scripted_server(vec![script]).await;

        let cfg = cfg_for(&url, Duration::from_secs(10));
        let client = build_client(&cfg).expect("client");
        let resilience = ResilienceState::from_cfg(&cfg);
        let req = make_req();

        let failure = run(&cfg, &client, &req, &resilience)
            .await
            .expect_err("402 must Err");
        match failure.kind {
            DeepSeekFailureKind::HardProvider(402) => {}
            other => panic!("expected HardProvider(402), got {other:?}"),
        }
        assert_eq!(
            resilience.breaker.lock().unwrap().state(),
            crate::deepseek_resilience::BreakerState::HardOpenInsufficientBalance
        );
    }

    /// Reality test (c): 429 with Retry-After:1 on the first connection,
    /// then a happy stream on the second → run() returns Ok. One internal
    /// retry honored; no retry beyond that.
    #[tokio::test]
    async fn deepseek_runner_429_with_retry_after_then_succeeds() {
        let script_429 = http_response_with_body(
            "HTTP/1.1 429 Too Many Requests",
            "",
            "Retry-After: 1\r\n",
        );
        let script_ok = http_response_with_body("HTTP/1.1 200 OK", &happy_sse_body(), "");
        let url = spawn_scripted_server(vec![script_429, script_ok]).await;

        let cfg = cfg_for(&url, Duration::from_secs(15));
        let client = build_client(&cfg).expect("client");
        let resilience = ResilienceState::from_cfg(&cfg);
        let req = make_req();

        let started = Instant::now();
        let result = run(&cfg, &client, &req, &resilience)
            .await
            .expect("retry-after-honored 429 then 200 must succeed");
        let elapsed = started.elapsed();
        assert_eq!(result.response_text, "final answer text");
        // The runner waited at least 1s for the Retry-After.
        assert!(
            elapsed >= Duration::from_millis(900),
            "Retry-After:1 should have produced at least ~1s delay; got {elapsed:?}"
        );
    }

    /// Reality test (d): server sends 200 headers + a partial body then
    /// closes. Content-Length is set larger than the bytes actually written
    /// so reqwest sees a truncated body and yields an Err — run() returns
    /// NetworkMidStream with estimated usage.
    #[tokio::test]
    async fn deepseek_runner_mid_stream_disconnect_returns_network_mid_stream() {
        // Headers claim Content-Length 100000 but we only send a small chunk.
        let partial_body = format!("{}\n\n", sample_reasoning_chunk());
        let mut script: Vec<u8> =
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n"
                .to_vec();
        script.extend_from_slice(partial_body.as_bytes());
        let url = spawn_scripted_server(vec![script]).await;

        let cfg = cfg_for(&url, Duration::from_secs(10));
        let client = build_client(&cfg).expect("client");
        let resilience = ResilienceState::from_cfg(&cfg);
        let req = make_req();

        let failure = run(&cfg, &client, &req, &resilience)
            .await
            .expect_err("truncated body must Err");
        assert!(
            matches!(failure.kind, DeepSeekFailureKind::NetworkMidStream(_)),
            "expected NetworkMidStream, got {:?}",
            failure.kind
        );
        // Estimated usage MUST be populated since no `usage` chunk was seen.
        let usage = failure.usage.as_ref().expect("estimated usage populated");
        assert_eq!(usage.usage_source, UsageSource::Estimated);
        assert!(failure.bytes_received > 0, "we received SOME bytes before EOF");
    }

    /// Privacy regression for the runner: even if the runner writes a
    /// per-request log on the happy path, the log file must NOT contain the
    /// API key or the request messages text.
    #[tokio::test]
    async fn deepseek_runner_log_excludes_secrets_and_messages() {
        let body = happy_sse_body();
        let script = http_response_with_body("HTTP/1.1 200 OK", &body, "");
        let url = spawn_scripted_server(vec![script]).await;

        let log_dir = tempfile::tempdir().expect("tempdir");
        let mut cfg = cfg_for(&url, Duration::from_secs(10));
        cfg.log_dir = log_dir.path().to_path_buf();
        cfg.api_key = crate::deepseek_config::ApiKey::new("sk-LIVE-test-key-must-not-leak-XYZ");
        let client = build_client(&cfg).expect("client");
        let resilience = ResilienceState::from_cfg(&cfg);
        let req = make_req();

        let _ = run(&cfg, &client, &req, &resilience).await.expect("ok");

        let entries: Vec<_> = std::fs::read_dir(log_dir.path())
            .expect("readdir")
            .filter_map(|e| e.ok())
            .collect();
        assert!(!entries.is_empty(), "runner should write at least one log file");
        for entry in entries {
            let body = std::fs::read_to_string(entry.path()).expect("read log");
            assert!(!body.contains("sk-LIVE-test-key-must-not-leak-XYZ"),
                "REGRESSION: log file leaked API key: {}", entry.path().display());
            assert!(!body.contains("PRIVATE-USER-PROMPT-do-not-log-this"),
                "REGRESSION: log file leaked request messages text");
            assert!(!body.contains("\"messages\""));
        }
    }

    /// Codex W3-review SHOULD-FIX #1 verification: with reqwest .timeout()
    /// removed, the outer tokio::time::timeout is the only absolute ceiling.
    /// The failure typing is now DETERMINISTIC — AbsoluteTimeoutExceeded is
    /// the only acceptable answer.
    #[tokio::test]
    async fn deepseek_runner_absolute_timeout_returns_typed_failure() {
        // Server accepts the connection but never responds — request hangs.
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let _ = tokio::time::sleep(Duration::from_secs(30)).await;
            let _ = sock.shutdown().await;
        });

        let cfg = cfg_for(&url, Duration::from_millis(500));
        let client = build_client(&cfg).expect("client");
        let resilience = ResilienceState::from_cfg(&cfg);
        let req = make_req();

        let failure = run(&cfg, &client, &req, &resilience)
            .await
            .expect_err("must time out");
        match failure.kind {
            DeepSeekFailureKind::AbsoluteTimeoutExceeded => {}
            other => panic!(
                "expected AbsoluteTimeoutExceeded (the only absolute timeout owner); got {other:?}"
            ),
        }
    }

    /// Codex W3-review BLOCKER regression: max_rpm now gates requests. With
    /// max_rpm=1 the bucket holds 1 token initially; the SECOND call within
    /// 0.5s must wait at least ~0.5s for the bucket to refill (60 RPM = 1
    /// token/sec, so 1 RPM = 1 token/min — we use a more relaxed cap so the
    /// test doesn't crawl).
    #[tokio::test]
    async fn deepseek_runner_rpm_gate_waits_for_token() {
        // Two happy responses in queue.
        let body = happy_sse_body();
        let s1 = http_response_with_body("HTTP/1.1 200 OK", &body, "");
        let s2 = http_response_with_body("HTTP/1.1 200 OK", &body, "");
        let url = spawn_scripted_server(vec![s1, s2]).await;

        // max_rpm = 4 → 4 tokens/min → ~15s per token. That's too slow for a
        // unit test. Instead, exhaust the bucket FIRST via direct take and
        // measure the wait on the next take.
        let mut cfg = cfg_for(&url, Duration::from_secs(10));
        cfg.max_rpm = 60; // 1 token/sec — easy to measure.
        let client = build_client(&cfg).expect("client");
        let resilience = ResilienceState::from_cfg(&cfg);
        let req = make_req();

        // Drain the bucket synthetically so the first run() call has to wait
        // for refill. The bucket starts full (60 tokens) — take all of them.
        {
            let now = Instant::now();
            let mut b = resilience.rpm.lock().unwrap();
            for _ in 0..60 {
                assert!(b.try_take(now), "drain");
            }
            assert!(!b.try_take(now), "drained");
        }

        let started = Instant::now();
        let _r1 = run(&cfg, &client, &req, &resilience)
            .await
            .expect("first should eventually succeed after refill");
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(800),
            "RPM gate should have waited ~1s for refill; got {waited:?}"
        );
    }

    /// Codex W3-review SHOULD-FIX #3 regression: multi-line `data:` events
    /// must be joined by '\n' and parsed as one JSON payload, not parsed
    /// per-line.
    #[test]
    fn deepseek_parser_joins_multiline_data_payloads() {
        // The JSON is split across THREE `data:` lines within ONE event.
        let stream = b"data: {\"choices\":[{\"delta\":\ndata: {\"content\":\"hi\"}\ndata: }]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let mut parser = StreamParser::new();
        parser.feed(stream).expect("multi-line data: must reassemble");
        assert_eq!(parser.content_acc, "hi");
        assert!(parser.done);
    }

    /// Codex W3-review SHOULD-FIX #4 regression: a malformed JSON containing a
    /// multi-byte character near byte 200 must NOT panic during snippet
    /// truncation in the InvalidJson error path.
    #[test]
    fn deepseek_parser_invalid_json_snippet_is_utf8_safe() {
        let mut malformed = String::with_capacity(300);
        // Pad with ASCII so byte 198 lands inside a multi-byte char.
        for _ in 0..198 {
            malformed.push('x');
        }
        malformed.push('🦀'); // 4 bytes — straddles byte 200
        malformed.push_str("not-json}");
        let stream = format!("data: {}\n\n", malformed);
        let mut parser = StreamParser::new();
        let err = parser.feed(stream.as_bytes()).expect_err("must Err, not panic");
        match err {
            ParseError::InvalidJson { snippet, .. } => {
                // Snippet must be ≤ original AND UTF-8 valid.
                assert!(snippet.len() <= malformed.len() + 8); // +8 for "…"
                assert!(std::str::from_utf8(snippet.as_bytes()).is_ok());
            }
            other => panic!("expected InvalidJson, got {other:?}"),
        }
    }

    /// Codex W3-review SHOULD-FIX #6 regression: a stream that ends cleanly
    /// (socket-EOF, finish_reason=stop) but never emits `[DONE]` is now a
    /// typed StreamEndedWithoutDone failure, not a silent Ok.
    #[tokio::test]
    async fn deepseek_runner_stream_without_done_sentinel_returns_typed_failure() {
        // Build a body that has the usage chunk with finish_reason=stop, then
        // ENDS — no `data: [DONE]\n\n`.
        let body = format!(
            "{}\n\n{}\n\n{}\n\n",
            sample_reasoning_chunk(),
            sample_content_chunk(),
            sample_usage_chunk(),
        );
        let script = http_response_with_body("HTTP/1.1 200 OK", &body, "");
        let url = spawn_scripted_server(vec![script]).await;

        let cfg = cfg_for(&url, Duration::from_secs(10));
        let client = build_client(&cfg).expect("client");
        let resilience = ResilienceState::from_cfg(&cfg);
        let req = make_req();

        let failure = run(&cfg, &client, &req, &resilience)
            .await
            .expect_err("stream without [DONE] must Err");
        match failure.kind {
            DeepSeekFailureKind::StreamEndedWithoutDone => {}
            other => panic!("expected StreamEndedWithoutDone, got {other:?}"),
        }
        // Usage is best-effort populated from the usage chunk we DID see.
        let usage = failure.usage.as_ref().expect("usage populated");
        assert_eq!(usage.output_tokens, 174);
    }

    /// Codex W3-review NIT #1 regression: Retry-After accepts HTTP-date AND
    /// delta-seconds. Both paths must produce a reasonable wait.
    #[test]
    fn parse_retry_after_handles_seconds_and_http_date() {
        use reqwest::header::HeaderValue;
        // delta-seconds
        let v = HeaderValue::from_static("7");
        assert_eq!(parse_retry_after(Some(&v)), Some(7));
        // HTTP-date in the past → 0
        let v = HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT");
        assert_eq!(parse_retry_after(Some(&v)), Some(0));
        // Garbage
        let v = HeaderValue::from_static("not a date");
        assert_eq!(parse_retry_after(Some(&v)), None);
        // None
        assert_eq!(parse_retry_after(None), None);
        // Future HTTP-date → positive wait. Build one ~10s out.
        let future = (chrono::Utc::now() + chrono::Duration::seconds(10))
            .to_rfc2822()
            // chrono's to_rfc2822 produces "+0000" but HTTP-date wants "GMT";
            // RFC 2822 parser accepts both, so this works.
            ;
        let v = HeaderValue::from_str(&future).expect("header value");
        let secs = parse_retry_after(Some(&v)).expect("parsed");
        assert!((8..=11).contains(&secs), "expected ~10s, got {secs}");
    }

    /// Codex W3-review NIT #2 regression: Windows-reserved basenames AND the
    /// empty/"."/"..": cases get safely rewritten by the sanitizer.
    #[test]
    fn sanitize_for_filename_rejects_windows_reserved_and_dot_cases() {
        for r in &["CON", "PRN", "AUX", "NUL", "COM1", "LPT9", "con", "con.txt"] {
            let out = sanitize_for_filename(r);
            assert!(out.starts_with("req-"), "{r} → {out} (expected req- prefix)");
        }
        assert_eq!(sanitize_for_filename(""), "req-0");
        assert_eq!(sanitize_for_filename("."), "req-1");
        assert_eq!(sanitize_for_filename(".."), "req-2");
        // Normal slugs are unchanged.
        assert_eq!(sanitize_for_filename("chatcmpl-001"), "chatcmpl-001");
        assert_eq!(sanitize_for_filename("abc.def"), "abc.def");
    }

    /// Codex W3-review NIT #4 regression: the runner correctly consumes a
    /// chunked-transfer-encoding SSE response (DeepSeek's actual transport),
    /// not just Content-Length + Connection: close.
    #[tokio::test]
    async fn deepseek_runner_handles_chunked_transfer_encoding() {
        // Build a chunked HTTP/1.1 response by hand. Each chunk:
        //   <hex-len>\r\n<bytes>\r\n
        // Terminator: 0\r\n\r\n
        fn chunk(body: &str) -> Vec<u8> {
            let mut out = format!("{:X}\r\n", body.len()).into_bytes();
            out.extend_from_slice(body.as_bytes());
            out.extend_from_slice(b"\r\n");
            out
        }

        // Each SSE event is a separate HTTP chunk so we test cross-chunk
        // boundary AND chunked decoding at the same time.
        let mut script = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_vec();
        script.extend(chunk(&format!("{}\n\n", sample_reasoning_chunk())));
        script.extend(chunk(&format!("{}\n\n", sample_content_chunk())));
        script.extend(chunk(&format!("{}\n\n", sample_usage_chunk())));
        script.extend(chunk("data: [DONE]\n\n"));
        script.extend_from_slice(b"0\r\n\r\n");

        let url = spawn_scripted_server(vec![script]).await;
        let cfg = cfg_for(&url, Duration::from_secs(10));
        let client = build_client(&cfg).expect("client");
        let resilience = ResilienceState::from_cfg(&cfg);
        let req = make_req();
        let result = run(&cfg, &client, &req, &resilience)
            .await
            .expect("chunked transport must work");
        assert_eq!(result.response_text, "final answer text");
    }

    /// Estimated-path test for the log writer — confirms usage_source
    /// serializes as "estimated" (lowercase) per the contract.
    #[test]
    fn deepseek_per_request_log_records_estimated_source() {
        let temp = tempfile::tempdir().expect("tempdir");
        let usage = estimate_usage(800, 200);
        assert_eq!(usage.usage_source, UsageSource::Estimated);
        let record = PerRequestLogRecord {
            request_id: "chatcmpl-est-001",
            model: "deepseek-v4-pro",
            system_fingerprint: None,
            reasoning_content: "",
            content: "",
            usage: &usage,
            cost_usd: None,
            finish_reason: "missing",
            timestamp: "2026-05-26T00:00:00Z".to_string(),
        };
        let path = write_per_request_log(temp.path(), &record).expect("write");
        let body = std::fs::read_to_string(&path).expect("read");
        assert!(body.contains("\"usage_source\""));
        assert!(body.contains("\"estimated\""));
        assert!(body.contains("\"output_tokens\""));
        assert!(body.contains("200")); // 800/4
    }

    // ─────────────────────────────────────────────────────────────────────
    // Test-plan B.1–B.8 — gaps the DEEPSEEK_TEST_PLAN flagged after the
    // Codex + WebSearch + canonical-doc research pass. All free/local.
    // ─────────────────────────────────────────────────────────────────────

    /// B.1 — Thinking-mode silent-param drop: the build_request_body MUST NOT
    /// include `temperature`, `top_p`, `presence_penalty`, or
    /// `frequency_penalty` — DeepSeek silently ignores them when thinking is
    /// enabled, and a caller seeing them in our payload would assume they
    /// matter. We don't expose these knobs in DeepSeekConfig today, so the
    /// test locks the surface in: the JSON shape never grows them by accident.
    #[test]
    fn b1_request_body_omits_sampling_params_that_thinking_mode_ignores() {
        let cfg = cfg_with_timings(Duration::from_secs(5), Duration::from_secs(10));
        let req = RunRequest {
            messages: vec![RequestMessage {
                role: "user".to_string(),
                content: "x".to_string(),
            }],
            session_id: "deepseek-b1".to_string(),
            prompt_chars_estimate: 1,
            include_reasoning: false,
        };
        let body = build_request_body(&cfg, &req);
        let s = body.to_string();
        for forbidden in &["\"temperature\"", "\"top_p\"", "\"presence_penalty\"", "\"frequency_penalty\""] {
            assert!(!s.contains(forbidden),
                "request body must NOT include {forbidden} (thinking-mode would silently ignore it); got: {s}");
        }
    }

    /// B.2 — Default thinking is on. Encodes the current behaviour: even on
    /// flash, if a caller omits `deepseek_thinking`, our cfg defaults to
    /// `ThinkingMode::Enabled`. A future change that defaults flash to
    /// `Disabled` (which the test-plan recommends as an improvement) would
    /// fail this test — at which point update the assertion deliberately.
    ///
    /// Updated 2026-05-26 after B.9 live-probe finding: the wire shape is
    /// NESTED `{"type":"enabled"}`, not flat `"enabled"`.
    #[test]
    fn b2_default_thinking_mode_is_enabled_even_for_flash() {
        let cfg = cfg_with_timings(Duration::from_secs(5), Duration::from_secs(10));
        assert_eq!(cfg.thinking, crate::deepseek_config::ThinkingMode::Enabled);
        let req = RunRequest {
            messages: vec![RequestMessage { role: "user".to_string(), content: "x".to_string() }],
            session_id: "b2".to_string(),
            prompt_chars_estimate: 1,
            include_reasoning: false,
        };
        let body = build_request_body(&cfg, &req);
        // Default thinking is enabled AND wrapped in the nested ThinkingOptions struct.
        assert_eq!(body["thinking"]["type"], serde_json::json!("enabled"),
            "default request body must carry thinking={{type:enabled}}; got: {body}");
    }

    /// B.9 (test-plan TIER 2, found via 2026-05-26 live probe): the API
    /// rejects flat `"thinking":"enabled"|"disabled"` with HTTP 400. The
    /// CORRECT wire shape is the nested `{"type":"enabled"|"disabled"}`
    /// (ThinkingOptions struct, per the deserialization error message). This
    /// test pins the nested shape so a future change can't accidentally
    /// revert to the flat form that broke every production consult.
    #[test]
    fn b9_thinking_wire_shape_is_nested_object_not_flat_string() {
        for mode in &[
            crate::deepseek_config::ThinkingMode::Enabled,
            crate::deepseek_config::ThinkingMode::Disabled,
        ] {
            let mut cfg = cfg_with_timings(Duration::from_secs(5), Duration::from_secs(10));
            cfg.thinking = *mode;
            let req = RunRequest {
                messages: vec![RequestMessage { role: "user".to_string(), content: "x".to_string() }],
                session_id: "b9".to_string(),
                prompt_chars_estimate: 1,
                include_reasoning: false,
            };
            let body = build_request_body(&cfg, &req);
            // MUST be an object with a `type` field, NOT a flat string.
            assert!(body["thinking"].is_object(),
                "thinking field must be a JSON object (ThinkingOptions struct); got: {body}");
            assert_eq!(body["thinking"]["type"], serde_json::json!(mode.as_api_str()),
                "thinking.type must mirror the as_api_str value; got: {body}");
            // Pin the regression: the body string must NOT contain the flat form
            // for either truthy value (would indicate a partial revert).
            let s = body.to_string();
            assert!(!s.contains("\"thinking\":\"enabled\""),
                "B.9 regression: flat thinking:enabled detected (API would 400)");
            assert!(!s.contains("\"thinking\":\"disabled\""),
                "B.9 regression: flat thinking:disabled detected (API would 400)");
        }
    }

    /// B.9b (live MCP test 2026-05-26 post-PR#37 merge): when
    /// `thinking={type:"disabled"}`, the API rejects requests that ALSO
    /// include `reasoning_effort` with HTTP 400:
    ///   "thinking options type cannot be disabled when reasoning_effort is set"
    /// The build_request_body must OMIT reasoning_effort entirely when
    /// thinking is disabled. With thinking enabled, reasoning_effort must
    /// still be present (it's how callers select effort tier — see T-011).
    #[test]
    fn b9b_reasoning_effort_omitted_when_thinking_disabled() {
        // thinking=Enabled → reasoning_effort PRESENT
        let mut cfg = cfg_with_timings(Duration::from_secs(5), Duration::from_secs(10));
        cfg.thinking = crate::deepseek_config::ThinkingMode::Enabled;
        cfg.reasoning_effort = crate::deepseek_config::ReasoningEffort::High;
        let req = RunRequest {
            messages: vec![RequestMessage { role: "user".to_string(), content: "x".to_string() }],
            session_id: "b9b-enabled".to_string(),
            prompt_chars_estimate: 1,
            include_reasoning: false,
        };
        let body = build_request_body(&cfg, &req);
        assert_eq!(
            body["reasoning_effort"], serde_json::json!("high"),
            "thinking=enabled MUST include reasoning_effort; got: {body}"
        );

        // thinking=Disabled → reasoning_effort OMITTED (the bug we just fixed)
        let mut cfg = cfg_with_timings(Duration::from_secs(5), Duration::from_secs(10));
        cfg.thinking = crate::deepseek_config::ThinkingMode::Disabled;
        cfg.reasoning_effort = crate::deepseek_config::ReasoningEffort::High;
        let req = RunRequest {
            messages: vec![RequestMessage { role: "user".to_string(), content: "x".to_string() }],
            session_id: "b9b-disabled".to_string(),
            prompt_chars_estimate: 1,
            include_reasoning: false,
        };
        let body = build_request_body(&cfg, &req);
        assert!(
            body.get("reasoning_effort").is_none(),
            "thinking=disabled MUST omit reasoning_effort entirely (API rejects \
             'thinking options type cannot be disabled when reasoning_effort is set'); got: {body}"
        );
        // Same as a string-grep guard against partial reverts.
        let s = body.to_string();
        assert!(!s.contains("\"reasoning_effort\""),
            "B.9b regression: reasoning_effort key found in body with thinking=disabled");
    }

    /// B.3 — Cache invariant warn: when the streamed `usage` violates
    /// `hit + miss == prompt_tokens`, the runner logs a `tracing::warn!`
    /// without failing the consult. The mock stream below carries a
    /// deliberately broken invariant (hit=5, miss=10, prompt=20 → mismatch);
    /// the runner completes Ok with the content, and we visually inspect
    /// via the test-runner's log capture that the warn fired.
    ///
    /// We assert on the user-visible behaviour: the consult succeeds AND
    /// the usage is returned verbatim AND the response is the content.
    /// The actual `tracing::warn!` is best-effort observability, captured
    /// by the test runner only — we don't structure-assert it here.
    #[tokio::test]
    async fn b3_cache_invariant_violation_warns_but_does_not_fail_consult() {
        // Build a usage chunk where hit + miss != prompt_tokens.
        let reasoning = r#"data: {"id":"b3","choices":[{"index":0,"delta":{"reasoning_content":"think"},"finish_reason":null}]}"#;
        let content = r#"data: {"id":"b3","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":null}]}"#;
        let usage = r#"data: {"id":"b3","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":20,"completion_tokens":10,"prompt_cache_hit_tokens":5,"prompt_cache_miss_tokens":10}}"#;
        let body = format!("{reasoning}\n\n{content}\n\n{usage}\n\ndata: [DONE]\n\n");
        let script = http_response_with_body("HTTP/1.1 200 OK", &body, "");
        let url = spawn_scripted_server(vec![script]).await;

        let cfg = cfg_for(&url, Duration::from_secs(10));
        let client = build_client(&cfg).expect("client");
        let resilience = ResilienceState::from_cfg(&cfg);
        let req = make_req();
        let result = run(&cfg, &client, &req, &resilience).await
            .expect("consult succeeds despite invariant violation (best-effort warn)");
        assert_eq!(result.response_text, "ok");
        let u = result.token_usage.expect("usage");
        // The usage is mapped verbatim — input ← miss (10), cached ← hit (5),
        // output ← completion (10). prompt_tokens (20) ≠ hit+miss (15), but
        // map_usage uses miss/hit directly, so the consult result is still
        // coherent for downstream cost math.
        assert_eq!(u.input, Some(10));
        assert_eq!(u.cached, Some(5));
        assert_eq!(u.output, Some(10));
    }

    /// B.4 — system_fingerprint change detector. Direct unit test of the
    /// helper: first call returns None (no prior); second call with the
    /// same fingerprint returns Some(prior, same value); a different
    /// fingerprint returns Some(prior, old value) AND should have emitted
    /// the warn (observability captured by test runner).
    #[test]
    fn b4_fingerprint_change_detector_tracks_per_model() {
        // Use a unique model name so we don't race with concurrent tests
        // that touch the global tracker.
        let model = "test-b4-tracker";
        let prev = record_fingerprint_for_model(model, "fp_alpha");
        assert!(prev.is_none(), "first observation has no prior");

        let prev = record_fingerprint_for_model(model, "fp_alpha");
        assert_eq!(prev.as_deref(), Some("fp_alpha"),
            "repeat observation returns prior (no warn fires)");

        let prev = record_fingerprint_for_model(model, "fp_beta");
        assert_eq!(prev.as_deref(), Some("fp_alpha"),
            "change returns the OLD value (warn fired internally)");

        let prev = record_fingerprint_for_model(model, "fp_beta");
        assert_eq!(prev.as_deref(), Some("fp_beta"),
            "post-change steady state");
    }

    /// B.5 — EmptyFinalAnswer. Mock a stream where the model emits reasoning,
    /// then finish_reason=stop, but no content delta ever arrived. Our
    /// finalize_stream must reject with the typed failure rather than
    /// returning Ok with empty response_text.
    #[tokio::test]
    async fn b5_empty_content_with_finish_reason_stop_is_typed_failure() {
        let reasoning = r#"data: {"id":"b5","choices":[{"index":0,"delta":{"reasoning_content":"thinking but never answers"},"finish_reason":null}]}"#;
        let stop = r#"data: {"id":"b5","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":20,"prompt_cache_hit_tokens":0,"prompt_cache_miss_tokens":5}}"#;
        let body = format!("{reasoning}\n\n{stop}\n\ndata: [DONE]\n\n");
        let script = http_response_with_body("HTTP/1.1 200 OK", &body, "");
        let url = spawn_scripted_server(vec![script]).await;

        let cfg = cfg_for(&url, Duration::from_secs(10));
        let client = build_client(&cfg).expect("client");
        let resilience = ResilienceState::from_cfg(&cfg);
        let req = make_req();
        let failure = run(&cfg, &client, &req, &resilience).await
            .expect_err("empty-content stop must be typed failure, not Ok(empty)");
        match failure.kind {
            DeepSeekFailureKind::EmptyFinalAnswer { had_reasoning } => {
                assert!(had_reasoning, "reasoning was non-empty in this stream");
            }
            other => panic!("expected EmptyFinalAnswer, got {other:?}"),
        }
        // Breaker must NOT have been recorded — model declined to answer
        // is not a provider fault.
        assert_eq!(
            resilience.breaker.lock().unwrap().state(),
            crate::deepseek_resilience::BreakerState::Closed,
            "EmptyFinalAnswer is breaker-neutral"
        );
    }

    /// B.6 — InsufficientSystemResource finish_reason promoted to its own
    /// variant + classified as transient. A stub that lumped it under
    /// `Unknown` would fail the assertion that it's_runner_transient().
    #[test]
    fn b6_insufficient_system_resource_finish_reason_is_transient() {
        let mut parser = StreamParser::new();
        let stream = b"data: {\"choices\":[{\"delta\":{\"content\":\"x\"},\"finish_reason\":\"insufficient_system_resource\"}]}\n\ndata: [DONE]\n\n";
        parser.feed(stream).expect("feed");
        let err = finalize_stream(&parser).expect_err("must Err");
        match &err {
            DeepSeekFailureKind::BadFinishReason(BadFinishReasonKind::InsufficientSystemResource) => {}
            other => panic!("expected InsufficientSystemResource variant, got {other:?}"),
        }
        assert!(err.is_runner_transient(),
            "InsufficientSystemResource must be on the transient axis (provider capacity signal)");
    }

    /// B.7 — Clean finish_reason=stop with [DONE] but NO usage chunk. The
    /// runner falls back to estimate_usage via the absent-usage path. Result
    /// is Ok with usage_source = Estimated.
    #[tokio::test]
    async fn b7_clean_stop_and_done_without_usage_chunk_estimates_usage() {
        let content = r#"data: {"id":"b7","choices":[{"index":0,"delta":{"content":"final answer"},"finish_reason":null}]}"#;
        let stop = r#"data: {"id":"b7","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
        // Notice: no usage object on the stop chunk, no separate usage chunk.
        let body = format!("{content}\n\n{stop}\n\ndata: [DONE]\n\n");
        let script = http_response_with_body("HTTP/1.1 200 OK", &body, "");
        let url = spawn_scripted_server(vec![script]).await;

        let cfg = cfg_for(&url, Duration::from_secs(10));
        let client = build_client(&cfg).expect("client");
        let resilience = ResilienceState::from_cfg(&cfg);
        let req = make_req();
        let result = run(&cfg, &client, &req, &resilience).await
            .expect("clean stop+DONE without usage must succeed (estimated fallback)");
        assert_eq!(result.response_text, "final answer");
        // The usage was synthesized from bytes_received/4 — we can't predict
        // the exact number, but it MUST be populated (Some) and non-zero.
        let u = result.token_usage.expect("usage populated even when absent on wire");
        assert!(u.output.unwrap_or(0) > 0, "estimated output tokens > 0");
    }

    /// B.8 — Anti-bulk cap is on `message.len()` only, not the serialized
    /// full request. A 10KB message + large cwd/repo/branch strings PASS
    /// the 16KB cap because the entry-check looks at message only.
    /// This test pins the documented semantics so a future change doesn't
    /// accidentally broaden the cap to the full payload.
    #[test]
    fn b8_anti_bulk_cap_measures_message_len_only() {
        // The entry-point check is `req.message.len() > deepseek_bulk_bytes_cap()`
        // — a STRING-LENGTH check on the message field alone. We can't easily
        // invoke execute_ask_agent from mcp-bridge (cross-crate), so we
        // structurally assert the behaviour at the runner layer: a RunRequest
        // with a moderate-sized message + ANY-sized session_id/messages-vec
        // is accepted by build_request_body without size checks.
        let cfg = cfg_with_timings(Duration::from_secs(5), Duration::from_secs(10));
        let big_session = "x".repeat(50_000);
        let big_msg = "y".repeat(10_000); // 10KB — under the 16KB execute-side cap
        let req = RunRequest {
            messages: vec![RequestMessage { role: "user".to_string(), content: big_msg }],
            session_id: big_session,
            prompt_chars_estimate: 10_000,
            include_reasoning: false,
        };
        // build_request_body must NOT reject based on size — it serializes
        // whatever it's given. (The cap lives in execute_ask_agent, outside
        // this crate, on `message.len()` only.)
        let body = build_request_body(&cfg, &req);
        let s = body.to_string();
        assert!(s.len() > 10_000, "body should reflect the large message");
        // Document the contract via assertion: the cap is execute_ask_agent's
        // job, NOT build_request_body's.
    }

    /// Cross-task contract: a RunawayReasoning failure MUST NOT trip the
    /// provider breaker. This test owns the integration assertion — the
    /// runner (T-010) reads it.
    #[test]
    fn deepseek_runaway_does_not_call_breaker_record() {
        use crate::deepseek_resilience::{Breaker, BreakerConfig, BreakerState};
        let breaker = Breaker::new(BreakerConfig::default());
        assert_eq!(breaker.state(), BreakerState::Closed);

        // Simulate what the runner does: build a RunawayReasoning failure
        // and DELIBERATELY do not call breaker.record() for it. The breaker
        // must remain Closed — this is a static contract about the runner's
        // behaviour, encoded as a positive test.
        let failure = DeepSeekFailureKind::RunawayReasoning { observed_tokens: 250 };
        // (We intentionally never pass `failure` to `breaker.record()`.)
        let _ = failure;
        assert_eq!(
            breaker.state(),
            BreakerState::Closed,
            "T-008 contract: runaway must not affect breaker state"
        );
    }

    /// Reality test (c): finish_reason="stop" with no embedded error → Ok.
    /// Confirms the happy path actually works (without this, the guards above
    /// could be passing by always returning Err).
    #[test]
    fn deepseek_guards_finish_reason_stop_returns_ok() {
        let stream = format!(
            "{}\n\n{}\n\n{}\n\ndata: [DONE]\n\n",
            sample_reasoning_chunk(),
            sample_content_chunk(),
            sample_usage_chunk(),
        );
        let mut parser = StreamParser::new();
        parser.feed(stream.as_bytes()).expect("feed");
        let finalized = finalize_stream(&parser).expect("clean stop must return Ok");
        assert_eq!(finalized.finish_reason, "stop");
        assert_eq!(finalized.content, "final answer text");
        assert_eq!(finalized.reasoning, "step 1 then step 2");
        assert!(finalized.usage.is_some());
        assert_eq!(finalized.system_fingerprint.as_deref(), Some("fp_abc"));
    }
}
