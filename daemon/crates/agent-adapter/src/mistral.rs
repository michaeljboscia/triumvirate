//! Mistral stream parser (Seven Primes REQ-022, REQ-044). research/01..03 hold the stream shapes; fixtures are
//! transcribed from vendor source (`*-provisional-20261018.jsonl`) until T-302 commits live captures.
//!
//! research/02-mistral-vibe-contract.md is the source for every shape here:
//! - `--output streaming` is NDJSON, one COMPLETED history entry per line, with a common envelope
//!   (`id`, `sessionId`, `turnId`, `generationStatus`, `type`, ...).
//! - `type:"message"` + `role:"assistant"` carries `content:[{"type":"text","text":..}]`: the answer.
//! - `type:"effect"` is a tool use: `title`, `detail{toolName, kind, input}`, `state{status, output, display}`.
//!   `read_file` input names its path `filePath` (camelCase), kept verbatim so the sight gate finds it.
//! - `type:"reasoning"` entries are never appended to the answer.
//! - `export.json` (RunExport, schema_version 1) is authoritative for `session_id`, `usage`, `cost_usd`,
//!   `vibe_version` and `outcome`. A run without one is `missing_export:mistral`, never an empty success.
//! - Exit codes: 0 finished, 1 usage/config error, 2 infrastructure failure, 3 partial (turn, token, price or
//!   deadline limit, terminated, length, refusal), 4 aborted (agent socket only).

use serde_json::Value;

use crate::{
    ParsedAgentResult, SeatOutcome, SeatParsed, TokenUsage, ToolCallRecord, ToolKind, UsageReport,
    WorkingState, WorkingStateEvent,
};

pub const PARSER_MODE: &str = "mistral-streaming";

const AGENT: &str = "mistral";

/// RunExport outcomes that end with usable but incomplete output (exit 3, research/02 exit-code table).
const PARTIAL_OUTCOMES: [&str; 7] = [
    "turn_limit",
    "token_limit",
    "price_limit",
    "deadline",
    "terminated",
    "length",
    "refusal",
];

/// Line-by-line parser for one dispatch.
#[derive(Debug, Default)]
pub struct MistralStreamParser {
    texts: Vec<String>,
    tool_calls: Vec<ToolCallRecord>,
    events: Vec<WorkingStateEvent>,
}

fn tool_kind(tool: &str, kind: Option<&str>) -> ToolKind {
    match tool {
        "read_file" => ToolKind::ReadFile,
        "grep" => ToolKind::Grep,
        "write_file" => ToolKind::WriteFile,
        "edit" => ToolKind::EditFile,
        "bash" | "git_bash" | "experimental_bash" | "windows_shell" => ToolKind::Bash,
        "ask_user_question" => ToolKind::RequestUserInput,
        _ => match kind {
            Some("file_read") => ToolKind::ReadFile,
            Some("shell") => ToolKind::Bash,
            _ => ToolKind::Unknown,
        },
    }
}

fn event(
    state: WorkingState,
    detail: String,
    tool: Option<String>,
    args: Option<String>,
) -> WorkingStateEvent {
    WorkingStateEvent {
        agent: AGENT.to_string(),
        state,
        detail,
        tool_name: tool,
        tool_args_json: args,
        token_usage: None,
        ts_ms: None,
    }
}

