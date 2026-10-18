//! Kimi stream parser (Seven Primes REQ-021, REQ-040, REQ-041, REQ-044, REQ-052). research/01-kimi-code-contract.md
//! ("stream-json output (from PromptJsonWriter, source)") holds the line shapes; fixtures are transcribed from vendor
//! source (`*-provisional-20261018.jsonl`) until T-302 commits live captures.
//!
//! One JSON object per stdout line, OpenAI-message shaped:
//! - `{"role":"assistant","content":"..","tool_calls":[{"id","function":{"name","arguments"}}]}`: text and tool calls;
//! - `{"role":"tool","tool_call_id","content"}`: a tool result, which carries no outcome (vendor issue #4241);
//! - `{"role":"meta","type":"system.version"|"turn.step.retrying"|"session.resume_hint",..}`.
//!
//! research/01: "No token usage and no model name in the stream", so usage is `NotReported` and `served_model` is
//! `None`. The session id comes only from `session.resume_hint`, written after a completed turn (#4240); without it the
//! id is `None`, never invented.

use serde_json::Value;

use crate::{
    ParsedAgentResult, SeatOutcome, SeatParsed, ToolCallRecord, ToolKind, UsageReport,
    WorkingState, WorkingStateEvent,
};

pub const PARSER_MODE: &str = "kimi-stream-json";

const AGENT: &str = "kimi";

/// Line-by-line parser for one dispatch.
#[derive(Debug, Default)]
pub struct KimiStreamParser {
    /// Non-empty assistant `content` strings, in stream order (flushed per step, research/01).
    text_chunks: Vec<String>,
    session_id: Option<String>,
    cli_version: Option<String>,
    events: Vec<WorkingStateEvent>,
    tool_calls: Vec<ToolCallRecord>,
    /// `status_code` of the most recent `turn.step.retrying` (429 feeds quota classification, research/01).
    last_retry_status: Option<u64>,
    /// Lines that parsed as a known kimi shape; zero means the stream was empty or not kimi's.
    recognized_lines: usize,
}

