// GAUNTLET tiebreaker suite for T-103, written blind from the spec (no implementation read).
// SPEC_AMBIGUITY: REQ-052 specifies outcome variants for exits 53, 55, 130, and other nonzero exits but not their exact reason strings.
// SPEC_AMBIGUITY: REQ-052 specifies failure for exit 0 without a successful result but only the empty-stream case has exact error text.

use agent_adapter::qwen::{PARSER_MODE, QwenStreamParser};
use agent_adapter::{SeatOutcome, ToolKind, UsageReport, WorkingState};

const SESSION: &str = r#"{"type":"system","subtype":"session_start","uuid":"sys-1","session_id":"session-alpha","model":"qwen3-coder-plus","cwd":"/repo","tools":["read_file"]}"#;
const TOOL_USE: &str = r#"{"type":"assistant","uuid":"msg-1","session_id":"session-alpha","message":{"id":"message-1","type":"message","role":"assistant","model":"qwen3-coder-plus","content":[{"type":"tool_use","id":"call-read-1","name":"read_file","input":{"file_path":"/repo/src/main.rs"}}],"usage":{"input_tokens":1200,"output_tokens":40}},"parent_tool_use_id":null}"#;
const TOOL_RESULT: &str = r#"{"type":"user","uuid":"tool-1","session_id":"session-alpha","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call-read-1","content":"1\tfn main() {\n2\t    println!(\"hello\");\n3\t}\n","is_error":false}]},"parent_tool_use_id":null}"#;
const TEXT: &str = r#"{"type":"assistant","uuid":"msg-2","session_id":"session-alpha","message":{"id":"message-2","type":"message","role":"assistant","model":"qwen3-coder-plus","content":[{"type":"text","text":"main.rs prints hello and exits."}],"usage":{"input_tokens":1400,"output_tokens":12}},"parent_tool_use_id":null}"#;
const RESULT: &str = r#"{"type":"result","subtype":"success","uuid":"result-1","session_id":"session-alpha","is_error":false,"duration_ms":4321,"num_turns":2,"result":"main.rs prints hello and exits.","usage":{"input_tokens":2600,"output_tokens":52,"cache_read_input_tokens":800,"total_tokens":2652}}"#;

fn complete(lines: &[&str], exit_code: Option<i32>) -> Result<agent_adapter::SeatParsed, String> {
    let mut parser = QwenStreamParser::new();
    for line in lines {
        parser.parse_line(line);
    }
    parser.finish(exit_code)
}

#[test]
fn req_040_fixture_populates_every_observable_result_field() {
    // REQ-040: The provisional vendor stream yields exact text, session, model, usage, tool record, and parser mode.
    let parsed = complete(&[SESSION, TOOL_USE, TOOL_RESULT, TEXT, RESULT], Some(0)).unwrap();

    assert_eq!(
        parsed.result.response_text,
        "main.rs prints hello and exits."
    );
    assert_eq!(parsed.result.session_id.as_deref(), Some("session-alpha"));
    assert_eq!(parsed.served_model.as_deref(), Some("qwen3-coder-plus"));
    assert_eq!(parsed.result.parser_mode, PARSER_MODE);
    assert_eq!(parsed.result.cli_version, None);
    assert_eq!(parsed.result.self_reported_cost_usd, None);
    assert_eq!(parsed.result.tool_calls.len(), 1);
    assert_eq!(parsed.result.events.len(), 4);

    let usage = parsed.result.token_usage.as_ref().unwrap();
    assert_eq!(usage.input, Some(2600));
    assert_eq!(usage.output, Some(52));
    assert_eq!(usage.cached, Some(800));
    assert_eq!(usage.total, Some(2652));
    assert_eq!(usage.thinking_tokens, None);
    assert_eq!(usage.latency_ms, None);
    assert_eq!(usage.tool_calls, None);

    match parsed.outcome {
        SeatOutcome::Success => {}
        _ => panic!("exit 0 with a successful result must be Success"),
    }
    match parsed.usage {
        UsageReport::Exact(usage) => {
            assert_eq!(usage.input, Some(2600));
            assert_eq!(usage.output, Some(52));
            assert_eq!(usage.cached, Some(800));
            assert_eq!(usage.total, Some(2652));
        }
        UsageReport::NotReported => panic!("fixture reports exact usage"),
    }
}

