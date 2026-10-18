//! Qwen stream parser (Seven Primes REQ-023, REQ-040, REQ-044, REQ-052). research/03-qwen-code-contract.md holds the
//! stream shapes; fixtures are transcribed from vendor docs (`*-provisional-20261018.jsonl`) until T-302 commits live
//! captures.
//!
//! Qwen Code's stream-json is the Claude Code family (research/03 "Output (stream-json)"): one JSON object per line,
//! `system`/`session_start` first, `assistant` lines whose `message` carries content blocks, `model` and `usage`, and a
//! closing `result` with `subtype`, `is_error`, `duration_ms`, `result` and `usage`. The `tool_use` and `tool_result`
//! block shapes and the usage keys are research/03 open item 3: they follow the Claude Code shape until a live
//! fixture says otherwise.

use serde_json::Value;

use crate::{
    ParsedAgentResult, SeatOutcome, SeatParsed, TokenUsage, ToolCallRecord, ToolKind, UsageReport,
    WorkingState, WorkingStateEvent,
};

pub const PARSER_MODE: &str = "qwen-stream-json";

const AGENT: &str = "qwen";

/// Line-by-line parser for one dispatch.
#[derive(Debug, Default)]
pub struct QwenStreamParser {
    /// Set once any line had a recognized event type (`system`, `assistant`, `user`, `result`).
    recognized: bool,
    session_id: Option<String>,
    served_model: Option<String>,
    text_parts: Vec<String>,
    tool_calls: Vec<ToolCallRecord>,
    events: Vec<WorkingStateEvent>,
    last_assistant_usage: Option<TokenUsage>,
    result: Option<ResultLine>,
}

#[derive(Debug, Default)]
struct ResultLine {
    subtype: Option<String>,
    is_error: bool,
    text: Option<String>,
    usage: Option<TokenUsage>,
}

