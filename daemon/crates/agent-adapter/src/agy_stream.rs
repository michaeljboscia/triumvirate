//! agy `--output-format stream-json` parser.
//!
//! WHY THIS EXISTS. Until 2026-09-01 agy was dispatched in plain-text mode and its
//! `ParsedAgentResult` hardcoded `tool_calls: Vec::new()`, because plain text carries no tool
//! events to record. That made Antigravity structurally invisible to the sight gate: a review
//! dispatched to it was rejected 100% of the time, however carefully it looked, since a gate
//! that cannot see tool calls cannot tell "did not look" from "cannot report".
//!
//! `agy-integration-spec.md` REQ-012 forbade `--output-format`, and REQ-060 recorded the
//! reason: verified 2026-05-24 against agy **v1.0.1**, which had no such flag. The installed
//! binary is now **v1.1.23** and the flag exists with a `stream-json` value. The prohibition
//! was correct when written and is now stale. Verified live on 2026-09-01, fixtures captured
//! in `tests/fixtures/agy-stream-*.jsonl`.
//!
//! THE WIRE SHAPE, from those captures and not from documentation:
//!
//! ```text
//! {"event":"init","conversation_id":"...","init":{"cwd":"...","tools":[...],...}}
//! {"event":"step_update","step_update":{"step_index":0,"state":"DONE","step_type":"user_input"}}
//! {"event":"step_update","step_update":{"step_index":1,"state":"DONE",
//!    "step_type":"agent_response","text_delta":"pong\n","usage":{...}}}
//! {"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"tool",
//!    "tool_name":"run_command","tool_info":{"name":"run_command",
//!    "parameters":{"CommandLine":"pwd"}}}}
//! {"event":"step_update","step_update":{"step_index":2,"state":"DONE","step_type":"tool",
//!    "tool_name":"run_command","duration_seconds":0.03,
//!    "tool_info":{...,"output":"/Users/...\n"}}}
//! {"event":"result","result":{"status":"SUCCESS","response":"pong\n","num_turns":1,
//!    "usage":{...}}}
//! ```
//!
//! This is strictly more information than the plain-text path gave: the final answer arrives
//! as a single `result.response` instead of being scraped out of ANSI-stripped terminal
//! output, token counts come from the stream instead of the log file, and tool calls exist
//! at all.

use serde_json::Value;

use crate::types::{
    ParsedAgentResult, TokenUsage, ToolCallRecord, ToolKind, WorkingState, WorkingStateEvent,
};

/// The parser mode this emits. On the sight gate's allowlist, unlike the plain-text modes.
pub const PARSER_MODE_STREAM: &str = "agy-stream-json";

/// A step index, used to pair an `ACTIVE` tool step with its later `DONE` update.
type StepIndex = i64;


/// Build a lifecycle event with the fields this parser can fill. Matches the shape the grok
/// parser uses so downstream consumers see one event type, not two dialects.
fn event(
    state: WorkingState,
    detail: impl Into<String>,
    tool_name: Option<String>,
    tool_args_json: Option<String>,
    token_usage: Option<TokenUsage>,
) -> WorkingStateEvent {
    WorkingStateEvent {
        agent: "gemini".to_string(),
        state,
        detail: detail.into(),
        tool_name,
        tool_args_json,
        token_usage,
        ts_ms: None,
    }
}

#[derive(Debug, Default)]
pub struct AgyStreamParser {
    text_deltas: Vec<String>,
    final_response: Option<String>,
    conversation_id: Option<String>,
    tool_calls: Vec<ToolCallRecord>,
    /// Position in `tool_calls` for each open step index, so a `DONE` update lands on the call
    /// it belongs to rather than the most recent one. agy interleaves steps.
    open_steps: Vec<(StepIndex, usize)>,
    usage: Option<TokenUsage>,
    events: Vec<WorkingStateEvent>,
    status: Option<String>,
    saw_result: bool,
}