#[test]
fn req_040_read_file_record_preserves_identity_operand_and_result_lines() {
    // REQ-040 and REQ-063: read_file becomes one successful ReadFile record with its file_path operand and returned lines.
    let parsed = complete(&[SESSION, TOOL_USE, TOOL_RESULT, TEXT, RESULT], Some(0)).unwrap();
    let call = &parsed.result.tool_calls[0];

    assert_eq!(call.id.as_deref(), Some("call-read-1"));
    assert_eq!(call.tool, "read_file");
    assert!(matches!(call.kind, ToolKind::ReadFile));
    assert_eq!(call.success, Some(true));
    assert_eq!(call.duration_ms, None);
    let args: serde_json::Value = serde_json::from_str(call.args_json.as_deref().unwrap()).unwrap();
    assert_eq!(
        args.get("file_path").and_then(|value| value.as_str()),
        Some("/repo/src/main.rs")
    );
    assert_eq!(call.returned_lines, vec![(1, 3)]);
    assert_eq!(call.shown_output, None);
}

#[test]
#[ignore = "GAUNTLET triage FP: the spec does not fix which lines emit working-state events; session_start emits TurnStarted"]
fn req_040_parse_line_emits_exact_working_state_transitions() {
    // REQ-040: Recognized stream lines emit the documented working-state classes.
    let mut parser = QwenStreamParser::new();

    assert!(parser.parse_line(SESSION).is_none());
    let started = parser.parse_line(TOOL_USE).unwrap();
    assert!(matches!(started.state, WorkingState::ToolCallStarted));
    assert_eq!(started.agent, "qwen");
    assert_eq!(started.tool_name.as_deref(), Some("read_file"));
    let completed = parser.parse_line(TOOL_RESULT).unwrap();
    assert!(matches!(completed.state, WorkingState::ToolCallCompleted));
    let delta = parser.parse_line(TEXT).unwrap();
    assert!(matches!(delta.state, WorkingState::MessageDelta));
    let finished = parser.parse_line(RESULT).unwrap();
    assert!(matches!(finished.state, WorkingState::TurnCompleted));
}

#[test]
#[ignore = "GAUNTLET triage FP: the spec does not fix which lines emit working-state events; session_start emits TurnStarted, so the event count is 3; the skipping itself is asserted by net and filter"]
fn req_044_blank_malformed_partial_and_unknown_lines_are_ignored() {
    // REQ-044: Blank, malformed, partial, and unknown event lines do not create events or corrupt valid output.
    let parsed = complete(
        &[
            "",
            "   ",
            "not-json",
            r#"{"type":"assistant","message":{"content":["#,
            r#"{"type":"future_vendor_event","value":99}"#,
            SESSION,
            TEXT,
            RESULT,
        ],
        Some(0),
    )
    .unwrap();

    assert_eq!(
        parsed.result.response_text,
        "main.rs prints hello and exits."
    );
    assert_eq!(parsed.result.session_id.as_deref(), Some("session-alpha"));
    assert_eq!(parsed.result.events.len(), 2);
}

#[test]
fn req_044_key_casing_is_not_silently_normalized() {
    // REQ-044: Incorrectly cased event keys remain unrecognized while later canonical events still parse.
    let wrong_case = r#"{"Type":"system","SubType":"session_start","Session_Id":"wrong-session"}"#;
    let parsed = complete(&[wrong_case, SESSION, TEXT, RESULT], Some(0)).unwrap();

    assert_eq!(parsed.result.session_id.as_deref(), Some("session-alpha"));
}

#[test]
fn req_040_second_fixture_variant_retains_its_own_values() {
    // REQ-040: A distinct stream cannot leak hard-coded values from the canonical fixture.
    let session = r#"{"type":"system","subtype":"session_start","session_id":"session-beta","model":"qwen3.7-plus"}"#;
    let text = r#"{"type":"assistant","session_id":"session-beta","message":{"role":"assistant","model":"qwen3.7-plus","content":[{"type":"text","text":"beta answer"}],"usage":{"input_tokens":3,"output_tokens":5}}}"#;
    let result = r#"{"type":"result","subtype":"success","session_id":"session-beta","is_error":false,"duration_ms":17,"result":"beta answer","usage":{"input_tokens":8,"output_tokens":13,"cache_read_input_tokens":2,"total_tokens":23}}"#;
    let parsed = complete(&[session, text, result], Some(0)).unwrap();

    assert_eq!(parsed.result.response_text, "beta answer");
    assert_eq!(parsed.result.session_id.as_deref(), Some("session-beta"));
    assert_eq!(parsed.served_model.as_deref(), Some("qwen3.7-plus"));
    let usage = parsed.result.token_usage.as_ref().unwrap();
    assert_eq!(usage.input, Some(8));
    assert_eq!(usage.output, Some(13));
    assert_eq!(usage.cached, Some(2));
    assert_eq!(usage.total, Some(23));
}