impl QwenStreamParser {
    /// A parser with no lines seen.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one stdout line; returns a working-state event when the line carries one. Blank, malformed and partial
    /// lines are skipped with a debug log, and unknown event types are ignored (REQ-044).
    pub fn parse_line(&mut self, line: &str) -> Option<WorkingStateEvent> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(error = %e, "qwen stream: skipping malformed line");
                return None;
            }
        };
        let event = match value.get("type").and_then(Value::as_str) {
            Some("system") => self.on_system(&value),
            Some("assistant") => self.on_assistant(&value),
            Some("user") => self.on_user(&value),
            Some("result") => self.on_result(&value),
            other => {
                tracing::debug!(event_type = ?other, "qwen stream: ignoring unknown event type");
                None
            }
        };
        if let Some(ev) = &event {
            self.events.push(ev.clone());
        }
        event
    }

    /// Finish the dispatch given the process exit code. Exit codes per research/03 "Exit codes": 0 success, 53
    /// `--max-session-turns` exceeded (partial), 55 run budget exceeded (partial), 130 SIGINT, anything else an
    /// error. Exit 0 is a success only when the stream closed with a non-error `result` (REQ-052).
    pub fn finish(self, exit_code: Option<i32>) -> Result<SeatParsed, String> {
        let outcome = match exit_code {
            Some(0) => match &self.result {
                _ if !self.recognized => {
                    SeatOutcome::Failure("empty_or_unrecognized_stream:qwen".to_string())
                }
                None => SeatOutcome::Failure("missing_result:qwen".to_string()),
                Some(r) if r.is_error || r.subtype.as_deref() != Some("success") => {
                    SeatOutcome::Failure(format!(
                        "result_error:qwen: {}",
                        r.subtype.as_deref().unwrap_or("no_subtype")
                    ))
                }
                Some(_) => SeatOutcome::Success,
            },
            Some(53) => SeatOutcome::Partial("max_session_turns_exceeded:qwen".to_string()),
            Some(55) => SeatOutcome::Partial("run_budget_exceeded:qwen".to_string()),
            Some(130) => SeatOutcome::Failure("interrupted:qwen: exit 130".to_string()),
            Some(code) => SeatOutcome::Failure(format!("exit_error:qwen: exit {code}")),
            None => SeatOutcome::Failure("no_exit_code:qwen".to_string()),
        };

        let assistant_text = self.text_parts.join("\n");
        let (result_text, result_usage) = match self.result {
            Some(r) => (r.text, r.usage),
            None => (None, None),
        };
        let response_text = if assistant_text.is_empty() {
            result_text.unwrap_or_default()
        } else {
            assistant_text
        };
        // `result.usage` is the run total; a stream cut before `result` falls back to the last assistant usage.
        let token_usage = result_usage.or(self.last_assistant_usage);
        let usage = match &token_usage {
            Some(u) => UsageReport::Exact(u.clone()),
            None => UsageReport::NotReported,
        };

        Ok(SeatParsed {
            result: ParsedAgentResult {
                response_text,
                session_id: self.session_id,
                events: self.events,
                tool_calls: self.tool_calls,
                token_usage,
                self_reported_cost_usd: None,
                cli_version: None,
                parser_mode: PARSER_MODE.to_string(),
            },
            outcome,
            usage,
            served_model: self.served_model,
        })
    }

    fn note_session(&mut self, value: &Value) {
        if self.session_id.is_none()
            && let Some(id) = value.get("session_id").and_then(Value::as_str)
            && !id.is_empty()
        {
            self.session_id = Some(id.to_string());
        }
    }

    fn on_system(&mut self, value: &Value) -> Option<WorkingStateEvent> {
        self.recognized = true;
        if value.get("subtype").and_then(Value::as_str) != Some("session_start") {
            return None;
        }
        // research/03: `session_start.session_id` is the id `--resume` takes. Its `model` is the configured model,
        // not proof of what answered; the served model comes from `assistant.message.model` (REQ-043).
        self.note_session(value);
        Some(event(
            WorkingState::TurnStarted,
            "session started",
            None,
            None,
        ))
    }

    fn on_assistant(&mut self, value: &Value) -> Option<WorkingStateEvent> {
        self.recognized = true;
        self.note_session(value);
        let message = value.get("message")?;
        if let Some(model) = message.get("model").and_then(Value::as_str)
            && !model.is_empty()
        {
            self.served_model = Some(model.to_string());
        }
        if let Some(u) = message.get("usage").and_then(usage_from) {
            self.last_assistant_usage = Some(u);
        }
        let mut last = None;
        for block in message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(Value::as_str)
                        && !t.is_empty()
                    {
                        self.text_parts.push(t.to_string());
                        last = Some(event(WorkingState::MessageDelta, t, None, None));
                    }
                }
                Some("tool_use") => {
                    let tool = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string();
                    // The vendor's own input object is kept verbatim, so `read_file`'s path stays under its own key
                    // (`file_path` or `absolute_path`), both of which are sight-gate READ_OPERAND_KEYS.
                    let args_json = block.get("input").map(Value::to_string);
                    self.tool_calls.push(ToolCallRecord {
                        id: block.get("id").and_then(Value::as_str).map(str::to_string),
                        kind: tool_kind(&tool),
                        tool: tool.clone(),
                        success: None,
                        duration_ms: None,
                        args_json: args_json.clone(),
                        returned_lines: Vec::new(),
                        shown_output: None,
                    });
                    last = Some(event(
                        WorkingState::ToolCallStarted,
                        &tool,
                        Some(tool.clone()),
                        args_json,
                    ));
                }
                _ => {}
            }
        }
        last
    }

    fn on_user(&mut self, value: &Value) -> Option<WorkingStateEvent> {
        self.recognized = true;
        let mut last = None;
        let blocks = value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)?;
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let id = block.get("tool_use_id").and_then(Value::as_str);
            let failed = block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(rec) = self
                .tool_calls
                .iter_mut()
                .rev()
                .find(|r| id.is_some() && r.id.as_deref() == id)
            {
                rec.success = Some(!failed);
                last = Some(event(
                    WorkingState::ToolCallCompleted,
                    &rec.tool,
                    Some(rec.tool.clone()),
                    rec.args_json.clone(),
                ));
            }
        }
        last
    }

    fn on_result(&mut self, value: &Value) -> Option<WorkingStateEvent> {
        self.recognized = true;
        self.note_session(value);
        let mut usage = value.get("usage").and_then(usage_from);
        if let (Some(u), Some(ms)) = (
            usage.as_mut(),
            value.get("duration_ms").and_then(Value::as_u64),
        ) {
            u.latency_ms = Some(ms);
        }
        let subtype = value
            .get("subtype")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.result = Some(ResultLine {
            is_error: value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            text: value
                .get("result")
                .and_then(Value::as_str)
                .map(str::to_string),
            usage,
            subtype: subtype.clone(),
        });
        Some(event(
            WorkingState::TurnCompleted,
            subtype.as_deref().unwrap_or("result"),
            None,
            None,
        ))
    }
}

