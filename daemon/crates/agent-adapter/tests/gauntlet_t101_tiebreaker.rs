// GAUNTLET tiebreaker suite for T-101, written blind from the spec (no implementation read).

use agent_adapter::kimi::{KimiStreamParser, PARSER_MODE};
use agent_adapter::{SeatOutcome, ToolKind, UsageReport, WorkingState};

fn parse(lines: &[&str], exit_code: Option<i32>) -> agent_adapter::SeatParsed {
    let mut parser = KimiStreamParser::new();
    for line in lines {
        parser.parse_line(line);
    }
    parser
        .finish(exit_code)
        .expect("parser finish must succeed")
}

#[test]
fn fixture_values_are_parsed_field_by_field() {
    // REQ-040, REQ-041, and REQ-063: real stream values populate every required result field.
    let parsed = parse(
        &[
            r#"{"role":"meta","type":"system.version","version":"2.2.0"}"#,
            r#"{"role":"assistant","content":"Answer unique alpha","tool_calls":[{"type":"function","id":"call-alpha","function":{"name":"Read","arguments":"{\"file_path\":\"/fixture/alpha.txt\"}"}}]}"#,
            r#"{"role":"tool","tool_call_id":"call-alpha","content":"alpha contents"}"#,
            r#"{"role":"meta","type":"session.resume_hint","session_id":"session-alpha-101","command":"kimi -r session-alpha-101","content":"resume"}"#,
        ],
        Some(0),
    );

    assert_eq!(parsed.result.response_text, "Answer unique alpha");
    assert_eq!(
        parsed.result.session_id.as_deref(),
        Some("session-alpha-101")
    );
    assert_eq!(parsed.result.cli_version.as_deref(), Some("2.2.0"));
    assert_eq!(parsed.result.parser_mode, PARSER_MODE);
    assert_eq!(parsed.result.tool_calls.len(), 1);
    let tool = &parsed.result.tool_calls[0];
    assert_eq!(tool.tool, "Read");
    assert_eq!(tool.kind, ToolKind::ReadFile);
    assert_eq!(tool.id.as_deref(), Some("call-alpha"));
    assert_eq!(
        tool.args_json.as_deref(),
        Some(r#"{"file_path":"/fixture/alpha.txt"}"#)
    );
    assert_eq!(tool.success, None);
    assert!(matches!(parsed.usage, UsageReport::NotReported));
    assert!(matches!(parsed.outcome, SeatOutcome::Success));
    assert_eq!(parsed.served_model, None);
}

#[test]
fn a_second_variant_proves_values_are_not_constants() {
    // REQ-040 reality test: different text, path, and session id produce different output.
    let parsed = parse(
        &[
            r#"{"role":"assistant","content":"Answer unique beta","tool_calls":[{"type":"function","id":"call-beta","function":{"name":"Read","arguments":"{\"path\":\"/fixture/beta.rs\"}"}}]}"#,
            r#"{"role":"meta","type":"session.resume_hint","session_id":"session-beta-202","command":"kimi -r session-beta-202","content":"resume"}"#,
        ],
        Some(0),
    );

    assert_eq!(parsed.result.response_text, "Answer unique beta");
    assert_eq!(
        parsed.result.session_id.as_deref(),
        Some("session-beta-202")
    );
    assert_eq!(parsed.result.tool_calls.len(), 1);
    assert_eq!(
        parsed.result.tool_calls[0].args_json.as_deref(),
        Some(r#"{"path":"/fixture/beta.rs"}"#)
    );
}

#[test]
fn malformed_partial_and_unknown_lines_do_not_discard_valid_content() {
    // REQ-044: malformed JSON, a partial line, and unknown metadata are skipped.
    let parsed = parse(
        &[
            r#"{"role":"meta","type":"future.unknown","value":99}"#,
            "not json at all",
            r#"{"role":"assistant","content":"surviving text"}"#,
            r#"{"role":"meta","type":"session.resume_hint","session_id":"session-survives"}"#,
            r#"{"role":"assistant","content":"unterminated"#,
        ],
        Some(0),
    );

    assert_eq!(parsed.result.response_text, "surviving text");
    assert_eq!(
        parsed.result.session_id.as_deref(),
        Some("session-survives")
    );
    assert!(matches!(parsed.outcome, SeatOutcome::Success));
}

#[test]
fn missing_resume_hint_never_fabricates_a_session_id() {
    // REQ-044 and vendor issue 4240: session identity comes only from resume_hint.
    let parsed = parse(
        &[r#"{"role":"assistant","content":"completed without hint"}"#],
        Some(0),
    );

    assert_eq!(parsed.result.session_id, None);
    assert_eq!(parsed.result.response_text, "completed without hint");
}

#[test]
fn assistant_chunks_are_joined_in_stream_order() {
    // REQ-040: non-empty assistant chunks retain stream order.
    let parsed = parse(
        &[
            r#"{"role":"assistant","content":"first"}"#,
            r#"{"role":"assistant","content":"second"}"#,
        ],
        Some(0),
    );

    assert_eq!(parsed.result.response_text, "first\nsecond");
}

#[test]
fn empty_exit_zero_is_the_exact_failure() {
    // REQ-052: exit zero without assistant text is not success.
    let parsed = parse(&[], Some(0));
    match parsed.outcome {
        SeatOutcome::Failure(code) => assert_eq!(code, "empty_or_unrecognized_stream:kimi"),
        _ => panic!("expected empty stream failure"),
    }
}

#[test]
fn nonzero_exit_is_the_exact_failure() {
    // REQ-052: a nonzero process exit maps to its exact structured failure.
    let parsed = parse(
        &[r#"{"role":"assistant","content":"text before failure"}"#],
        Some(7),
    );
    match parsed.outcome {
        SeatOutcome::Failure(code) => assert_eq!(code, "nonzero_exit:kimi:7"),
        _ => panic!("expected nonzero exit failure"),
    }
}

#[test]
fn missing_exit_code_is_the_exact_failure() {
    // REQ-052: signal termination maps to the exact no-exit-code failure.
    let parsed = parse(
        &[r#"{"role":"assistant","content":"text before signal"}"#],
        None,
    );
    match parsed.outcome {
        SeatOutcome::Failure(code) => assert_eq!(code, "no_exit_code:kimi"),
        _ => panic!("expected missing exit code failure"),
    }
}

#[test]
fn latest_retry_status_is_appended_to_terminal_failures() {
    // REQ-052 and BACKEND_STRUCTURE section 6: the latest retry status feeds classification.
    let parsed = parse(
        &[
            r#"{"role":"meta","type":"turn.step.retrying","failed_attempt":1,"next_attempt":2,"max_attempts":3,"delay_ms":10,"error_name":"RateLimit","error_message":"retry","status_code":503}"#,
            r#"{"role":"meta","type":"turn.step.retrying","failed_attempt":2,"next_attempt":3,"max_attempts":3,"delay_ms":20,"error_name":"RateLimit","error_message":"quota","status_code":429}"#,
        ],
        Some(1),
    );
    match parsed.outcome {
        SeatOutcome::Failure(code) => {
            assert_eq!(code, "nonzero_exit:kimi:1 last_retry_status=429")
        }
        _ => panic!("expected retry terminal failure"),
    }
}

#[test]
fn tool_start_and_completion_events_are_exact() {
    // REQ-040 and vendor issue 4241: tool events preserve identity and report no invented success.
    let mut parser = KimiStreamParser::new();
    let started = parser.parse_line(
        r#"{"role":"assistant","content":"","tool_calls":[{"type":"function","id":"call-read","function":{"name":"Read","arguments":"{\"file_path\":\"/fixture/event.txt\"}"}}]}"#,
    );
    let completed =
        parser.parse_line(r#"{"role":"tool","tool_call_id":"call-read","content":"contents"}"#);

    assert!(matches!(
        started.map(|event| event.state),
        Some(WorkingState::ToolCallStarted)
    ));
    assert!(matches!(
        completed.map(|event| event.state),
        Some(WorkingState::ToolCallCompleted)
    ));
}
