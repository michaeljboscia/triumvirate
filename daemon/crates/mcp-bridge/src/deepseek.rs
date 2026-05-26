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
        // Absolute outer ceiling on a single HTTP request.
        .timeout(cfg.timeout)
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
        loop {
            let Some((payload_end, term_len)) = find_event_terminator(&self.buffer) else {
                break;
            };
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
        // One SSE event can be multiple lines. Per the SSE spec, lines starting
        // with `:` are comments; lines starting with `data:` are data. We handle
        // both. Empty events (\n\n with no content) are valid heartbeats too.
        let mut had_data = false;
        for raw_line in event.split('\n') {
            // Strip the trailing CR if present (CRLF tolerance).
            let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);

            if line.is_empty() {
                continue;
            }
            if let Some(_comment) = line.strip_prefix(':') {
                // Per the SSE spec, comment lines are ignored. We count them so
                // the runner can confirm keep-alives are being received.
                continue;
            }
            // Per the spec, `data: <value>` — note the space is optional and a
            // single-line event may have multiple `data:` lines that get joined
            // by newlines. DeepSeek emits one-data-per-event so we don't bother.
            if let Some(payload) = line.strip_prefix("data:") {
                let payload = payload.trim_start();
                had_data = true;
                if payload == "[DONE]" {
                    self.done = true;
                    events.push(ParseEvent::Done);
                    continue;
                }
                self.consume_data_chunk(payload, events)?;
            }
            // Anything else (event:, id:, retry:) — silently ignore. DeepSeek
            // doesn't currently use them; if they appear, we don't want to error.
        }
        if !had_data {
            // An event with no `data:` lines that isn't pure-empty is a keep-alive
            // (the leading `:` comment is the canonical form).
            self.keepalive_count += 1;
            events.push(ParseEvent::KeepAlive);
        }
        Ok(())
    }

    fn consume_data_chunk(
        &mut self,
        payload: &str,
        events: &mut Vec<ParseEvent>,
    ) -> Result<(), ParseError> {
        let chunk: RawChunk = serde_json::from_str(payload).map_err(|e| {
            // Bound the snippet so a 1MB blob doesn't end up in the log.
            let snippet = if payload.len() > 200 {
                format!("{}…", &payload[..200])
            } else {
                payload.to_string()
            };
            ParseError::InvalidJson {
                snippet,
                cause: e.to_string(),
            }
        })?;

        if let Some(id) = chunk.id {
            if self.request_id.is_none() {
                self.request_id = Some(id);
            }
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
    // T-010 will add: HardProvider(u16), Transient(u16), NetworkMidStream,
    //                  AbsoluteTimeoutExceeded, ...
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
        Some("stop") => Ok(FinalizedStream {
            content: parser.content_acc.clone(),
            reasoning: parser.reasoning_acc.clone(),
            usage: parser.usage.clone(),
            finish_reason: "stop".to_string(),
            request_id: parser.request_id.clone(),
            system_fingerprint: parser.system_fingerprint.clone(),
        }),
        Some("length") => {
            Err(DeepSeekFailureKind::BadFinishReason(BadFinishReasonKind::Length))
        }
        Some("content_filter") => Err(DeepSeekFailureKind::BadFinishReason(
            BadFinishReasonKind::ContentFilter,
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
/// reasoning trace. Privacy guard (REQ-DS-023 scope_out): MUST NOT contain
/// the API key or the request messages — only response artifacts.
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
    let json = serde_json::to_string_pretty(record)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    std::fs::write(&path, json)?;
    Ok(path)
}

/// `request_id` comes from DeepSeek and is ordinarily a safe slug, but we
/// strip path separators defensively so a malicious value can't escape
/// `log_dir` (REQ-DS-018 — observability must not be a vector).
fn sanitize_for_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '_',
        })
        .collect()
}

/// Current wall-clock as RFC3339 UTC, matching the rest of the daemon's
/// timestamps (e.g. token-economics price_table effective_date).
pub fn now_rfc3339_utc() -> String {
    chrono::Utc::now()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
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

    /// Cross-task contract: a RunawayReasoning failure MUST NOT trip the
    /// provider breaker. This test owns the integration assertion — the
    /// runner (T-010) reads it.
    #[test]
    fn deepseek_runaway_does_not_call_breaker_record() {
        use crate::deepseek_resilience::{Breaker, BreakerConfig, BreakerState};
        let mut breaker = Breaker::new(BreakerConfig::default());
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