impl KimiStreamParser {
    /// A parser with nothing seen yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one stdout line; returns a working-state event when the line carries one.
    ///
    /// Malformed or partial JSON is skipped with a debug log and never aborts the parse; unknown roles and meta types
    /// are ignored.
    pub fn parse_line(&mut self, line: &str) -> Option<WorkingStateEvent> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(error = %e, line = trimmed, "kimi: skipping malformed stream-json line");
                return None;
            }
        };
        match value.get("role").and_then(Value::as_str) {
            Some("assistant") => self.on_assistant(&value),
            Some("tool") => self.on_tool_result(&value),
            Some("meta") => self.on_meta(&value),
            other => {
                tracing::debug!(role = ?other, "kimi: ignoring line with unknown role");
                None
            }
        }
    }

    fn on_assistant(&mut self, value: &Value) -> Option<WorkingStateEvent> {
        self.recognized_lines += 1;
        let mut last = None;
        if let Some(text) = value.get("content").and_then(Value::as_str)
            && !text.is_empty()
        {
            self.text_chunks.push(text.to_string());
            last = Some(self.record(event(WorkingState::MessageDelta, text)));
        }
        for call in value
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let function = call.get("function");
            let name = function
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            // `arguments` is a JSON string in the OpenAI shape; keep it verbatim so the vendor's `path` key (a
            // READ_OPERAND_KEYS member, reference_tools.md: "`Read` accepts a file path (`path`)") reaches the gate.
            let args_json = function
                .and_then(|f| f.get("arguments"))
                .and_then(|a| match a {
                    Value::String(s) => Some(s.clone()),
                    Value::Null => None,
                    other => Some(other.to_string()),
                });
            self.tool_calls.push(ToolCallRecord {
                id: call.get("id").and_then(Value::as_str).map(str::to_string),
                tool: name.clone(),
                kind: tool_kind(&name),
                // research/01, issue #4241: tool result lines carry no execution outcome.
                success: None,
                duration_ms: None,
                args_json: args_json.clone(),
                returned_lines: Vec::new(),
                shown_output: None,
            });
            let mut ev = event(WorkingState::ToolCallStarted, format!("calling {name}"));
            ev.tool_name = Some(name);
            ev.tool_args_json = args_json;
            last = Some(self.record(ev));
        }
        last
    }

    fn on_tool_result(&mut self, value: &Value) -> Option<WorkingStateEvent> {
        self.recognized_lines += 1;
        let id = value.get("tool_call_id").and_then(Value::as_str);
        let call = id.and_then(|id| self.tool_calls.iter().find(|c| c.id.as_deref() == Some(id)));
        let mut ev = event(WorkingState::ToolCallCompleted, "tool result");
        if let Some(call) = call {
            ev.detail = format!("{} returned", call.tool);
            ev.tool_name = Some(call.tool.clone());
            ev.tool_args_json = call.args_json.clone();
        }
        Some(self.record(ev))
    }

    fn on_meta(&mut self, value: &Value) -> Option<WorkingStateEvent> {
        match value.get("type").and_then(Value::as_str) {
            Some("system.version") => {
                self.recognized_lines += 1;
                self.cli_version = value
                    .get("version")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                None
            }
            Some("turn.step.retrying") => {
                self.recognized_lines += 1;
                let status = value.get("status_code").and_then(Value::as_u64);
                self.last_retry_status = status.or(self.last_retry_status);
                let detail = format!(
                    "retrying step: attempt {} of {}, status {}, {}",
                    value
                        .get("next_attempt")
                        .map_or("?".into(), Value::to_string),
                    value
                        .get("max_attempts")
                        .map_or("?".into(), Value::to_string),
                    status.map_or("none".into(), |s| s.to_string()),
                    value
                        .get("error_message")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                );
                Some(self.record(event(WorkingState::Unknown, detail)))
            }
            Some("session.resume_hint") => {
                self.recognized_lines += 1;
                if let Some(id) = value
                    .get("session_id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    self.session_id = Some(id.to_string());
                }
                None
            }
            other => {
                tracing::debug!(meta_type = ?other, "kimi: ignoring unknown meta type");
                None
            }
        }
    }

    fn record(&mut self, ev: WorkingStateEvent) -> WorkingStateEvent {
        self.events.push(ev.clone());
        ev
    }

    /// Finish the dispatch given the process exit code.
    ///
    /// Outcome (BACKEND_STRUCTURE section 6, research/01 "a failed turn throws -> non-zero"): exit 0 with assistant
    /// text is `Success`; exit 0 with no text is `Failure("empty_or_unrecognized_stream:kimi")`; a non-zero exit is
    /// `Failure("nonzero_exit:kimi:<code>")` with the last retry status appended when one was seen; no exit code
    /// (killed by a signal) is `Failure("no_exit_code:kimi")`.
    pub fn finish(self, exit_code: Option<i32>) -> Result<SeatParsed, String> {
        let response_text = self.text_chunks.join("\n");
        let retry = self
            .last_retry_status
            .map(|s| format!(" last_retry_status={s}"))
            .unwrap_or_default();
        let outcome = match exit_code {
            Some(0) if self.recognized_lines > 0 && !response_text.is_empty() => {
                SeatOutcome::Success
            }
            Some(0) => SeatOutcome::Failure("empty_or_unrecognized_stream:kimi".to_string()),
            Some(code) => SeatOutcome::Failure(format!("nonzero_exit:kimi:{code}{retry}")),
            None => SeatOutcome::Failure(format!("no_exit_code:kimi{retry}")),
        };
        Ok(SeatParsed {
            result: ParsedAgentResult {
                response_text,
                session_id: self.session_id,
                events: self.events,
                tool_calls: self.tool_calls,
                token_usage: None,
                self_reported_cost_usd: None,
                cli_version: self.cli_version,
                parser_mode: PARSER_MODE.to_string(),
            },
            outcome,
            usage: UsageReport::NotReported,
            served_model: None,
        })
    }
}