impl MistralStreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one stdout line; returns a working-state event when the line carries one.
    pub fn parse_line(&mut self, line: &str) -> Option<WorkingStateEvent> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(error = %e, "mistral: skipping malformed stream line");
                return None;
            }
        };
        if let Some(status) = v.get("generationStatus").and_then(Value::as_str)
            && status != "completed"
        {
            return None;
        }
        let ev = match v.get("type").and_then(Value::as_str) {
            Some("message") => self.message(&v),
            Some("effect") => self.effect(&v),
            other => {
                tracing::debug!(entry_type = ?other, "mistral: ignoring stream entry");
                None
            }
        };
        if let Some(e) = &ev {
            self.events.push(e.clone());
        }
        ev
    }

    fn message(&mut self, v: &Value) -> Option<WorkingStateEvent> {
        if v.get("role").and_then(Value::as_str) != Some("assistant") {
            return None;
        }
        let text: String = v
            .get("content")
            .and_then(Value::as_array)?
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect();
        if text.is_empty() {
            return None;
        }
        self.texts.push(text.clone());
        Some(event(WorkingState::MessageDelta, text, None, None))
    }

    fn effect(&mut self, v: &Value) -> Option<WorkingStateEvent> {
        let detail = v.get("detail");
        let tool = detail
            .and_then(|d| d.get("toolName").or_else(|| d.get("tool_name")))
            .and_then(Value::as_str)
            .or_else(|| v.get("title").and_then(Value::as_str))?
            .to_string();
        let kind = tool_kind(
            &tool,
            detail.and_then(|d| d.get("kind")).and_then(Value::as_str),
        );
        let args_json = detail.and_then(|d| d.get("input")).map(Value::to_string);
        let state = v.get("state");
        // Fail closed: an explicit failed/error/rejected status wins over display.success, so a failed read can never
        // earn sight credit (GAUNTLET T-102 filter: status "failed" with display.success true was recorded a success).
        let status = state.and_then(|s| s.get("status")).and_then(Value::as_str);
        let display_success = state
            .and_then(|s| s.get("display"))
            .and_then(|d| d.get("success"))
            .and_then(Value::as_bool);
        let success = match status {
            Some("failed") | Some("error") | Some("rejected") => Some(false),
            _ => display_success.or(match status {
                Some("completed") => Some(true),
                _ => None,
            }),
        };
        let id = v.get("id").and_then(Value::as_str).map(str::to_string);
        let record = ToolCallRecord {
            id: id.clone(),
            tool: tool.clone(),
            kind,
            success,
            duration_ms: None,
            args_json: args_json.clone(),
            returned_lines: Vec::new(),
            shown_output: None,
        };
        // An entry is emitted once completed, but a re-emitted id replaces its record rather than doubling it.
        match self
            .tool_calls
            .iter_mut()
            .find(|r| id.is_some() && r.id == id)
        {
            Some(existing) => *existing = record,
            None => self.tool_calls.push(record),
        }
        Some(event(
            WorkingState::ToolCallCompleted,
            tool.clone(),
            Some(tool),
            args_json,
        ))
    }

    /// Finish the dispatch given the exit code and the contents of the per-call export.json (`None` when missing).
    pub fn finish(
        self,
        exit_code: Option<i32>,
        export_json: Option<&str>,
    ) -> Result<SeatParsed, String> {
        let mut result = ParsedAgentResult {
            response_text: self.texts.join("\n"),
            events: self.events,
            tool_calls: self.tool_calls,
            parser_mode: PARSER_MODE.to_string(),
            ..ParsedAgentResult::default()
        };
        let exit = exit_code.map_or_else(|| "none".to_string(), |c| c.to_string());

        let Some(raw) = export_json else {
            return Ok(SeatParsed {
                result,
                outcome: SeatOutcome::Failure(format!("missing_export:{AGENT}: exit {exit}")),
                usage: UsageReport::NotReported,
                served_model: None,
            });
        };
        let export: Value = match serde_json::from_str(raw) {
            Ok(v @ Value::Object(_)) => v,
            Ok(_) | Err(_) => {
                tracing::debug!("mistral: export.json is not a JSON object");
                return Ok(SeatParsed {
                    result,
                    outcome: SeatOutcome::Failure(format!("malformed_export:{AGENT}: exit {exit}")),
                    usage: UsageReport::NotReported,
                    served_model: None,
                });
            }
        };

        result.session_id = export
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        result.self_reported_cost_usd = export.get("cost_usd").and_then(Value::as_f64);
        result.cli_version = export
            .get("vibe_version")
            .and_then(Value::as_str)
            .map(str::to_string);

        let usage = export.get("usage").filter(|u| u.is_object()).map(|u| {
            let n = |k: &str| u.get(k).and_then(Value::as_u64);
            TokenUsage {
                input: n("input_tokens"),
                output: n("output_tokens"),
                cached: n("cached_input_tokens"),
                thinking_tokens: None,
                latency_ms: None,
                tool_calls: None,
                total: n("total_tokens"),
            }
        });
        let usage = match usage {
            Some(u) if u.input.is_some() || u.output.is_some() || u.total.is_some() => {
                result.token_usage = Some(u.clone());
                UsageReport::Exact(u)
            }
            _ => UsageReport::NotReported,
        };

        let outcome = classify(&export, exit_code, &exit, result.response_text.is_empty());
        Ok(SeatParsed {
            result,
            outcome,
            usage,
            served_model: None,
        })
    }
}