/// Map an agy tool name to a kind.
///
/// Names come from the `init` event's own `tools` array, captured live. Anything unrecognised
/// is `Unknown`, deliberately: `Unknown` is not read-shaped, so an unmapped tool cannot
/// satisfy a `required_sources` check by accident. Guessing generously here would open the
/// exact hole the sight gate exists to close.
pub fn map_agy_tool_kind(name: &str) -> ToolKind {
    match name {
        "view_file" | "read_url_content" | "read_resource" | "read_browser_page"
        | "notebook_execution" => ToolKind::ReadFile,
        "write_to_file" | "generate_image" => ToolKind::WriteFile,
        "replace_file_content" | "multi_replace_file_content" | "sed_file" | "notebook_edit" => {
            ToolKind::EditFile
        }
        "run_command" | "command_status" | "send_command_input" => ToolKind::Bash,
        "grep_search" | "search_web" => ToolKind::Grep,
        "list_dir" | "find_by_name" | "list_resources" => ToolKind::Glob,
        "ask_question" | "ask_permission" | "ask_custom_permission" => ToolKind::RequestUserInput,
        _ => ToolKind::Unknown,
    }
}

fn parse_usage(u: &Value) -> TokenUsage {
    let g = |k: &str| u.get(k).and_then(Value::as_u64);
    TokenUsage {
        input: g("input_tokens"),
        output: g("output_tokens"),
        // agy reports cache reads separately. Kept out of `input` so a cached turn is not
        // double counted, matching how the grok parser treats its own cache fields.
        cached: g("cache_read_tokens"),
        thinking_tokens: g("thinking_tokens"),
        latency_ms: None,
        tool_calls: None,
        total: g("total_tokens"),
    }
}