#[test]
#[ignore = "GAUNTLET triage FP: the spec does not fix Err versus Ok(SeatOutcome::Failure); the implementation returns Ok with Failure(empty_or_unrecognized_stream:qwen), the exact code"]
fn req_052_empty_stream_exit_zero_has_exact_failure() {
    // REQ-052: Exit zero without any recognized event is the specified failure, never empty success.
    let error = complete(&[], Some(0)).unwrap_err();

    assert_eq!(error, "empty_or_unrecognized_stream:qwen");
}

#[test]
fn req_052_unknown_only_stream_exit_zero_has_exact_failure() {
    // REQ-052: Ignored vendor events do not satisfy the recognized-stream success precondition.
    let error = complete(&[r#"{"type":"future_vendor_event"}"#], Some(0)).unwrap_err();

    assert_eq!(error, "empty_or_unrecognized_stream:qwen");
}

#[test]
fn req_052_exit_zero_requires_a_non_error_result() {
    // REQ-052: Recognized assistant output without a closing result remains a failure on exit zero.
    let parsed = complete(&[SESSION, TEXT], Some(0)).unwrap();

    assert!(matches!(parsed.outcome, SeatOutcome::Failure(_)));
}

#[test]
fn req_052_turn_and_budget_caps_are_partial() {
    // REQ-052: Exit 53 and 55 preserve usable output as Partial rather than Success or Failure.
    for exit_code in [53, 55] {
        let parsed = complete(&[SESSION, TEXT], Some(exit_code)).unwrap();
        assert!(
            matches!(parsed.outcome, SeatOutcome::Partial(_)),
            "exit {exit_code}"
        );
        assert_eq!(
            parsed.result.response_text,
            "main.rs prints hello and exits."
        );
    }
}

#[test]
fn req_052_sigint_and_other_nonzero_exits_are_failures() {
    // REQ-052: SIGINT, an ordinary nonzero exit, and a missing exit status all map to Failure.
    for exit_code in [Some(130), Some(1), None] {
        let parsed = complete(&[SESSION, TEXT], exit_code).unwrap();
        assert!(
            matches!(parsed.outcome, SeatOutcome::Failure(_)),
            "exit {exit_code:?}"
        );
    }
}

#[test]
fn req_041_missing_usage_is_not_reported_while_reported_zero_is_exact() {
    // REQ-041: Absence stays NotReported, while explicit zero remains Exact(0).
    let without_usage = r#"{"type":"result","subtype":"success","session_id":"session-alpha","is_error":false,"duration_ms":1,"result":"answer"}"#;
    let parsed = complete(&[SESSION, without_usage], Some(0)).unwrap();
    assert!(matches!(parsed.usage, UsageReport::NotReported));
    assert!(parsed.result.token_usage.is_none());

    let zero_usage = r#"{"type":"result","subtype":"success","session_id":"session-alpha","is_error":false,"duration_ms":1,"result":"answer","usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"total_tokens":0}}"#;
    let parsed = complete(&[SESSION, zero_usage], Some(0)).unwrap();
    match parsed.usage {
        UsageReport::Exact(usage) => {
            assert_eq!(usage.input, Some(0));
            assert_eq!(usage.output, Some(0));
            assert_eq!(usage.cached, Some(0));
            assert_eq!(usage.total, Some(0));
        }
        UsageReport::NotReported => panic!("reported zeros must remain exact"),
    }
}

#[test]
fn req_043_served_model_comes_from_assistant_message() {
    // REQ-043: The served model is the assistant message model, not the session-start configured model.
    let message = r#"{"type":"assistant","session_id":"session-alpha","message":{"role":"assistant","model":"glm-5","content":[{"type":"text","text":"cross-lab answer"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
    let result = r#"{"type":"result","subtype":"success","session_id":"session-alpha","is_error":false,"duration_ms":1,"result":"cross-lab answer","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#;
    let parsed = complete(&[SESSION, message, result], Some(0)).unwrap();

    assert_eq!(parsed.served_model.as_deref(), Some("glm-5"));
}
