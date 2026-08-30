//! Grok `streaming-json` parser. REQ-GROK-005/007/011/012/018.
//!
//! The binary documents this format as "NDJSON: one ACP session update per line, the agent's
//! native format", so these event shapes are ACP session updates rather than a Grok invention.
//! That matters for later: a future `grok agent stdio` client would reuse this mapping and change
//! only the transport.
//!
//! Fixtures in `tests/fixtures/grok-streaming-*.jsonl` are REAL captures from `grok 1.0.13`, not
//! hand-written, so the tests below assert against bytes the CLI actually emitted.
//!
//! Two behaviors are load-bearing and were established by measurement, not documentation:
//!
//! 1. `input_tokens` and `cache_read_input_tokens` are SEPARATE counters. Total context is their
//!    sum. Conflating them produced a materially wrong cost measurement during investigation, so
//!    `cached` is kept distinct from `input` here.
//! 2. `thought` events are chain-of-thought and must never reach `response_text`. A one-word
//!    answer produced 33 of them in the captured fixture, so this is not a hypothetical.

use crate::types::{
    ParsedAgentResult, TokenUsage, ToolCallRecord, ToolKind, WorkingState, WorkingStateEvent,
};
use serde_json::Value;
use shared_types::AgentStreamEvent;
use tokio::sync::mpsc;

/// How the turn ended, as a FACT. Policy (is a partial answer acceptable?) belongs to the
/// runner, not here.
///
/// Codex's review of the spec caught this: the vendor guide said to fail closed on
/// `max_turns_reached` "unless `text` already has a usable answer", which is a judgment call
/// buried in a parser where it cannot be tested independently. The parser classifies; the runner
/// decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Termination {
    /// No `end` event was seen. The process died mid-turn.
    #[default]
    Incomplete,
    /// `end` seen with a normal stop reason.
    EndTurn,
    /// An `error` event was seen.
    Errored,
    /// The turn hit `--max-turns`. Whether the partial text is usable is the runner's call.
    MaxTurnsReached,
}

#[derive(Debug, Default)]
pub struct GrokStreamParser {
    session_id: Option<String>,
    response_chunks: Vec<String>,
    events: Vec<WorkingStateEvent>,
    tool_calls: Vec<ToolCallRecord>,
    token_usage: Option<TokenUsage>,
    /// Grok reports its own spend per turn. It is the only sibling that is subscription-billed
    /// AND self-reporting, so this is a usage signal rather than a bill: on a flat plan the
    /// marginal dollar cost of a call is zero. Captured because quota burn is otherwise invisible.
    total_cost_usd: Option<f64>,
    termination: Termination,
    error_detail: Option<String>,
    stream_tx: Option<mpsc::Sender<AgentStreamEvent>>,
    stream_seq: u64,
    /// Set once `end` is seen. Grok should not emit a second `end`, nor text after one, but a
    /// parser that trusts that produces duplicate TurnCompleted events and answers appended after
    /// a completed turn. Codex named all three in review.
    ended: bool,
}

/// Map a Grok tool to Triumvirate's `ToolKind`.
///
/// EXACT matching, deliberately. The first draft used substring matching and it was wrong twice:
/// `glob_file_search` matched Grep because it contains "search", and Codex named more collisions
/// waiting to happen (`thread` contains "read", `rewrite` contains "write", `research` contains
/// "search", `spreadsheet` contains "read"). `gemini.rs` uses exact names for the same reason.
///
/// An unrecognized tool is `Unknown`, which is honest. A wrong `ToolKind` is worse than no kind,
/// because the runner branches on it to pick CommandCompleted vs FileEditCompleted.
fn map_tool_kind(kind: Option<&str>, tool_name: &str) -> ToolKind {
    // The ACP `kind` field is authoritative when present.
    if let Some(k) = kind {
        match k.to_lowercase().as_str() {
            "read" => return ToolKind::ReadFile,
            "write" => return ToolKind::WriteFile,
            "edit" => return ToolKind::EditFile,
            "execute" => return ToolKind::Bash,
            "search" => return ToolKind::Grep,
            _ => {}
        }
    }
    // Grok's native tool names, verified from `available_commands` in the captured fixture.
    match tool_name.to_lowercase().as_str() {
        "read_file" | "read" => ToolKind::ReadFile,
        "write" | "write_file" => ToolKind::WriteFile,
        "search_replace" | "edit" | "edit_file" | "apply_patch" | "str_replace" => ToolKind::EditFile,
        "run_terminal_command" | "bash" | "shell" | "terminal" => ToolKind::Bash,
        "grep" => ToolKind::Grep,
        "glob" | "glob_file_search" | "list_dir" => ToolKind::Glob,
        "ask_user_question" | "request_user_input" | "ask" => ToolKind::RequestUserInput,
        _ => ToolKind::Unknown,
    }
}