impl AgyStreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one NDJSON line. Unparseable lines are ignored rather than fatal: agy may print
    /// a warning to stdout, and one stray line must not lose an entire turn.
    pub fn parse_line(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match v.get("event").and_then(Value::as_str) {
            Some("init") => {
                self.conversation_id = v
                    .get("conversation_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                self.events.push(event(
                    WorkingState::TurnStarted,
                    "turn started (agy stream)",
                    None,
                    None,
                    None,
                ));
            }
            Some("step_update") => {
                if let Some(su) = v.get("step_update") {
                    self.handle_step(su);
                }
            }
            Some("result") => {
                if let Some(r) = v.get("result") {
                    self.saw_result = true;
                    self.status = r.get("status").and_then(Value::as_str).map(str::to_string);
                    if let Some(resp) = r.get("response").and_then(Value::as_str) {
                        self.final_response = Some(resp.to_string());
                    }
                    if let Some(u) = r.get("usage") {
                        self.usage = Some(parse_usage(u));
                    }
                    self.events.push(event(
                        WorkingState::TurnCompleted,
                        format!(
                            "turn completed (agy stream, {})",
                            self.status.as_deref().unwrap_or("no status")
                        ),
                        None,
                        None,
                        self.usage.clone(),
                    ));
                }
            }
            _ => {}
        }
    }

    fn handle_step(&mut self, su: &Value) {
        let step_type = su.get("step_type").and_then(Value::as_str).unwrap_or("");
        let state = su.get("state").and_then(Value::as_str).unwrap_or("");
        match step_type {
            "agent_response" => {
                if let Some(t) = su.get("text_delta").and_then(Value::as_str) {
                    self.text_deltas.push(t.to_string());
                }
                // Usage appears on EVERY agent_response step, one per turn, so the last one
                // wins rather than being summed. The `result` event's usage is authoritative
                // and overwrites this if it arrives.
                if let Some(u) = su.get("usage") {
                    self.usage = Some(parse_usage(u));
                }
            }
            "tool" => self.handle_tool(su, state),
            _ => {}
        }
    }

    fn handle_tool(&mut self, su: &Value, state: &str) {
        let idx = su.get("step_index").and_then(Value::as_i64).unwrap_or(-1);
        let name = su
            .get("tool_name")
            .and_then(Value::as_str)
            .or_else(|| {
                su.get("tool_info")
                    .and_then(|ti| ti.get("name"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("unknown")
            .to_string();
        let params = su
            .get("tool_info")
            .and_then(|ti| ti.get("parameters"))
            .map(|p| p.to_string());
        let duration_ms = su
            .get("duration_seconds")
            .and_then(Value::as_f64)
            .map(|s| (s * 1000.0).round() as u64);

        // A tool step that carries an `output` and state DONE succeeded. agy does not emit an
        // explicit error flag in the captures, so absence of DONE means "still running", which
        // is `None` and NOT a claim of success.
        let done = state.eq_ignore_ascii_case("DONE");

        if let Some(pos) = self.open_steps.iter().position(|(i, _)| *i == idx) {
            let (_, call_pos) = self.open_steps[pos];
            let rec = &mut self.tool_calls[call_pos];
            if done {
                rec.success = Some(true);
                self.open_steps.remove(pos);
            }
            if params.is_some() {
                rec.args_json = params;
            }
            if duration_ms.is_some() {
                rec.duration_ms = duration_ms;
            }
            self.events.push(event(
                WorkingState::ToolCallCompleted,
                name.clone(),
                Some(name),
                None,
                None,
            ));
            return;
        }

        self.tool_calls.push(ToolCallRecord {
            id: Some(idx.to_string()),
            tool: name.clone(),
            kind: map_agy_tool_kind(&name),
            success: if done { Some(true) } else { None },
            duration_ms,
            args_json: params,
        });
        if !done {
            self.open_steps.push((idx, self.tool_calls.len() - 1));
        }
        self.events.push(event(
            WorkingState::ToolCallStarted,
            name.clone(),
            Some(name),
            None,
            None,
        ));
    }

    /// Did the stream reach a terminal `result` event?
    ///
    /// Exposed because a truncated stream is NOT a successful empty answer. The plain-text path
    /// already treats exit-0-with-empty-output as a canary (REQ-024); this is the structured
    /// equivalent and the caller must keep enforcing it.
    pub fn saw_result(&self) -> bool {
        self.saw_result
    }

    /// The stream's conversation id, for logs and correlation ONLY. Never published as a
    /// resumable session id: see the note in `finish`.
    pub fn conversation_id(&self) -> Option<&str> {
        self.conversation_id.as_deref()
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub fn finish(self) -> ParsedAgentResult {
        // `result.response` is the whole answer and wins. The accumulated deltas are the
        // fallback for a stream that ended without a result event, so a truncated turn still
        // yields whatever text did arrive rather than nothing.
        let response_text = self
            .final_response
            .clone()
            .unwrap_or_else(|| self.text_deltas.join(""))
            .trim()
            .to_string();
        ParsedAgentResult {
            response_text,
            // DELIBERATELY None, even though the stream carries a `conversation_id`.
            //
            // agy is single-turn by contract (agy-integration-spec REQ-040/042): resume flags
            // are never passed and an inbound session id is ignored. Publishing an id here
            // would let the worker registry cache it and a later turn try to resume a session
            // the dispatcher will never actually resume, which is the shape of the
            // cross-session leak this project already fixed once.
            //
            // The id is still parsed and exposed via `conversation_id()` for logging and
            // correlation, which is what it is actually good for.
            session_id: None,
            events: self.events,
            tool_calls: self.tool_calls,
            token_usage: self.usage,
            cli_version: None,
            parser_mode: PARSER_MODE_STREAM.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures are LIVE captures from agy v1.1.23 on 2026-09-01, not hand-written. A
    // hand-written fixture only proves the parser matches my belief about the format, which is
    // exactly the mistake that left agy unable to report tool calls for months.
    const NOTOOLS: &str = include_str!("../tests/fixtures/agy-stream-notools-20260901.jsonl");
    const TOOLS: &str = include_str!("../tests/fixtures/agy-stream-tools-20260901.jsonl");

    fn parse(s: &str) -> ParsedAgentResult {
        let mut p = AgyStreamParser::new();
        for line in s.lines() {
            p.parse_line(line);
        }
        p.finish()
    }

    /// THE WHOLE POINT. RED IF: tool steps stop being recorded, which is the state that made
    /// Antigravity permanently unable to pass the sight gate.
    #[test]
    fn agy_01_tool_steps_become_tool_call_records() {
        let r = parse(TOOLS);
        assert!(
            !r.tool_calls.is_empty(),
            "the tools fixture contains step_type=tool events and they must be recorded; \
             an empty vec here is the exact defect this parser exists to fix"
        );
        assert!(
            r.tool_calls.iter().any(|c| c.tool == "run_command"),
            "expected the run_command call from the capture, got: {:?}",
            r.tool_calls.iter().map(|c| &c.tool).collect::<Vec<_>>()
        );
    }

    /// The sight gate matches `required_sources` against recorded arguments, so parameters
    /// must survive. RED IF: `tool_info.parameters` stops being captured into `args_json`.
    #[test]
    fn agy_02_tool_parameters_survive_into_args_json() {
        let r = parse(TOOLS);
        let args: Vec<&str> = r
            .tool_calls
            .iter()
            .filter_map(|c| c.args_json.as_deref())
            .collect();
        assert!(
            args.iter().any(|a| a.contains("CommandLine")),
            "run_command's parameters must be preserved for source matching; got: {args:?}"
        );
        assert!(
            args.iter().any(|a| a.contains("evidence.txt")),
            "the searched-for path must be visible in recorded args, or required_sources \
             cannot be enforced for agy; got: {args:?}"
        );
    }

    /// agy is single-turn: publishing a resumable session id would let the worker registry
    /// cache something no dispatcher will ever resume.
    /// RED IF: `conversation_id` starts leaking into `session_id`.
    #[test]
    fn agy_11_no_session_id_is_published() {
        let r = parse(NOTOOLS);
        assert_eq!(r.session_id, None, "agy is single-turn (REQ-040/042)");
        let mut p = AgyStreamParser::new();
        for l in NOTOOLS.lines() {
            p.parse_line(l);
        }
        assert!(
            p.conversation_id().is_some(),
            "the id is still parsed, just not published as resumable"
        );
    }

    /// RED IF: the parser mode drifts. The sight gate allowlist keys on this exact string, so
    /// a rename silently locks Antigravity out again.
    #[test]
    fn agy_03_parser_mode_is_the_one_the_sight_gate_trusts() {
        assert_eq!(parse(NOTOOLS).parser_mode, "agy-stream-json");
        assert_eq!(PARSER_MODE_STREAM, "agy-stream-json");
    }

    /// RED IF: `result.response` stops being preferred, or text is lost entirely. Switching
    /// output formats must not cost us the actual answer.
    #[test]
    fn agy_04_the_final_answer_comes_from_the_result_event() {
        let r = parse(NOTOOLS);
        assert_eq!(r.response_text, "pong", "got: {:?}", r.response_text);
    }

    /// RED IF: usage stops being read from the stream. This replaces log-file scraping.
    #[test]
    fn agy_05_token_usage_is_read_from_the_stream() {
        let u = parse(NOTOOLS).token_usage.expect("usage present");
        assert_eq!(u.input, Some(13246));
        assert_eq!(u.output, Some(290));
        assert_eq!(u.thinking_tokens, Some(289));
        assert_eq!(u.cached, Some(8122), "cache reads stay separate from input");
    }

    /// A tool step still ACTIVE has not been shown to have succeeded.
    ///
    /// This matters to the gate: `success: None` is treated as success there, so a parser that
    /// marked in-flight calls as successful would let an incomplete look satisfy a source.
    /// RED IF: an ACTIVE-only step is recorded with `success: Some(true)`.
    #[test]
    fn agy_06_an_unfinished_tool_step_is_not_marked_successful() {
        let line = r#"{"event":"step_update","step_update":{"step_index":9,"state":"ACTIVE","step_type":"tool","tool_name":"view_file","tool_info":{"name":"view_file","parameters":{"AbsolutePath":"/x/y.rs"}}}}"#;
        let mut p = AgyStreamParser::new();
        p.parse_line(line);
        let r = p.finish();
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(
            r.tool_calls[0].success, None,
            "an ACTIVE step has not completed and must not claim success"
        );
    }

    /// A later DONE must land on ITS OWN step, not the most recent one.
    /// RED IF: pairing goes back to "update the last open call", which mislabels interleaved
    /// steps and would attribute one tool's arguments to another.
    #[test]
    fn agy_07_a_done_update_lands_on_its_own_step_index() {
        let mut p = AgyStreamParser::new();
        p.parse_line(r#"{"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"tool","tool_name":"view_file","tool_info":{"name":"view_file","parameters":{"AbsolutePath":"/a.rs"}}}}"#);
        p.parse_line(r#"{"event":"step_update","step_update":{"step_index":3,"state":"ACTIVE","step_type":"tool","tool_name":"grep_search","tool_info":{"name":"grep_search","parameters":{"Query":"z"}}}}"#);
        p.parse_line(r#"{"event":"step_update","step_update":{"step_index":2,"state":"DONE","step_type":"tool","tool_name":"view_file","duration_seconds":0.5,"tool_info":{"name":"view_file","parameters":{"AbsolutePath":"/a.rs"},"output":"ok"}}}"#);
        let r = p.finish();
        assert_eq!(r.tool_calls.len(), 2, "two distinct steps");
        let view = r.tool_calls.iter().find(|c| c.tool == "view_file").unwrap();
        let grep = r.tool_calls.iter().find(|c| c.tool == "grep_search").unwrap();
        assert_eq!(view.success, Some(true), "step 2 completed");
        assert_eq!(grep.success, None, "step 3 never completed and must stay open");
    }

    /// Write tools must classify as writes so the no-touch check can see them.
    /// RED IF: write_to_file or replace_file_content stop mapping to a mutation kind.
    #[test]
    fn agy_08_write_tools_are_classified_as_mutations() {
        assert_eq!(map_agy_tool_kind("write_to_file"), ToolKind::WriteFile);
        assert_eq!(map_agy_tool_kind("replace_file_content"), ToolKind::EditFile);
        assert_eq!(map_agy_tool_kind("multi_replace_file_content"), ToolKind::EditFile);
        assert_eq!(map_agy_tool_kind("view_file"), ToolKind::ReadFile);
        // Unmapped names must NOT be read-shaped, or an unknown tool could satisfy a source.
        assert_eq!(map_agy_tool_kind("some_future_tool"), ToolKind::Unknown);
    }

    /// A truncated stream must be distinguishable from a completed one.
    /// RED IF: `saw_result` stops tracking the terminal event, which would let a cut-off turn
    /// look like a successful short answer.
    #[test]
    fn agy_09_a_truncated_stream_is_detectable() {
        let mut p = AgyStreamParser::new();
        p.parse_line(r#"{"event":"init","conversation_id":"c1","init":{"cwd":"/x"}}"#);
        assert!(!p.saw_result(), "no result event was seen");
        let full = {
            let mut q = AgyStreamParser::new();
            for l in NOTOOLS.lines() {
                q.parse_line(l);
            }
            q
        };
        assert!(full.saw_result(), "the complete fixture reaches a result event");
        assert_eq!(full.status(), Some("SUCCESS"));
    }

    /// Garbage on stdout must not lose the turn.
    /// RED IF: a non-JSON line becomes fatal.
    #[test]
    fn agy_10_a_stray_non_json_line_is_survivable() {
        let mut p = AgyStreamParser::new();
        p.parse_line("warning: something on stdout");
        for l in NOTOOLS.lines() {
            p.parse_line(l);
        }
        assert_eq!(p.finish().response_text, "pong");
    }
}
