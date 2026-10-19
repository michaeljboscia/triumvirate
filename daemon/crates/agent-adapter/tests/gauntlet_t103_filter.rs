// GAUNTLET filter suite for T-103, written blind from the spec (no implementation read).
//
// Role: filter. Crate: agent-adapter (Qwen stream parser). The builder half lives in
// daemon/crates/mcp-bridge/tests/gauntlet_t103_filter.rs.
//
// SPEC_AMBIGUITY: when result.usage and an assistant usage both exist, the spec says "result.usage / assistant usage" without
// saying which wins. This suite assumes the closing result wins and the last assistant usage is the fallback.
// SPEC_AMBIGUITY: whether a failure is returned as Err(code) or as Ok(SeatParsed{outcome: Failure(code)}) is not stated for
// every case. Tests accept either shape through `failure_code`, and only the code text is asserted.
// SPEC_AMBIGUITY: no failure code is named for "exit 0 but the stream never closed with a result", for non-zero exits, or
// for SIGINT (None exit code). Those tests assert only that the outcome is a failure.
// SPEC_AMBIGUITY: the ToolKind for grep_search, glob and list_directory is not stated, so only read_file -> ReadFile and
// "write or shell tools are never ReadFile" are asserted.
// SPEC_AMBIGUITY: ToolCallRecord.success from a tool_result block is not specified (the Kimi contract leaves it None), so
// only "an is_error tool_result never yields Some(true)" is asserted.
// SPEC_AMBIGUITY: the cached-token mapping from `cache_read_input_tokens` to TokenUsage.cached is assumed (Claude shape).

use agent_adapter::qwen::{PARSER_MODE, QwenStreamParser};
use agent_adapter::{SeatOutcome, SeatParsed, ToolKind, UsageReport};

const FIXTURE: &str = include_str!("fixtures/qwen-0.25.2-provisional-20261018.jsonl");

const FIXTURE_SESSION: &str = "7f3c9a2e-5b1d-4e8a-9c40-2d6f1e8b7a01";

fn parse(stream: &str, exit: Option<i32>) -> Result<SeatParsed, String> {
    let mut p = QwenStreamParser::new();
    for line in stream.lines() {
        let _ = p.parse_line(line);
    }
    p.finish(exit)
}

fn parse_ok(stream: &str, exit: Option<i32>) -> SeatParsed {
    parse(stream, exit).expect("finish should return a parsed result for this stream")
}

fn failure_code(r: Result<SeatParsed, String>) -> String {
    match r {
        Err(e) => e,
        Ok(p) => match p.outcome {
            SeatOutcome::Failure(s) => s,
            SeatOutcome::Success => panic!("expected a failure, got Success"),
            SeatOutcome::Partial(s) => panic!("expected a failure, got Partial({s})"),
        },
    }
}

fn is_failure(r: Result<SeatParsed, String>) -> bool {
    match r {
        Err(_) => true,
        Ok(p) => matches!(p.outcome, SeatOutcome::Failure(_)),
    }
}

fn args_of(call: &agent_adapter::ToolCallRecord) -> String {
    call.args_json.clone().unwrap_or_default()
}

fn start(session: &str, model: &str) -> String {
    format!(
        r#"{{"type":"system","subtype":"session_start","uuid":"u0","session_id":"{session}","model":"{model}","cwd":"/repo","tools":["read_file"]}}"#
    )
}

fn assistant_text(session: &str, model: &str, text: &str, usage: &str) -> String {
    format!(
        r#"{{"type":"assistant","uuid":"u1","session_id":"{session}","message":{{"id":"m","type":"message","role":"assistant","model":"{model}","content":[{{"type":"text","text":"{text}"}}]{usage}}},"parent_tool_use_id":null}}"#
    )
}

fn assistant_tool(session: &str, model: &str, id: &str, name: &str, input: &str) -> String {
    format!(
        r#"{{"type":"assistant","uuid":"u2","session_id":"{session}","message":{{"id":"m","type":"message","role":"assistant","model":"{model}","content":[{{"type":"tool_use","id":"{id}","name":"{name}","input":{input}}}]}},"parent_tool_use_id":null}}"#
    )
}