fn u64_at(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

/// Grok's usage block maps onto `TokenUsage`. `cache_read_input_tokens` becomes `cached` and is
/// deliberately NOT added into `input`: they are separate counters and the sum is total context.
fn parse_usage(usage: &Value, tool_calls: u64) -> TokenUsage {
    TokenUsage {
        input: u64_at(usage, "input_tokens"),
        output: u64_at(usage, "output_tokens"),
        cached: u64_at(usage, "cache_read_input_tokens"),
        thinking_tokens: u64_at(usage, "reasoning_tokens"),
        latency_ms: None,
        tool_calls: Some(tool_calls),
        total: u64_at(usage, "total_tokens"),
    }
}

impl GrokStreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_stream_channel(tx: mpsc::Sender<AgentStreamEvent>) -> Self {
        Self {
            stream_tx: Some(tx),
            ..Self::default()
        }
    }

    /// How the turn ended. Read this BEFORE `finish`, which consumes the parser.
    pub fn termination(&self) -> Termination {
        self.termination
    }

    /// Grok's self-reported spend for the turn, if an `end` or `usage` event carried it.
    pub fn total_cost_usd(&self) -> Option<f64> {
        self.total_cost_usd
    }

    /// Detail from an `error` event, for operator-facing classification.
    pub fn error_detail(&self) -> Option<&str> {
        self.error_detail.as_deref()
    }

    fn emit_stream_event(&mut self, event: AgentStreamEvent) {
        if let Some(tx) = &self.stream_tx {
            // Best effort. Dropping a display event must never stall parsing.
            let _ = tx.try_send(event);
        }
    }

    fn next_seq(&mut self) -> u64 {
        self.stream_seq += 1;
        self.stream_seq
    }

    fn event(&self, state: WorkingState, detail: impl Into<String>) -> WorkingStateEvent {
        WorkingStateEvent {
            agent: "grok".to_string(),
            state,
            detail: detail.into(),
            tool_name: None,
            tool_args_json: None,
            token_usage: None,
            ts_ms: None,
        }
    }

    fn record(&mut self, ev: WorkingStateEvent) -> Option<WorkingStateEvent> {
        self.events.push(ev.clone());
        Some(ev)
    }

    /// Parse one NDJSON line. Returns a display event when the line produced one.
    ///
    /// A line that is not JSON is IGNORED rather than fatal: the CLI may print a banner or an
    /// update notice, and killing the turn over that would be a self-inflicted outage.
    pub fn parse_line(&mut self, line: &str) -> Option<WorkingStateEvent> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let json: Value = serde_json::from_str(line).ok()?;
        let event_type = json.get("type").and_then(Value::as_str).unwrap_or_default();

        match event_type {
            // REQ-GROK-011. Chain of thought. NEVER appended to response_text.
            "thought" => {
                let ev = self.event(WorkingState::MessageDelta, "thinking");
                self.record(ev)
            }

            "text" => {
                let data = json.get("data").and_then(Value::as_str).unwrap_or_default();
                if data.is_empty() || self.ended {
                    // Text after `end` is protocol-invalid. Appending it would mutate an answer
                    // the runner may already have reported.
                    return None;
                }
                self.response_chunks.push(data.to_string());
                let preview: String = data.chars().take(120).collect();
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::ResponseChunk {
                    agent: "grok".to_string(),
                    text_preview: preview.clone(),
                    seq,
                });
                let ev = self.event(WorkingState::MessageDelta, preview);
                self.record(ev)
            }

            "tool_call" => {
                let tool = json
                    .get("toolName")
                    .and_then(Value::as_str)
                    .or_else(|| json.get("title").and_then(Value::as_str))
                    .unwrap_or("unknown")
                    .to_string();
                let kind = map_tool_kind(json.get("kind").and_then(Value::as_str), &tool);
                let args_json = json.get("rawInput").map(|v| v.to_string());

                self.tool_calls.push(ToolCallRecord {
                    id: json.get("toolCallId").and_then(Value::as_str).map(str::to_string),
                    tool: tool.clone(),
                    kind: kind.clone(),
                    success: None,
                    duration_ms: None,
                    args_json: args_json.clone(),
                });

                let seq = self.next_seq();
                if kind == ToolKind::ReadFile {
                    let path = json
                        .get("rawInput")
                        .and_then(|v| v.get("path"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    self.emit_stream_event(AgentStreamEvent::FileRead {
                        agent: "grok".to_string(),
                        file_path: path,
                        seq,
                    });
                } else {
                    self.emit_stream_event(AgentStreamEvent::ToolCall {
                        agent: "grok".to_string(),
                        tool_name: tool.clone(),
                        args_summary: args_json.clone().unwrap_or_default().chars().take(120).collect(),
                        seq,
                    });
                }

                let mut ev = self.event(WorkingState::ToolCallStarted, tool.clone());
                ev.tool_name = Some(tool);
                ev.tool_args_json = args_json;
                self.record(ev)
            }

            "tool_call_update" => {
                let id = json.get("toolCallId").and_then(Value::as_str);
                let completed =
                    json.get("status").and_then(Value::as_str) == Some("completed");
                // Match on id so out-of-order updates attach to the right call.
                // Prefer the LAST still-open call with this id. Grok should not reuse ids, but
                // if it does, `find` would attach every update to the first one and leave the
                // rest permanently un-completed. Codex flagged this in review.
                let target = id.and_then(|i| {
                    self.tool_calls
                        .iter()
                        .rposition(|r| r.id.as_deref() == Some(i) && r.success.is_none())
                        .or_else(|| self.tool_calls.iter().rposition(|r| r.id.as_deref() == Some(i)))
                });
                if let Some(idx) = target {
                    self.tool_calls[idx].success = Some(completed);
                }
                let kind = target
                    .map(|i| self.tool_calls[i].kind.clone())
                    .unwrap_or(ToolKind::Unknown);
                let state = match kind {
                    ToolKind::Bash => WorkingState::CommandCompleted,
                    ToolKind::EditFile | ToolKind::WriteFile => WorkingState::FileEditCompleted,
                    _ => WorkingState::ToolCallCompleted,
                };
                let ev = self.event(state, if completed { "completed" } else { "failed" });
                self.record(ev)
            }

            "usage" => {
                let n = self.tool_calls.len() as u64;
                if let Some(u) = json.get("usage") {
                    let usage = parse_usage(u, n);
                    // If `end` already fired, its TurnCompleted event is carrying stale usage.
                    // Backfill it rather than leaving the recorded event wrong.
                    if self.ended
                        && let Some(ev) = self
                            .events
                            .iter_mut()
                            .rev()
                            .find(|e| e.state == WorkingState::TurnCompleted)
                    {
                        ev.token_usage = Some(usage.clone());
                    }
                    self.token_usage = Some(usage);
                }
                if let Some(c) = json.get("total_cost_usd").and_then(Value::as_f64) {
                    self.total_cost_usd = Some(c);
                }
                None
            }

            "end" => {
                if self.ended {
                    // Idempotent: a second `end` must not emit a second TurnCompleted.
                    return None;
                }
                self.ended = true;
                // REQ-GROK-007: the parser's sessionId is the source of truth, even when it
                // differs from the id we requested.
                if let Some(sid) = json.get("sessionId").and_then(Value::as_str) {
                    self.session_id = Some(sid.to_string());
                }
                let n = self.tool_calls.len() as u64;
                if let Some(u) = json.get("usage") {
                    self.token_usage = Some(parse_usage(u, n));
                }
                if let Some(c) = json.get("total_cost_usd").and_then(Value::as_f64) {
                    self.total_cost_usd = Some(c);
                }
                if self.termination == Termination::Incomplete {
                    self.termination = Termination::EndTurn;
                }
                let detail = json
                    .get("stopReason")
                    .and_then(Value::as_str)
                    .unwrap_or("end_turn")
                    .to_string();
                let mut ev = self.event(WorkingState::TurnCompleted, detail);
                ev.token_usage = self.token_usage.clone();
                self.record(ev)
            }

            "error" => {
                let msg = json
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("grok reported an error")
                    .to_string();
                self.termination = Termination::Errored;
                self.error_detail = Some(msg.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::Error {
                    agent: "grok".to_string(),
                    message: msg.clone(),
                    seq,
                });
                let ev = self.event(WorkingState::Error, msg);
                self.record(ev)
            }

            "max_turns_reached" => {
                // Classified as a fact. The runner decides whether partial text is acceptable.
                self.termination = Termination::MaxTurnsReached;
                let ev = self.event(WorkingState::Stuck, "max_turns_reached");
                self.record(ev)
            }

            // Skipped by design: plan, available_commands, auto_compact_*. Unknown types fall
            // here too, so an upstream addition is inert rather than fatal.
            _ => None,
        }
    }

    /// Parse the one-shot `--output-format json` payload, used when streaming is disabled.
    pub fn parse_batch_json(value: &Value) -> ParsedAgentResult {
        let usage = value.get("usage").map(|u| parse_usage(u, 0));
        ParsedAgentResult {
            response_text: value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            session_id: value.get("sessionId").and_then(Value::as_str).map(str::to_string),
            events: Vec::new(),
            tool_calls: Vec::new(),
            token_usage: usage,
            cli_version: None,
            parser_mode: "grok-batch-json".to_string(),
        }
    }

    /// Everything the runner needs, in ONE value.
    ///
    /// The first draft exposed `termination()` / `total_cost_usd()` as getters that had to be read
    /// BEFORE `finish` consumed the parser. Codex correctly called that a trap: `agent_exec.rs`
    /// calls every sibling parser as a bare `let parsed = parser.finish();`, so those facts would
    /// simply never be read. Do not require call ordering to get correct behavior.
    pub fn finish_full(self) -> GrokParsed {
        GrokParsed {
            termination: self.termination,
            total_cost_usd: self.total_cost_usd,
            error_detail: self.error_detail.clone(),
            parsed: self.finish(),
        }
    }

    pub fn finish(self) -> ParsedAgentResult {
        ParsedAgentResult {
            response_text: self.response_chunks.concat(),
            session_id: self.session_id,
            events: self.events,
            tool_calls: self.tool_calls,
            token_usage: self.token_usage,
            cli_version: None,
            parser_mode: "grok-streaming-json".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REAL capture from `grok 1.0.13`, not hand-written. 37 lines: available_commands, 33
    /// thought events, one text event, usage, end.
    const FIXTURE: &str = include_str!("../tests/fixtures/grok-streaming-20260830.jsonl");

    fn parse_fixture(src: &str) -> (GrokParsed, ParsedAgentResult) {
        let mut p = GrokStreamParser::new();
        for line in src.lines() {
            let _ = p.parse_line(line);
        }
        let full = p.finish_full();
        let parsed = full.parsed.clone();
        (full, parsed)
    }

    // ---- U-C-01: happy path against real bytes ----
    #[test]
    fn u_c_01_real_fixture_yields_text_session_and_tokens() {
        let (_, r) = parse_fixture(FIXTURE);
        assert_eq!(r.response_text, "pong", "the only text event in the capture");
        assert_eq!(r.session_id.as_deref(), Some("CD94C2BD-530A-48E7-8EA4-91D7853CE6B0"));
        let u = r.token_usage.expect("usage must be captured");
        assert_eq!(u.input, Some(66559));
        assert_eq!(u.output, Some(39));
        assert_eq!(u.total, Some(67110));
    }

    // ---- U-C-02: REQ-GROK-011, the one that would leak CoT to an operator ----
    #[test]
    fn u_c_02_thoughts_never_reach_response_text() {
        let (_, r) = parse_fixture(FIXTURE);
        let thoughts = FIXTURE.lines().filter(|l| l.contains("\"type\":\"thought\"")).count();
        assert!(thoughts >= 30, "fixture must actually contain thoughts, found {thoughts}");
        assert_eq!(r.response_text, "pong", "33 thought events must contribute zero characters");
        assert!(!r.response_text.contains("Analyz"), "no chain-of-thought prose may leak");
    }

    // ---- U-C-03: REQ-GROK-007 ----
    #[test]
    fn u_c_03_parser_session_id_wins_over_the_requested_one() {
        let mut p = GrokStreamParser::new();
        p.parse_line(r#"{"type":"end","stopReason":"end_turn","sessionId":"server-chosen"}"#);
        assert_eq!(p.finish().session_id.as_deref(), Some("server-chosen"));
    }

    // ---- U-C-04 / U-C-05: token map, and the separate-counters rule ----
    #[test]
    fn u_c_05_cache_read_is_separate_from_input_not_folded_in() {
        let mut p = GrokStreamParser::new();
        p.parse_line(r#"{"type":"end","usage":{"input_tokens":1323,"output_tokens":31,"cache_read_input_tokens":11648,"reasoning_tokens":7,"total_tokens":13002}}"#);
        let u = p.finish().token_usage.unwrap();
        assert_eq!(u.input, Some(1323), "input must NOT absorb the cached count");
        assert_eq!(u.cached, Some(11648));
        assert_eq!(u.thinking_tokens, Some(7), "reasoning_tokens maps to thinking_tokens");
        // Total context is the SUM. Conflating these produced a wrong measurement in testing.
        assert_eq!(u.input.unwrap() + u.cached.unwrap(), 12971);
    }

    // ---- U-C-06: cost, captured because quota burn is otherwise invisible ----
    #[test]
    fn u_c_06_self_reported_cost_is_captured() {
        let mut p = GrokStreamParser::new();
        for l in FIXTURE.lines() { let _ = p.parse_line(l); }
        assert_eq!(p.finish_full().total_cost_usd, Some(0.02271336));
    }

    // ---- U-C-07 / U-C-08: hostile and unknown input must not kill a turn ----
    #[test]
    fn u_c_07_non_json_lines_are_ignored_not_fatal() {
        let mut p = GrokStreamParser::new();
        assert!(p.parse_line("Grok 1.0.13 starting up").is_none());
        assert!(p.parse_line("").is_none());
        assert!(p.parse_line("{not json").is_none());
        p.parse_line(r#"{"type":"text","data":"ok"}"#);
        assert_eq!(p.finish().response_text, "ok", "a banner must not lose the answer");
    }

    #[test]
    fn u_c_08_unknown_event_type_is_inert() {
        let mut p = GrokStreamParser::new();
        assert!(p.parse_line(r#"{"type":"auto_compact_start"}"#).is_none());
        assert!(p.parse_line(r#"{"type":"some_future_event","data":"x"}"#).is_none());
        assert!(p.parse_line(r#"{"type":"plan","entries":[]}"#).is_none());
        p.parse_line(r#"{"type":"text","data":"still here"}"#);
        assert_eq!(p.finish().response_text, "still here");
    }

    // ---- U-C-09 / U-C-10: failure classification ----
    #[test]
    fn u_c_09_error_event_is_recorded_as_error() {
        let mut p = GrokStreamParser::new();
        p.parse_line(r#"{"type":"error","message":"auth failed"}"#);
        assert_eq!(p.termination(), Termination::Errored);
        assert_eq!(p.error_detail(), Some("auth failed"));
        assert!(p.finish().events.iter().any(|e| e.state == WorkingState::Error));
    }

    /// The parser classifies; the runner decides. Codex flagged that burying "unless the text is
    /// usable" in the parser makes the policy untestable.
    #[test]
    fn u_c_10_max_turns_is_a_fact_not_a_policy_decision() {
        let mut p = GrokStreamParser::new();
        p.parse_line(r#"{"type":"text","data":"partial answer"}"#);
        p.parse_line(r#"{"type":"max_turns_reached"}"#);
        assert_eq!(p.termination(), Termination::MaxTurnsReached);
        let r = p.finish();
        // The partial text is PRESERVED and handed up. The parser does not silently discard it,
        // and does not silently bless it either.
        assert_eq!(r.response_text, "partial answer");
    }

    #[test]
    fn u_c_10b_no_end_event_means_incomplete() {
        let mut p = GrokStreamParser::new();
        p.parse_line(r#"{"type":"text","data":"cut off"}"#);
        assert_eq!(p.termination(), Termination::Incomplete, "a died-mid-turn process is not a success");
    }

    // ---- U-C-11: tool calls ----
    #[test]
    fn u_c_11_tool_call_name_falls_back_title_then_unknown() {
        let mut p = GrokStreamParser::new();
        p.parse_line(r#"{"type":"tool_call","toolCallId":"c1","toolName":"read_file","kind":"read","rawInput":{"path":"src/main.rs"}}"#);
        p.parse_line(r#"{"type":"tool_call","toolCallId":"c2","title":"Read"}"#);
        p.parse_line(r#"{"type":"tool_call","toolCallId":"c3"}"#);
        p.parse_line(r#"{"type":"tool_call_update","toolCallId":"c1","status":"completed"}"#);
        p.parse_line(r#"{"type":"tool_call_update","toolCallId":"c2","status":"failed"}"#);
        let r = p.finish();
        assert_eq!(r.tool_calls.len(), 3);
        assert_eq!(r.tool_calls[0].tool, "read_file");
        assert_eq!(r.tool_calls[0].kind, ToolKind::ReadFile);
        assert_eq!(r.tool_calls[0].success, Some(true));
        assert_eq!(r.tool_calls[1].tool, "Read", "falls back to title");
        assert_eq!(r.tool_calls[1].success, Some(false), "status != completed is a failure");
        assert_eq!(r.tool_calls[2].tool, "unknown", "never panic on a nameless call");
    }

    #[test]
    fn u_c_11b_out_of_order_updates_attach_to_the_right_call() {
        let mut p = GrokStreamParser::new();
        p.parse_line(r#"{"type":"tool_call","toolCallId":"a","toolName":"grep"}"#);
        p.parse_line(r#"{"type":"tool_call","toolCallId":"b","toolName":"bash"}"#);
        // b completes before a.
        p.parse_line(r#"{"type":"tool_call_update","toolCallId":"b","status":"completed"}"#);
        p.parse_line(r#"{"type":"tool_call_update","toolCallId":"a","status":"failed"}"#);
        let r = p.finish();
        assert_eq!(r.tool_calls[0].success, Some(false), "a failed");
        assert_eq!(r.tool_calls[1].success, Some(true), "b completed");
    }

    #[test]
    fn u_c_11c_tool_kind_mapping() {
        for (kind, name, want) in [
            (Some("read"), "read_file", ToolKind::ReadFile),
            (None, "write_file", ToolKind::WriteFile),
            (None, "search_replace", ToolKind::EditFile),
            (None, "run_terminal_command", ToolKind::Bash),
            (None, "grep", ToolKind::Grep),
            (None, "glob_file_search", ToolKind::Glob),
            (None, "ask_user_question", ToolKind::RequestUserInput),
            (None, "image_gen", ToolKind::Unknown),
        ] {
            assert_eq!(map_tool_kind(kind, name), want, "{name}");
        }
    }

    // ---- U-C-12 / U-C-13 ----
    #[test]
    fn u_c_12_parser_mode_is_declared() {
        let (_, r) = parse_fixture(FIXTURE);
        assert_eq!(r.parser_mode, "grok-streaming-json");
    }

    #[test]
    fn u_c_13_batch_json_fallback_reads_text_session_and_usage() {
        let v: Value = serde_json::from_str(
            r#"{"text":"batch answer","sessionId":"s-9","usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}"#,
        ).unwrap();
        let r = GrokStreamParser::parse_batch_json(&v);
        assert_eq!(r.response_text, "batch answer");
        assert_eq!(r.session_id.as_deref(), Some("s-9"));
        assert_eq!(r.token_usage.unwrap().total, Some(12));
        assert_eq!(r.parser_mode, "grok-batch-json");
    }

    /// Every event carries the CANONICAL key, never an alias and never the display name.
    #[test]
    fn u_c_14_events_are_tagged_with_the_canonical_agent_key() {
        let (_, r) = parse_fixture(FIXTURE);
        assert!(!r.events.is_empty());
        assert!(r.events.iter().all(|e| e.agent == "grok"), "never \"Grok\" or \"supergrok\"");
    }

    // ---- Defects Codex found in review of the first draft ----

    /// Text arriving after `end` would mutate an answer the runner may already have reported.
    #[test]
    fn u_c_15_text_after_end_is_rejected() {
        let mut p = GrokStreamParser::new();
        p.parse_line(r#"{"type":"text","data":"real answer"}"#);
        p.parse_line(r#"{"type":"end","sessionId":"s1","stopReason":"end_turn"}"#);
        p.parse_line(r#"{"type":"text","data":" INJECTED"}"#);
        assert_eq!(p.finish().response_text, "real answer");
    }

    /// A second `end` must not emit a second TurnCompleted.
    #[test]
    fn u_c_16_end_is_idempotent() {
        let mut p = GrokStreamParser::new();
        p.parse_line(r#"{"type":"end","sessionId":"first","stopReason":"end_turn"}"#);
        p.parse_line(r#"{"type":"end","sessionId":"second","stopReason":"end_turn"}"#);
        let r = p.finish();
        assert_eq!(r.session_id.as_deref(), Some("first"), "the first end wins");
        assert_eq!(r.events.iter().filter(|e| e.state == WorkingState::TurnCompleted).count(), 1);
    }

    /// `usage` after `end` left the recorded TurnCompleted carrying stale usage.
    #[test]
    fn u_c_17_usage_after_end_backfills_the_completion_event() {
        let mut p = GrokStreamParser::new();
        p.parse_line(r#"{"type":"end","sessionId":"s","stopReason":"end_turn"}"#);
        p.parse_line(r#"{"type":"usage","usage":{"input_tokens":900,"output_tokens":5,"total_tokens":905}}"#);
        let r = p.finish();
        let done = r.events.iter().rev().find(|e| e.state == WorkingState::TurnCompleted).unwrap();
        assert_eq!(done.token_usage.as_ref().unwrap().input, Some(900),
            "the completion event must not report stale usage");
        assert_eq!(r.token_usage.unwrap().total, Some(905));
    }

    /// A reused toolCallId attached every update to the FIRST call, leaving later ones
    /// permanently un-completed.
    #[test]
    fn u_c_18_duplicate_tool_call_ids_do_not_starve_later_calls() {
        let mut p = GrokStreamParser::new();
        p.parse_line(r#"{"type":"tool_call","toolCallId":"dup","toolName":"grep"}"#);
        p.parse_line(r#"{"type":"tool_call","toolCallId":"dup","toolName":"grep"}"#);
        p.parse_line(r#"{"type":"tool_call_update","toolCallId":"dup","status":"completed"}"#);
        p.parse_line(r#"{"type":"tool_call_update","toolCallId":"dup","status":"completed"}"#);
        let r = p.finish();
        assert!(r.tool_calls.iter().all(|c| c.success == Some(true)),
            "both calls sharing an id must resolve, not just the first");
    }

    /// Exact matching, after substring matching was wrong twice.
    #[test]
    fn u_c_19_tool_names_that_merely_contain_a_keyword_are_not_miscategorized() {
        for name in ["thread_create", "rewrite_history", "research_topic", "spreadsheet_open", "task_list"] {
            assert_eq!(map_tool_kind(None, name), ToolKind::Unknown,
                "{name} must not be guessed from a substring");
        }
        // The ACP `kind` field still wins when present.
        assert_eq!(map_tool_kind(Some("read"), "totally_unknown_tool"), ToolKind::ReadFile);
    }

}

/// Grok's parse result plus the turn facts the runner must branch on. Returned by
/// `finish_full` so no caller can get correct behavior wrong by reading fields in the wrong
/// order.
#[derive(Debug, Clone)]
pub struct GrokParsed {
    pub parsed: ParsedAgentResult,
    /// How the turn ended. The runner owns the policy question of whether a partial answer from
    /// `MaxTurnsReached` is acceptable.
    pub termination: Termination,
    /// Grok's self-reported spend. A usage signal on a flat subscription, not a bill.
    pub total_cost_usd: Option<f64>,
    pub error_detail: Option<String>,
}