/// The export's `outcome` wins over the exit code when they disagree (an exit 0 with `infrastructure_failure` is a
/// failure); with no recognised outcome the exit-code table decides.
fn classify(export: &Value, exit_code: Option<i32>, exit: &str, empty: bool) -> SeatOutcome {
    let s = |k: &str| export.get(k).and_then(Value::as_str);
    let error = export
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let fail = |code: &str| {
        SeatOutcome::Failure(
            format!("{code}:{AGENT}: exit {exit} {error}")
                .trim_end()
                .to_string(),
        )
    };
    match s("outcome") {
        Some("finished") if exit_code == Some(0) => {
            if empty {
                SeatOutcome::Failure(format!("empty_or_unrecognized_stream:{AGENT}"))
            } else {
                SeatOutcome::Success
            }
        }
        Some("finished") => fail("exit_outcome_mismatch"),
        Some(o) if PARTIAL_OUTCOMES.contains(&o) => match s("stop_reason") {
            Some(r) if !r.is_empty() => SeatOutcome::Partial(format!("{o}: {r}")),
            _ => SeatOutcome::Partial(o.to_string()),
        },
        Some(o @ ("infrastructure_failure" | "usage_error" | "config_error" | "aborted")) => {
            fail(o)
        }
        _ => match exit_code {
            Some(0) if !empty => SeatOutcome::Success,
            Some(0) => SeatOutcome::Failure(format!("empty_or_unrecognized_stream:{AGENT}")),
            Some(1) => fail("usage_or_config_error"),
            Some(2) => fail("infrastructure_failure"),
            Some(3) => SeatOutcome::Partial("limit".to_string()),
            Some(4) => fail("aborted"),
            _ => fail("unexpected_exit"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STREAM: &str =
        include_str!("../tests/fixtures/mistral-2.26.1-provisional-20261018.jsonl");
    const EXPORT: &str =
        include_str!("../tests/fixtures/mistral-2.26.1-export-provisional-20261018.json");

    fn parse(stream: &str, exit: Option<i32>, export: Option<&str>) -> SeatParsed {
        let mut p = MistralStreamParser::new();
        for line in stream.lines() {
            p.parse_line(line);
        }
        p.finish(exit, export).expect("finish")
    }

    fn with_outcome(outcome: &str) -> String {
        let mut v: Value = serde_json::from_str(EXPORT).expect("fixture");
        v["outcome"] = Value::String(outcome.to_string());
        v.to_string()
    }

    /// Fixtures are provisional: transcribed from research/02 stream shapes, nothing is installed (T-302 replaces).
    /// RED IF: the answer loses an assistant message or gains user/reasoning text, the read_file record loses its
    /// `filePath` key (the FIND-GROK-01 sight-gate class), or session id / usage / cost stop coming from export.json.
    #[test]
    fn fixture_yields_text_read_session_and_usage() {
        let r = parse(STREAM, Some(0), Some(EXPORT));
        assert_eq!(r.outcome, SeatOutcome::Success);
        assert_eq!(
            r.result.response_text,
            "Reading the file.\nThe add function is correct."
        );
        assert!(!r.result.response_text.contains("should read"));
        assert_eq!(r.result.parser_mode, "mistral-streaming");
        assert_eq!(r.result.session_id.as_deref(), Some("vibe-sess-7f3a"));
        assert_eq!(r.result.cli_version.as_deref(), Some("2.26.1"));
        assert_eq!(r.result.self_reported_cost_usd, Some(0.0041));
        assert_eq!(r.result.tool_calls.len(), 1);
        let read = &r.result.tool_calls[0];
        assert_eq!(read.kind, ToolKind::ReadFile);
        assert_eq!(read.tool, "read_file");
        assert_eq!(read.success, Some(true));
        let args: Value =
            serde_json::from_str(read.args_json.as_deref().expect("args")).expect("json");
        assert_eq!(args["filePath"], "/repo/src/lib.rs");
        let UsageReport::Exact(u) = r.usage else {
            panic!("usage not exact: {:?}", r.usage)
        };
        assert_eq!(
            (u.input, u.output, u.cached, u.total),
            (Some(1834), Some(212), Some(640), Some(2046))
        );
    }

    /// Provisional fixture (research/02 shapes, nothing installed).
    /// RED IF: exit 3 stops being Partial, or an exit 0 whose export says infrastructure_failure reads as success.
    #[test]
    fn outcome_follows_export_and_exit_table() {
        let r = parse(STREAM, Some(3), Some(&with_outcome("turn_limit")));
        assert_eq!(
            r.outcome,
            SeatOutcome::Partial("turn_limit: end_turn".to_string())
        );
        let r = parse(
            STREAM,
            Some(0),
            Some(&with_outcome("infrastructure_failure")),
        );
        assert!(
            matches!(&r.outcome, SeatOutcome::Failure(m) if m.starts_with("infrastructure_failure:mistral")),
            "{:?}",
            r.outcome
        );
        let r = parse(STREAM, Some(1), Some(&with_outcome("config_error")));
        assert!(
            matches!(&r.outcome, SeatOutcome::Failure(m) if m.starts_with("config_error:mistral"))
        );
        let r = parse(STREAM, Some(2), Some(&with_outcome("")));
        assert!(
            matches!(&r.outcome, SeatOutcome::Failure(m) if m.starts_with("infrastructure_failure:mistral"))
        );
        let r = parse(STREAM, Some(3), Some(&with_outcome("")));
        assert!(matches!(r.outcome, SeatOutcome::Partial(_)));
        // The exit-code arms when the export names no outcome: every row of the table, by its own failure code.
        for (exit, code) in [
            (1, "usage_or_config_error"),
            (4, "aborted"),
            (9, "unexpected_exit"),
        ] {
            let r = parse(STREAM, Some(exit), Some(&with_outcome("")));
            assert!(
                matches!(&r.outcome, SeatOutcome::Failure(m) if m.starts_with(&format!("{code}:mistral"))),
                "exit {exit}: {:?}",
                r.outcome
            );
        }
        let r = parse(STREAM, Some(2), Some(EXPORT));
        assert!(
            matches!(&r.outcome, SeatOutcome::Failure(m) if m.starts_with("exit_outcome_mismatch:mistral"))
        );
    }

    /// Provisional fixture (research/02 shapes, nothing installed).
    /// RED IF: a run with no export.json reads as an empty success or invents a session id or usage.
    #[test]
    fn missing_export_is_failure() {
        let r = parse(STREAM, Some(0), None);
        assert!(
            matches!(&r.outcome, SeatOutcome::Failure(m) if m.starts_with("missing_export:mistral"))
        );
        assert_eq!(r.result.session_id, None);
        assert_eq!(r.usage, UsageReport::NotReported);
        let r = parse("", Some(0), Some("not json"));
        assert!(
            matches!(&r.outcome, SeatOutcome::Failure(m) if m.starts_with("malformed_export:mistral"))
        );
        let r = parse("", Some(0), Some(EXPORT));
        assert_eq!(
            r.outcome,
            SeatOutcome::Failure("empty_or_unrecognized_stream:mistral".to_string())
        );
    }

    /// Second variant, inline and provisional (research/02 shapes, nothing installed).
    /// RED IF: the parser returns constants instead of reading this variant's own values.
    #[test]
    fn second_variant_yields_its_own_values() {
        let stream = concat!(
            r#"{"id":"x-1","sessionId":"s","type":"effect","generationStatus":"completed","title":"grep","detail":{"toolName":"grep","kind":"search","input":{"pattern":"TODO","path":"/repo"}},"state":{"status":"failed"}}"#,
            "\n",
            r#"{"id":"x-2","sessionId":"s","type":"message","role":"assistant","generationStatus":"completed","content":[{"type":"text","text":"No TODOs "},{"type":"text","text":"found."}]}"#,
        );
        let export = r#"{"schema_version":1,"outcome":"deadline","stop_reason":"","session_id":"vibe-sess-b2","usage":{"input_tokens":90,"output_tokens":9,"cached_input_tokens":0,"total_tokens":99},"cost_usd":0.0002,"vibe_version":"2.26.2"}"#;
        let r = parse(stream, Some(3), Some(export));
        assert_eq!(r.outcome, SeatOutcome::Partial("deadline".to_string()));
        assert_eq!(r.result.response_text, "No TODOs found.");
        assert_eq!(r.result.session_id.as_deref(), Some("vibe-sess-b2"));
        assert_eq!(r.result.cli_version.as_deref(), Some("2.26.2"));
        assert_eq!(r.result.tool_calls[0].kind, ToolKind::Grep);
        assert_eq!(r.result.tool_calls[0].success, Some(false));
        assert_eq!(
            r.usage,
            UsageReport::Exact(TokenUsage {
                input: Some(90),
                output: Some(9),
                cached: Some(0),
                thinking_tokens: None,
                latency_ms: None,
                tool_calls: None,
                total: Some(99),
            })
        );
    }

    /// REQ-044 mutations, provisional fixture (research/02 shapes, nothing installed).
    /// RED IF: a partial line, malformed JSON or unknown entry type aborts the parse or leaks into the answer, a
    /// wrong-cased `Type` key is misread as an entry, a non-camelCase path key is rewritten, or a missing session id
    /// is invented.
    #[test]
    fn mutations_are_skipped_not_fatal() {
        let lines: Vec<&str> = STREAM.lines().collect();
        let partial = &lines[2][..lines[2].len() / 2];
        let mutated = [
            lines[0],
            partial,
            "{not json",
            r#"{"type":"compaction_marker","role":"assistant","content":[{"type":"text","text":"LEAK"}]}"#,
            r#"{"Type":"message","role":"assistant","content":[{"type":"text","text":"LEAK2"}]}"#,
            r#"{"type":"message","role":"assistant","generationStatus":"streaming","content":[{"type":"text","text":"LEAK3"}]}"#,
            r#"{"id":"m-1","type":"effect","detail":{"tool_name":"read_file","input":{"file_path":"/repo/a.rs"}},"state":{"status":"completed"}}"#,
            lines[4],
        ]
        .join("\n");
        let export = r#"{"outcome":"finished","usage":{}}"#;
        let r = parse(&mutated, Some(0), Some(export));
        assert_eq!(r.outcome, SeatOutcome::Success);
        assert_eq!(r.result.response_text, "The add function is correct.");
        assert_eq!(r.result.session_id, None);
        assert_eq!(r.usage, UsageReport::NotReported);
        assert_eq!(r.result.tool_calls.len(), 1);
        let rec = &r.result.tool_calls[0];
        assert_eq!(rec.kind, ToolKind::ReadFile);
        let args: Value =
            serde_json::from_str(rec.args_json.as_deref().expect("args")).expect("json");
        assert_eq!(args["file_path"], "/repo/a.rs");
    }

    /// RED IF: parse_line stops surfacing tool and message events to the working-state feed.
    #[test]
    fn parse_line_emits_working_state() {
        let lines: Vec<&str> = STREAM.lines().collect();
        let mut p = MistralStreamParser::new();
        assert!(p.parse_line(lines[0]).is_none());
        assert!(p.parse_line(lines[1]).is_none());
        let m = p.parse_line(lines[2]).expect("message event");
        assert_eq!(m.state, WorkingState::MessageDelta);
        let e = p.parse_line(lines[3]).expect("tool event");
        assert_eq!(e.state, WorkingState::ToolCallCompleted);
        assert_eq!(e.tool_name.as_deref(), Some("read_file"));
        assert!(
            e.tool_args_json
                .as_deref()
                .expect("args")
                .contains("filePath")
        );
    }
}