/// Built-in tool names from research/vendor/kimi-code/reference_tools.md.
fn tool_kind(name: &str) -> ToolKind {
    match name {
        "Read" => ToolKind::ReadFile,
        "Write" => ToolKind::WriteFile,
        "Edit" => ToolKind::EditFile,
        "Bash" => ToolKind::Bash,
        "Grep" => ToolKind::Grep,
        "Glob" => ToolKind::Glob,
        "AskUserQuestion" => ToolKind::RequestUserInput,
        _ => ToolKind::Unknown,
    }
}

fn event(state: WorkingState, detail: impl Into<String>) -> WorkingStateEvent {
    WorkingStateEvent {
        agent: AGENT.to_string(),
        state,
        detail: detail.into(),
        tool_name: None,
        tool_args_json: None,
        token_usage: None,
        ts_ms: None,
    }
}

/// The path a read names, under the sight gate's operand keys (the test mirror of READ_OPERAND_KEYS).
#[cfg(test)]
fn read_operand(args_json: &str) -> Option<String> {
    const READ_OPERAND_KEYS: &[&str] = &[
        "target_file",
        "path",
        "file_path",
        "filePath",
        "file",
        "abs_path",
        "AbsolutePath",
        "absolute_path",
        "filename",
    ];
    let v: Value = serde_json::from_str(args_json).ok()?;
    READ_OPERAND_KEYS
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str).map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const FIXTURE: &str = include_str!("../tests/fixtures/kimi-2.2.0-provisional-20261018.jsonl");

    /// Second variant with a different path, answer and session id (defeats a parser that returns constants).
    const VARIANT: &str = concat!(
        r#"{"role":"meta","type":"system.version","version":"2.2.0"}"#,
        "\n",
        r#"{"role":"assistant","content":"","tool_calls":[{"type":"function","id":"call_x","function":{"name":"Read","arguments":"{\"path\":\"/other/lib/util.py\"}"}}]}"#,
        "\n",
        r#"{"role":"tool","tool_call_id":"call_x","content":"1\tdef f(): pass"}"#,
        "\n",
        r#"{"role":"assistant","content":"util.py has one function, f."}"#,
        "\n",
        r#"{"role":"meta","type":"session.resume_hint","session_id":"sess-variant-9999","command":"kimi -r sess-variant-9999","content":"resume"}"#,
        "\n",
    );

    fn parse(stream: &str, exit: Option<i32>) -> SeatParsed {
        let mut p = KimiStreamParser::new();
        for line in stream.split('\n') {
            p.parse_line(line);
        }
        p.finish(exit).expect("finish")
    }

    // Fixtures here are PROVISIONAL: transcribed from research/01's PromptJsonWriter shapes, nothing is installed.
    /// RED IF: the parser drops assistant text, the Read call, its path, the resume_hint id, or claims usage.
    #[test]
    fn fixture_yields_text_read_session_and_no_usage() {
        let parsed = parse(FIXTURE, Some(0));
        assert_eq!(
            parsed.result.response_text,
            "main.rs defines an empty main function."
        );
        assert_eq!(parsed.result.tool_calls.len(), 1);
        let call = &parsed.result.tool_calls[0];
        assert_eq!(call.tool, "Read");
        assert_eq!(call.kind, ToolKind::ReadFile);
        assert_eq!(call.success, None);
        assert_eq!(
            read_operand(call.args_json.as_deref().expect("args")).as_deref(),
            Some("/repo/src/main.rs")
        );
        assert_eq!(
            parsed.result.session_id.as_deref(),
            Some("sess-kimi-fixture-0001")
        );
        assert_eq!(parsed.usage, UsageReport::NotReported);
        assert_eq!(parsed.outcome, SeatOutcome::Success);
        assert_eq!(parsed.served_model, None);
        assert_eq!(parsed.result.cli_version.as_deref(), Some("2.2.0"));
        assert_eq!(parsed.result.parser_mode, PARSER_MODE);
        let states: HashSet<_> = parsed
            .result
            .events
            .iter()
            .map(|e| format!("{:?}", e.state))
            .collect();
        assert!(states.contains("ToolCallStarted") && states.contains("ToolCallCompleted"));
    }

    // Provisional inline variant, same shapes as the fixture.
    /// RED IF: any of path, answer or session id is a constant rather than read from the stream.
    #[test]
    fn variant_yields_its_own_values() {
        let parsed = parse(VARIANT, Some(0));
        assert_eq!(parsed.result.response_text, "util.py has one function, f.");
        let call = &parsed.result.tool_calls[0];
        assert_eq!(
            read_operand(call.args_json.as_deref().expect("args")).as_deref(),
            Some("/other/lib/util.py")
        );
        assert_eq!(
            parsed.result.session_id.as_deref(),
            Some("sess-variant-9999")
        );
    }

    /// RED IF: a missing resume_hint produces a fabricated session id (REQ-044, issue #4240).
    #[test]
    fn missing_resume_hint_gives_no_session_id() {
        let stream: String = FIXTURE
            .lines()
            .filter(|l| !l.contains("session.resume_hint"))
            .collect::<Vec<_>>()
            .join("\n");
        let parsed = parse(&stream, Some(0));
        assert_eq!(parsed.result.session_id, None);
        assert_eq!(parsed.outcome, SeatOutcome::Success);
    }

    /// RED IF: a malformed line or a partial trailing line aborts the parse or loses the lines around it.
    #[test]
    fn malformed_and_partial_lines_are_skipped() {
        let stream = format!(
            "{}\nnot json at all {{\n{}\n{{\"role\":\"assistant\",\"con",
            FIXTURE.lines().take(3).collect::<Vec<_>>().join("\n"),
            FIXTURE.lines().skip(3).collect::<Vec<_>>().join("\n"),
        );
        let parsed = parse(&stream, Some(0));
        assert_eq!(
            parsed.result.response_text,
            "main.rs defines an empty main function."
        );
        assert_eq!(parsed.result.tool_calls.len(), 1);
        assert_eq!(
            parsed.result.session_id.as_deref(),
            Some("sess-kimi-fixture-0001")
        );
    }

    /// RED IF: an unknown meta type changes the result or produces an event.
    #[test]
    fn unknown_meta_type_is_ignored() {
        let mut p = KimiStreamParser::new();
        assert!(
            p.parse_line(r#"{"role":"meta","type":"turn.mystery","x":1}"#)
                .is_none()
        );
        for line in FIXTURE.lines() {
            p.parse_line(line);
        }
        let parsed = p.finish(Some(0)).expect("finish");
        assert_eq!(parsed.outcome, SeatOutcome::Success);
        assert_eq!(
            parsed.result.response_text,
            "main.rs defines an empty main function."
        );
    }

    /// RED IF: an empty stream that exits 0 is reported as success instead of a coded failure.
    #[test]
    fn empty_stream_exit_zero_is_failure() {
        let parsed = parse("", Some(0));
        assert_eq!(
            parsed.outcome,
            SeatOutcome::Failure("empty_or_unrecognized_stream:kimi".to_string())
        );
        assert_eq!(parsed.result.session_id, None);
        assert_eq!(parsed.usage, UsageReport::NotReported);
    }

    /// RED IF: a non-zero exit (research/01: a failed turn throws) is not a failure, or loses the 429 retry signal.
    #[test]
    fn nonzero_exit_is_failure_with_retry_status() {
        let stream = r#"{"role":"meta","type":"turn.step.retrying","failed_attempt":1,"next_attempt":2,"max_attempts":3,"delay_ms":1000,"error_name":"RateLimit","error_message":"too many","status_code":429}"#;
        let parsed = parse(stream, Some(1));
        assert_eq!(
            parsed.outcome,
            SeatOutcome::Failure("nonzero_exit:kimi:1 last_retry_status=429".to_string())
        );
        assert_eq!(
            parse("", None).outcome,
            SeatOutcome::Failure("no_exit_code:kimi".to_string())
        );
    }
}