fn event(
    state: WorkingState,
    detail: &str,
    tool_name: Option<String>,
    tool_args_json: Option<String>,
) -> WorkingStateEvent {
    WorkingStateEvent {
        agent: AGENT.to_string(),
        state,
        detail: detail.to_string(),
        tool_name,
        tool_args_json,
        token_usage: None,
        ts_ms: None,
    }
}

/// Qwen tool names from research/03 "Round 1 research additions" (packages/core/src/tools/tool-names.ts).
fn tool_kind(name: &str) -> ToolKind {
    match name {
        "read_file" => ToolKind::ReadFile,
        "write_file" => ToolKind::WriteFile,
        "edit" => ToolKind::EditFile,
        "run_shell_command" | "exec" => ToolKind::Bash,
        "grep_search" => ToolKind::Grep,
        "glob" => ToolKind::Glob,
        _ => ToolKind::Unknown,
    }
}

/// Claude-shape usage keys (research/03 open item 3). An object with none of them is not a report: REQ-041 keeps
/// missing usage NotReported, never zero, while a reported zero stays Exact(0).
fn usage_from(usage: &Value) -> Option<TokenUsage> {
    let get = |k: &str| usage.get(k).and_then(Value::as_u64);
    let input = get("input_tokens");
    let output = get("output_tokens");
    let cached = get("cache_read_input_tokens");
    let total = get("total_tokens");
    if input.is_none() && output.is_none() && cached.is_none() && total.is_none() {
        return None;
    }
    Some(TokenUsage {
        input,
        output,
        cached,
        thinking_tokens: None,
        latency_ms: None,
        tool_calls: None,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/qwen-0.25.2-provisional-20261018.jsonl");

    /// Second variant: different session, path, answer, served model and usage, with the path under the
    /// `absolute_path` key the Gemini CLI lineage uses.
    const VARIANT: &str = concat!(
        r#"{"type":"system","subtype":"session_start","uuid":"u1","session_id":"c41d0e77-2a9b-4f3e-8d15-9be0a4c6f702","model":"qwen3.7-plus"}"#,
        "\n",
        r#"{"type":"assistant","uuid":"u2","session_id":"c41d0e77-2a9b-4f3e-8d15-9be0a4c6f702","message":{"role":"assistant","model":"qwen3.7-plus","content":[{"type":"tool_use","id":"call_9","name":"read_file","input":{"absolute_path":"/work/lib/util.py"}}],"usage":{"input_tokens":10,"output_tokens":2}},"parent_tool_use_id":null}"#,
        "\n",
        r#"{"type":"user","uuid":"u3","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_9","content":"def f(): pass","is_error":false}]}}"#,
        "\n",
        r#"{"type":"assistant","uuid":"u4","session_id":"c41d0e77-2a9b-4f3e-8d15-9be0a4c6f702","message":{"role":"assistant","model":"qwen3.7-plus","content":[{"type":"text","text":"util.py defines one empty function."}],"usage":{"input_tokens":30,"output_tokens":9}},"parent_tool_use_id":null}"#,
        "\n",
        r#"{"type":"result","subtype":"success","uuid":"u5","session_id":"c41d0e77-2a9b-4f3e-8d15-9be0a4c6f702","is_error":false,"duration_ms":999,"result":"util.py defines one empty function.","usage":{"input_tokens":0,"output_tokens":11}}"#,
        "\n",
    );

    fn parse(stream: &str, exit: Option<i32>) -> SeatParsed {
        let mut p = QwenStreamParser::new();
        for line in stream.lines() {
            p.parse_line(line);
        }
        p.finish(exit).expect("finish")
    }

    /// Provisional: the fixture is transcribed from research/03 stream shapes; nothing is installed yet (T-302).
    /// RED IF: the parser returns constants or defaults: session id, served model, usage, text or the ReadFile record.
    #[test]
    fn fixture_yields_session_model_usage_text_and_read() {
        let s = parse(FIXTURE, Some(0));
        assert_eq!(s.outcome, SeatOutcome::Success);
        assert_eq!(
            s.result.session_id.as_deref(),
            Some("7f3c9a2e-5b1d-4e8a-9c40-2d6f1e8b7a01")
        );
        assert_eq!(s.served_model.as_deref(), Some("qwen3-coder-plus"));
        assert_eq!(s.result.response_text, "main.rs prints hello and exits.");
        assert_eq!(s.result.parser_mode, PARSER_MODE);
        let UsageReport::Exact(u) = &s.usage else {
            panic!("expected Exact usage, got {:?}", s.usage)
        };
        assert_eq!(u.input, Some(2600));
        assert_eq!(u.output, Some(52));
        assert_eq!(u.cached, Some(800));
        assert_eq!(u.total, Some(2652));
        assert_eq!(u.latency_ms, Some(4321));
        assert_eq!(s.result.tool_calls.len(), 1);
        let rec = &s.result.tool_calls[0];
        assert_eq!(rec.tool, "read_file");
        assert_eq!(rec.kind, ToolKind::ReadFile);
        assert_eq!(rec.success, Some(true));
        let args: Value = serde_json::from_str(rec.args_json.as_deref().unwrap()).unwrap();
        assert_eq!(args["file_path"], "/repo/src/main.rs");
    }

    /// Provisional: inline variant transcribed from research/03 stream shapes (nothing installed).
    /// RED IF: a second stream with different values yields the first fixture's values (constant-returning parser),
    /// or a reported zero is turned into NotReported.
    #[test]
    fn second_variant_yields_its_own_values() {
        let s = parse(VARIANT, Some(0));
        assert_eq!(s.outcome, SeatOutcome::Success);
        assert_eq!(
            s.result.session_id.as_deref(),
            Some("c41d0e77-2a9b-4f3e-8d15-9be0a4c6f702")
        );
        assert_eq!(s.served_model.as_deref(), Some("qwen3.7-plus"));
        assert_eq!(
            s.result.response_text,
            "util.py defines one empty function."
        );
        let UsageReport::Exact(u) = &s.usage else {
            panic!("expected Exact usage")
        };
        assert_eq!(u.input, Some(0));
        assert_eq!(u.output, Some(11));
        let args: Value =
            serde_json::from_str(s.result.tool_calls[0].args_json.as_deref().unwrap()).unwrap();
        assert_eq!(args["absolute_path"], "/work/lib/util.py");
    }

    /// RED IF: the served model is read from session_start (configured) instead of assistant.message.model.
    #[test]
    fn served_model_comes_from_message_not_session_start() {
        let stream = FIXTURE.replace(
            r#""role":"assistant","model":"qwen3-coder-plus""#,
            r#""role":"assistant","model":"glm-5""#,
        );
        assert_eq!(
            parse(&stream, Some(0)).served_model.as_deref(),
            Some("glm-5")
        );
    }

    /// RED IF: an empty stream with exit 0 reads as success.
    #[test]
    fn empty_stream_exit_zero_is_failure() {
        let s = parse("", Some(0));
        assert_eq!(
            s.outcome,
            SeatOutcome::Failure("empty_or_unrecognized_stream:qwen".to_string())
        );
        assert_eq!(s.usage, UsageReport::NotReported);
        let s = parse("not json\n{\"type\":\"mystery\"}\n", Some(0));
        assert_eq!(
            s.outcome,
            SeatOutcome::Failure("empty_or_unrecognized_stream:qwen".to_string())
        );
    }

    /// RED IF: a missing session id is invented, or a stream with no result line reads as success.
    #[test]
    fn missing_session_id_is_none() {
        let stream: String = FIXTURE
            .lines()
            .map(|l| {
                let mut v: Value = serde_json::from_str(l).unwrap();
                v.as_object_mut().unwrap().remove("session_id");
                v.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let s = parse(&stream, Some(0));
        assert_eq!(s.result.session_id, None);
        assert_eq!(s.outcome, SeatOutcome::Success);
        let no_result: String = FIXTURE.lines().take(4).collect::<Vec<_>>().join("\n");
        assert_eq!(
            parse(&no_result, Some(0)).outcome,
            SeatOutcome::Failure("missing_result:qwen".to_string())
        );
    }

    /// RED IF: a malformed line, a partial trailing line or an unknown event aborts the parse or loses the rest.
    #[test]
    fn malformed_partial_and_unknown_lines_are_skipped() {
        let lines: Vec<&str> = FIXTURE.lines().collect();
        let stream = format!(
            "{}\n{{not json\n{}\n{{\"type\":\"stream_event\",\"event\":{{}}}}\n{}\n{}\n{}\n{{\"type\":\"assis",
            lines[0], lines[1], lines[2], lines[3], lines[4]
        );
        let s = parse(&stream, Some(0));
        assert_eq!(s.outcome, SeatOutcome::Success);
        assert_eq!(s.result.response_text, "main.rs prints hello and exits.");
        assert_eq!(s.result.tool_calls.len(), 1);
        assert_eq!(
            s.result.session_id.as_deref(),
            Some("7f3c9a2e-5b1d-4e8a-9c40-2d6f1e8b7a01")
        );
    }

    /// RED IF: the research/03 exit-code table (0/53/55/130/other) or exit-0-with-error-result mapping changes.
    #[test]
    fn exit_codes_map_to_outcomes() {
        assert!(matches!(
            parse(FIXTURE, Some(53)).outcome,
            SeatOutcome::Partial(r) if r == "max_session_turns_exceeded:qwen"
        ));
        assert!(matches!(
            parse(FIXTURE, Some(55)).outcome,
            SeatOutcome::Partial(r) if r == "run_budget_exceeded:qwen"
        ));
        assert!(matches!(
            parse(FIXTURE, Some(130)).outcome,
            SeatOutcome::Failure(r) if r.starts_with("interrupted:qwen")
        ));
        assert!(matches!(
            parse(FIXTURE, Some(1)).outcome,
            SeatOutcome::Failure(r) if r == "exit_error:qwen: exit 1"
        ));
        assert!(matches!(
            parse(FIXTURE, None).outcome,
            SeatOutcome::Failure(r) if r == "no_exit_code:qwen"
        ));
        let errored = FIXTURE
            .replace(
                r#""subtype":"success""#,
                r#""subtype":"error_during_execution""#,
            )
            .replace(
                r#""is_error":false,"duration_ms""#,
                r#""is_error":true,"duration_ms""#,
            );
        assert!(matches!(
            parse(&errored, Some(0)).outcome,
            SeatOutcome::Failure(r) if r == "result_error:qwen: error_during_execution"
        ));
    }

    /// RED IF: usage objects with no known keys are reported as Exact zeros instead of NotReported.
    #[test]
    fn missing_usage_is_not_reported() {
        let stream: String = FIXTURE
            .lines()
            .map(|l| {
                let mut v: Value = serde_json::from_str(l).unwrap();
                if let Some(m) = v.get_mut("message").and_then(Value::as_object_mut) {
                    m.remove("usage");
                }
                if let Some(o) = v.as_object_mut()
                    && o.contains_key("usage")
                {
                    o.insert("usage".to_string(), serde_json::json!({"unknown_key": 1}));
                }
                v.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let s = parse(&stream, Some(0));
        assert_eq!(s.usage, UsageReport::NotReported);
        assert_eq!(s.result.token_usage, None);
    }
}