fn tool_result(session: &str, id: &str, content: &str, is_error: bool) -> String {
    format!(
        r#"{{"type":"user","uuid":"u3","session_id":"{session}","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"{id}","content":"{content}","is_error":{is_error}}}]}},"parent_tool_use_id":null}}"#
    )
}

fn result_line(session: &str, text: &str, is_error: bool, subtype: &str, usage: &str) -> String {
    format!(
        r#"{{"type":"result","subtype":"{subtype}","uuid":"u4","session_id":"{session}","is_error":{is_error},"duration_ms":10,"num_turns":1,"result":"{text}"{usage}}}"#
    )
}

fn join(lines: &[String]) -> String {
    lines.join("\n") + "\n"
}

// A second, different variant of the happy stream (different session, model, path, answer, usage).
const SESSION_B: &str = "11112222-3333-4444-5555-666677778888";

fn variant_b() -> String {
    join(&[
        start(SESSION_B, "qwen3.7-plus"),
        assistant_tool(SESSION_B, "qwen3.7-plus", "call_b", "read_file", r#"{"file_path":"/repo/Cargo.toml"}"#),
        tool_result(SESSION_B, "call_b", "[package]", false),
        assistant_text(SESSION_B, "qwen3.7-plus", "Cargo.toml declares one package.", r#","usage":{"input_tokens":70,"output_tokens":9}"#),
        result_line(
            SESSION_B,
            "Cargo.toml declares one package.",
            false,
            "success",
            r#","usage":{"input_tokens":123,"output_tokens":45}"#,
        ),
    ])
}

// ---------------------------------------------------------------- happy path, exact values

// REQ-040: PARSER_MODE constant is the registered parser mode.
#[test]
fn test_should_expose_qwen_stream_json_parser_mode() {
    assert_eq!(PARSER_MODE, "qwen-stream-json");
}

// REQ-040: session id comes from system/session_start.
#[test]
fn test_should_take_session_id_from_session_start() {
    let sut = parse_ok(FIXTURE, Some(0));
    assert_eq!(sut.result.session_id.as_deref(), Some(FIXTURE_SESSION));
}

// REQ-043: served model comes from the assistant message.model.
#[test]
fn test_should_take_served_model_from_assistant_message() {
    let sut = parse_ok(FIXTURE, Some(0));
    assert_eq!(sut.served_model.as_deref(), Some("qwen3-coder-plus"));
}

// REQ-040: response text equals the assistant text.
#[test]
fn test_should_return_assistant_text_as_response() {
    let sut = parse_ok(FIXTURE, Some(0));
    assert_eq!(sut.result.response_text, "main.rs prints hello and exits.");
}

// REQ-040/REQ-063: one ReadFile record carries the path under file_path in args_json.
#[test]
fn test_should_record_one_read_file_call_with_path_under_operand_key() {
    let sut = parse_ok(FIXTURE, Some(0));
    assert_eq!(sut.result.tool_calls.len(), 1);
    let call = &sut.result.tool_calls[0];
    assert_eq!(call.tool, "read_file");
    assert!(matches!(call.kind, ToolKind::ReadFile));
    let args = args_of(call);
    assert!(args.contains("\"file_path\""), "args_json was {args}");
    assert!(args.contains("/repo/src/main.rs"), "args_json was {args}");
}

// REQ-040: the record keeps the vendor tool call id.
#[test]
fn test_should_keep_tool_call_id() {
    let sut = parse_ok(FIXTURE, Some(0));
    assert_eq!(sut.result.tool_calls[0].id.as_deref(), Some("call_read_1"));
}

// REQ-040: parser_mode on the result is the qwen mode.
#[test]
fn test_should_stamp_result_with_qwen_parser_mode() {
    let sut = parse_ok(FIXTURE, Some(0));
    assert_eq!(sut.result.parser_mode, "qwen-stream-json");
}

// REQ-052: exit 0 with a non-error result is Success.
#[test]
fn test_should_report_success_for_exit_zero_with_clean_result() {
    let sut = parse_ok(FIXTURE, Some(0));
    assert!(matches!(sut.outcome, SeatOutcome::Success));
}

// REQ-041: Exact usage from the closing result (assumes result wins, see SPEC_AMBIGUITY).
#[test]
fn test_should_report_exact_usage_from_result_line() {
    let sut = parse_ok(FIXTURE, Some(0));
    match sut.usage {
        UsageReport::Exact(u) => {
            assert_eq!(u.input, Some(2600));
            assert_eq!(u.output, Some(52));
            assert_eq!(u.cached, Some(800));
        }
        UsageReport::NotReported => panic!("fixture reports usage"),
    }
}

// REQ-040: a second variant yields its own, different values (defeats constant-returning parsers).
#[test]
fn test_should_yield_variant_specific_values_for_second_fixture() {
    let sut = parse_ok(&variant_b(), Some(0));
    assert_eq!(sut.result.session_id.as_deref(), Some(SESSION_B));
    assert_eq!(sut.served_model.as_deref(), Some("qwen3.7-plus"));
    assert_eq!(sut.result.response_text, "Cargo.toml declares one package.");
    assert_eq!(sut.result.tool_calls.len(), 1);
    let args = args_of(&sut.result.tool_calls[0]);
    assert!(args.contains("/repo/Cargo.toml"), "args_json was {args}");
    assert!(!args.contains("/repo/src/main.rs"));
    match sut.usage {
        UsageReport::Exact(u) => {
            assert_eq!(u.input, Some(123));
            assert_eq!(u.output, Some(45));
        }
        UsageReport::NotReported => panic!("variant reports usage"),
    }
}

// REQ-040: the two variants never share a session id or text.
#[test]
fn test_should_not_leak_state_between_parser_instances() {
    let a = parse_ok(FIXTURE, Some(0));
    let b = parse_ok(&variant_b(), Some(0));
    assert_ne!(a.result.session_id, b.result.session_id);
    assert_ne!(a.result.response_text, b.result.response_text);
}

// REQ-043: a glm-5 stream reports glm-5 as the served model (the mcp-bridge suite feeds this to check_served_model).
#[test]
fn test_should_surface_non_qwen_served_model_verbatim() {
    let s = "aaaa0000-0000-4000-8000-000000000000";
    let stream = join(&[
        start(s, "glm-5"),
        assistant_text(s, "glm-5", "hi", r#","usage":{"input_tokens":1,"output_tokens":1}"#),
        result_line(s, "hi", false, "success", r#","usage":{"input_tokens":1,"output_tokens":1}"#),
    ]);
    let sut = parse_ok(&stream, Some(0));
    assert_eq!(sut.served_model.as_deref(), Some("glm-5"));
}

// REQ-043: the assistant message model wins over the configured model in session_start.
#[test]
fn test_should_prefer_message_model_over_session_start_model() {
    let s = "bbbb0000-0000-4000-8000-000000000000";
    let stream = join(&[
        start(s, "qwen3-coder-plus"),
        assistant_text(s, "glm-5", "hi", ""),
        result_line(s, "hi", false, "success", ""),
    ]);
    let sut = parse_ok(&stream, Some(0));
    assert_eq!(sut.served_model.as_deref(), Some("glm-5"));
}

// REQ-040: several assistant text messages all reach the response text.
#[test]
fn test_should_keep_text_from_every_assistant_message() {
    let s = "cccc0000-0000-4000-8000-000000000000";
    let stream = join(&[
        start(s, "qwen3-coder-plus"),
        assistant_text(s, "qwen3-coder-plus", "FIRSTPART", ""),
        assistant_text(s, "qwen3-coder-plus", "SECONDPART", ""),
        result_line(s, "SECONDPART", false, "success", ""),
    ]);
    let sut = parse_ok(&stream, Some(0));
    assert!(sut.result.response_text.contains("SECONDPART"));
}

// REQ-040/REQ-063: two reads produce two records, each with its own path, in order.
#[test]
fn test_should_record_each_read_with_its_own_path() {
    let s = "dddd0000-0000-4000-8000-000000000000";
    let stream = join(&[
        start(s, "qwen3-coder-plus"),
        assistant_tool(s, "qwen3-coder-plus", "c1", "read_file", r#"{"file_path":"/repo/a.rs"}"#),
        tool_result(s, "c1", "a", false),
        assistant_tool(s, "qwen3-coder-plus", "c2", "read_file", r#"{"file_path":"/repo/b.rs"}"#),
        tool_result(s, "c2", "b", false),
        assistant_text(s, "qwen3-coder-plus", "done", ""),
        result_line(s, "done", false, "success", ""),
    ]);
    let sut = parse_ok(&stream, Some(0));
    assert_eq!(sut.result.tool_calls.len(), 2);
    assert!(args_of(&sut.result.tool_calls[0]).contains("/repo/a.rs"));
    assert!(args_of(&sut.result.tool_calls[1]).contains("/repo/b.rs"));
    assert_eq!(sut.result.tool_calls[0].id.as_deref(), Some("c1"));
    assert_eq!(sut.result.tool_calls[1].id.as_deref(), Some("c2"));
}

// REQ-063: a shell or write tool is never classified as a read (it would satisfy the sight gate).
#[test]
fn test_should_not_classify_shell_or_write_tools_as_reads() {
    let s = "eeee0000-0000-4000-8000-000000000000";
    let stream = join(&[
        start(s, "qwen3-coder-plus"),
        assistant_tool(s, "qwen3-coder-plus", "c1", "run_shell_command", r#"{"command":"cat /repo/a.rs"}"#),
        assistant_tool(s, "qwen3-coder-plus", "c2", "write_file", r#"{"file_path":"/repo/a.rs","content":"x"}"#),
        result_line(s, "x", false, "success", ""),
    ]);
    let sut = parse_ok(&stream, Some(0));
    assert_eq!(sut.result.tool_calls.len(), 2);
    for call in &sut.result.tool_calls {
        assert!(!matches!(call.kind, ToolKind::ReadFile), "{} classified as a read", call.tool);
    }
}

// REQ-063: a read whose tool_result is an error is not recorded as a successful read.
#[test]
fn test_should_not_mark_errored_read_as_success() {
    let s = "ffff0000-0000-4000-8000-000000000000";
    let stream = join(&[
        start(s, "qwen3-coder-plus"),
        assistant_tool(s, "qwen3-coder-plus", "c1", "read_file", r#"{"file_path":"/repo/missing.rs"}"#),
        tool_result(s, "c1", "ENOENT", true),
        result_line(s, "none", false, "success", ""),
    ]);
    let sut = parse_ok(&stream, Some(0));
    assert_eq!(sut.result.tool_calls.len(), 1);
    assert_ne!(sut.result.tool_calls[0].success, Some(true));
}

// ---------------------------------------------------------------- usage states (REQ-041)

// REQ-041: a reported zero stays Exact(0).
#[test]
fn test_should_keep_reported_zero_usage_as_exact_zero() {
    let s = "0000aaaa-0000-4000-8000-000000000000";
    let stream = join(&[
        start(s, "qwen3-coder-plus"),
        assistant_text(s, "qwen3-coder-plus", "ok", ""),
        result_line(s, "ok", false, "success", r#","usage":{"input_tokens":0,"output_tokens":0}"#),
    ]);
    let sut = parse_ok(&stream, Some(0));
    match sut.usage {
        UsageReport::Exact(u) => {
            assert_eq!(u.input, Some(0));
            assert_eq!(u.output, Some(0));
        }
        UsageReport::NotReported => panic!("a reported zero must stay Exact"),
    }
}

// REQ-041: no usage anywhere is NotReported, never zero.
#[test]
fn test_should_report_not_reported_when_no_usage_present() {
    let s = "0000bbbb-0000-4000-8000-000000000000";
    let stream = join(&[
        start(s, "qwen3-coder-plus"),
        assistant_text(s, "qwen3-coder-plus", "ok", ""),
        result_line(s, "ok", false, "success", ""),
    ]);
    let sut = parse_ok(&stream, Some(0));
    assert!(matches!(sut.usage, UsageReport::NotReported));
}

// REQ-041: a usage object with none of the known keys is not a report.
#[test]
fn test_should_treat_usage_object_without_known_keys_as_not_reported() {
    let s = "0000cccc-0000-4000-8000-000000000000";
    let stream = join(&[
        start(s, "qwen3-coder-plus"),
        assistant_text(s, "qwen3-coder-plus", "ok", r#","usage":{"unrelated":5}"#),
        result_line(s, "ok", false, "success", r#","usage":{}"#),
    ]);
    let sut = parse_ok(&stream, Some(0));
    assert!(matches!(sut.usage, UsageReport::NotReported));
}

// REQ-041: with no result usage, the last assistant usage is the report.
#[test]
fn test_should_fall_back_to_last_assistant_usage() {
    let s = "0000dddd-0000-4000-8000-000000000000";
    let stream = join(&[
        start(s, "qwen3-coder-plus"),
        assistant_text(s, "qwen3-coder-plus", "one", r#","usage":{"input_tokens":5,"output_tokens":1}"#),
        assistant_text(s, "qwen3-coder-plus", "two", r#","usage":{"input_tokens":7,"output_tokens":3}"#),
        result_line(s, "two", false, "success", ""),
    ]);
    let sut = parse_ok(&stream, Some(0));
    match sut.usage {
        UsageReport::Exact(u) => {
            assert_eq!(u.input, Some(7));
            assert_eq!(u.output, Some(3));
        }
        UsageReport::NotReported => panic!("assistant usage was reported"),
    }
}

// REQ-041: the result's token_usage mirrors the Exact usage, and is None when NotReported.
#[test]
fn test_should_keep_result_token_usage_consistent_with_usage_report() {
    let reported = parse_ok(FIXTURE, Some(0));
    let tu = reported.result.token_usage.expect("token_usage set when usage is Exact");
    assert_eq!(tu.input, Some(2600));
    assert_eq!(tu.output, Some(52));

    let s = "0000eeee-0000-4000-8000-000000000000";
    let stream = join(&[start(s, "qwen3-coder-plus"), assistant_text(s, "qwen3-coder-plus", "ok", ""), result_line(s, "ok", false, "success", "")]);
    let missing = parse_ok(&stream, Some(0));
    assert!(missing.result.token_usage.is_none());
}

// ---------------------------------------------------------------- REQ-044 mutation cases

// REQ-040: absent session_start and absent session ids give no session id, never a fabricated one.
#[test]
fn test_should_not_fabricate_session_id_when_absent() {
    let stream = join(&[
        r#"{"type":"assistant","message":{"role":"assistant","model":"qwen3-coder-plus","content":[{"type":"text","text":"ok"}]}}"#.to_string(),
        r#"{"type":"result","subtype":"success","is_error":false,"result":"ok"}"#.to_string(),
    ]);
    let sut = parse_ok(&stream, Some(0));
    assert_eq!(sut.result.session_id, None);
}

// REQ-044: a partial trailing line is skipped and the rest still parses.
#[test]
fn test_should_skip_partial_trailing_line() {
    let mut text = FIXTURE.to_string();
    text.push_str(r#"{"type":"assistant","uuid":"x","message":{"role":"assis"#);
    let sut = parse_ok(&text, Some(0));
    assert_eq!(sut.result.session_id.as_deref(), Some(FIXTURE_SESSION));
    assert_eq!(sut.result.response_text, "main.rs prints hello and exits.");
    assert!(matches!(sut.outcome, SeatOutcome::Success));
}

// REQ-044: a malformed JSON line in the middle is skipped and the rest still parses.
#[test]
fn test_should_skip_malformed_middle_line() {
    let mut lines: Vec<&str> = FIXTURE.lines().collect();
    lines.insert(2, "{not json at all");
    let sut = parse_ok(&lines.join("\n"), Some(0));
    assert_eq!(sut.result.tool_calls.len(), 1);
    assert_eq!(sut.result.response_text, "main.rs prints hello and exits.");
}

// REQ-044: blank lines are skipped.
#[test]
fn test_should_skip_blank_lines() {
    let text = FIXTURE.replace('\n', "\n\n   \n");
    let sut = parse_ok(&text, Some(0));
    assert_eq!(sut.result.session_id.as_deref(), Some(FIXTURE_SESSION));
    assert!(matches!(sut.outcome, SeatOutcome::Success));
}

// REQ-044: an unknown event type is ignored and the rest still parses.
#[test]
fn test_should_ignore_unknown_event_type() {
    let mut lines: Vec<&str> = FIXTURE.lines().collect();
    lines.insert(1, r#"{"type":"mystery_event","payload":{"a":1}}"#);
    let sut = parse_ok(&lines.join("\n"), Some(0));
    assert_eq!(sut.result.tool_calls.len(), 1);
    assert!(matches!(sut.outcome, SeatOutcome::Success));
}

// REQ-044: blank and malformed lines produce no working-state event.
#[test]
fn test_should_return_no_event_for_blank_and_malformed_lines() {
    let mut sut = QwenStreamParser::new();
    assert!(sut.parse_line("").is_none());
    assert!(sut.parse_line("   ").is_none());
    assert!(sut.parse_line("{broken").is_none());
    assert!(sut.parse_line("not json").is_none());
}

// REQ-044: JSON that is not an object never panics and never counts as a recognized stream.
#[test]
fn test_should_not_recognize_non_object_json_lines() {
    for junk in ["null", "[]", "123", "\"text\"", r#"{"type":5}"#, r#"{"no_type":true}"#] {
        let failure = failure_code(parse(junk, Some(0)));
        assert!(failure.starts_with("empty_or_unrecognized_stream:qwen"), "{junk} -> {failure}");
    }
}

// ---------------------------------------------------------------- REQ-052 outcome mapping

// REQ-052: an empty stream with exit 0 is failure empty_or_unrecognized_stream:qwen.
#[test]
fn test_should_fail_empty_stream_with_exit_zero() {
    let code = failure_code(parse("", Some(0)));
    assert!(code.starts_with("empty_or_unrecognized_stream:qwen"), "got {code}");
}

// REQ-052: a stream of only unknown event types with exit 0 is the same failure.
#[test]
fn test_should_fail_unrecognized_only_stream_with_exit_zero() {
    let stream = join(&[r#"{"type":"mystery"}"#.to_string(), r#"{"type":"other","x":1}"#.to_string()]);
    let code = failure_code(parse(&stream, Some(0)));
    assert!(code.starts_with("empty_or_unrecognized_stream:qwen"), "got {code}");
}

// REQ-052: exit 0 with an is_error result is a failure.
#[test]
fn test_should_fail_exit_zero_with_error_result() {
    let s = "2222aaaa-0000-4000-8000-000000000000";
    let stream = join(&[
        start(s, "qwen3-coder-plus"),
        assistant_text(s, "qwen3-coder-plus", "partial", ""),
        result_line(s, "boom", true, "error_during_execution", ""),
    ]);
    assert!(is_failure(parse(&stream, Some(0))));
}

// REQ-052: an is_error flag alone makes exit 0 a failure even when the subtype says success.
#[test]
fn test_should_fail_exit_zero_when_is_error_true_despite_success_subtype() {
    let s = "2222bbbb-0000-4000-8000-000000000000";
    let stream = join(&[start(s, "qwen3-coder-plus"), result_line(s, "x", true, "success", "")]);
    assert!(is_failure(parse(&stream, Some(0))));
}

// REQ-052: exit 0 is a success only when the stream closed with a result.
#[test]
fn test_should_not_succeed_on_exit_zero_without_result_line() {
    let s = "2222cccc-0000-4000-8000-000000000000";
    let stream = join(&[start(s, "qwen3-coder-plus"), assistant_text(s, "qwen3-coder-plus", "cut off", "")]);
    assert!(is_failure(parse(&stream, Some(0))));
}

// REQ-052: exit 53 (turn cap) is Partial and keeps the answer so far.
#[test]
fn test_should_report_partial_for_exit_53() {
    let s = "3333aaaa-0000-4000-8000-000000000000";
    let stream = join(&[start(s, "qwen3-coder-plus"), assistant_text(s, "qwen3-coder-plus", "got this far", "")]);
    let sut = parse_ok(&stream, Some(53));
    assert!(matches!(sut.outcome, SeatOutcome::Partial(_)));
    assert!(sut.result.response_text.contains("got this far"));
    assert_eq!(sut.result.session_id.as_deref(), Some(s));
}

// REQ-052: exit 55 (run budget) is Partial.
#[test]
fn test_should_report_partial_for_exit_55() {
    let s = "3333bbbb-0000-4000-8000-000000000000";
    let stream = join(&[start(s, "qwen3-coder-plus"), assistant_text(s, "qwen3-coder-plus", "budget hit", "")]);
    let sut = parse_ok(&stream, Some(55));
    assert!(matches!(sut.outcome, SeatOutcome::Partial(_)));
    assert!(sut.result.response_text.contains("budget hit"));
}

// REQ-052: the Partial reason is non-empty so the answer can be marked.
#[test]
fn test_should_give_partial_a_nonempty_reason() {
    let s = "3333cccc-0000-4000-8000-000000000000";
    let stream = join(&[start(s, "qwen3-coder-plus"), assistant_text(s, "qwen3-coder-plus", "x", "")]);
    for code in [53, 55] {
        match parse_ok(&stream, Some(code)).outcome {
            SeatOutcome::Partial(reason) => assert!(!reason.trim().is_empty(), "exit {code}"),
            _ => panic!("exit {code} should be Partial"),
        }
    }
}

// REQ-052: exit 53 and 55 reasons differ (they are different budgets).
#[test]
fn test_should_distinguish_partial_reasons_for_53_and_55() {
    let s = "3333dddd-0000-4000-8000-000000000000";
    let stream = join(&[start(s, "qwen3-coder-plus"), assistant_text(s, "qwen3-coder-plus", "x", "")]);
    let a = match parse_ok(&stream, Some(53)).outcome {
        SeatOutcome::Partial(r) => r,
        _ => panic!("53 not partial"),
    };
    let b = match parse_ok(&stream, Some(55)).outcome {
        SeatOutcome::Partial(r) => r,
        _ => panic!("55 not partial"),
    };
    assert_ne!(a, b);
}

// REQ-052: exit 130 (SIGINT) is a failure, not a partial.
#[test]
fn test_should_fail_exit_130() {
    assert!(is_failure(parse(FIXTURE, Some(130))));
}

// REQ-052: any other non-zero exit is a failure even when the stream looks clean.
#[test]
fn test_should_fail_other_nonzero_exits() {
    for code in [1, 2, 3, 52, 54, 56, 127, 137, 255] {
        assert!(is_failure(parse(FIXTURE, Some(code))), "exit {code} should fail");
    }
}

// REQ-052: a missing exit code (killed by signal) is a failure.
#[test]
fn test_should_fail_when_exit_code_missing() {
    assert!(is_failure(parse(FIXTURE, None)));
}

// REQ-052: a non-zero exit on an empty stream is a failure.
#[test]
fn test_should_fail_empty_stream_with_nonzero_exit() {
    assert!(is_failure(parse("", Some(1))));
}

// REQ-052: a clean success is never reported for exit codes the table calls partial or failing.
#[test]
fn test_should_never_return_success_for_nonzero_exit() {
    for code in [1, 53, 55, 130] {
        if let Ok(p) = parse(FIXTURE, Some(code)) {
            assert!(!matches!(p.outcome, SeatOutcome::Success), "exit {code} returned Success");
        }
    }
}
