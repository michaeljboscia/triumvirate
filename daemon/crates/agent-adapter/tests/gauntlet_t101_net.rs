// GAUNTLET net suite for T-101, written blind from the spec (no implementation read).
// SPEC_AMBIGUITY: ParsedAgentResult, ToolCallRecord, TokenUsage and WorkingStateEvent field names are not in the package, so the response text, session id, tool record and cli_version are not asserted.
// SPEC_AMBIGUITY: the spec places turn.step.retrying's status_code at top level in the line object. Nested placement is not stated, so top level is assumed.
// SPEC_AMBIGUITY: no accessor for the session id is listed, so the missing resume_hint case (no fabricated id) is not asserted.
// SPEC_AMBIGUITY: the spec does not say whether no_exit_code:kimi also takes the last_retry_status suffix ("both failure codes" is unclear). Not asserted.
// SPEC_AMBIGUITY: parse_line's returned WorkingStateEvent has no listed shape, so return values are asserted only for malformed input (None).

use agent_adapter::kimi::KimiStreamParser;
use agent_adapter::{SeatOutcome, SeatParsed, UsageReport};

fn parse(lines: &[&str], exit: Option<i32>) -> SeatParsed {
    let mut p = KimiStreamParser::new();
    for line in lines {
        let _ = p.parse_line(line);
    }
    p.finish(exit).expect("kimi finish returns Ok; outcomes live in SeatParsed")
}

fn outcome(p: &SeatParsed) -> String {
    match &p.outcome {
        SeatOutcome::Success => "Success".to_string(),
        SeatOutcome::Partial(s) => format!("Partial({s})"),
        SeatOutcome::Failure(s) => format!("Failure({s})"),
    }
}

const TEXT: &str = r#"{"role":"assistant","content":"The answer is 42."}"#;
const RETRY_429: &str = r#"{"role":"meta","type":"turn.step.retrying","status_code":429}"#;
const RETRY_503: &str = r#"{"role":"meta","type":"turn.step.retrying","status_code":503}"#;

// REQ-052: exit 0 with no assistant text is failure `empty_or_unrecognized_stream:kimi`.
#[test]
fn empty_stream_exit_zero_is_empty_failure() {
    let p = parse(&[], Some(0));
    assert_eq!(outcome(&p), "Failure(empty_or_unrecognized_stream:kimi)");
}

// REQ-052: a non-zero exit with no text is `nonzero_exit:kimi:<code>`, not the empty code.
#[test]
fn empty_stream_nonzero_exit_is_nonzero_failure() {
    let p = parse(&[], Some(2));
    assert_eq!(outcome(&p), "Failure(nonzero_exit:kimi:2)");
}

// REQ-052: exit 0 with assistant text is Success.
#[test]
fn text_with_exit_zero_is_success() {
    let p = parse(&[TEXT], Some(0));
    assert_eq!(outcome(&p), "Success");
}

// REQ-052: text with a non-zero exit is `nonzero_exit:kimi:1` with no retry suffix when no retry was seen.
#[test]
fn text_with_nonzero_exit_is_nonzero_failure() {
    let p = parse(&[TEXT], Some(1));
    assert_eq!(outcome(&p), "Failure(nonzero_exit:kimi:1)");
}

// REQ-052: no exit code (killed by a signal) is `no_exit_code:kimi`.
#[test]
fn no_exit_code_is_no_exit_code_failure() {
    let p = parse(&[TEXT], None);
    assert_eq!(outcome(&p), "Failure(no_exit_code:kimi)");
}

// REQ-041: usage is NotReported and served_model is None on every outcome (research/01: no usage, no model in the stream).
#[test]
fn usage_is_not_reported_and_model_unknown() {
    for (lines, exit) in [
        (vec![TEXT], Some(0)),
        (vec![TEXT], Some(1)),
        (vec![], Some(0)),
        (vec![], None),
    ] {
        let p = parse(&lines, exit);
        assert!(matches!(p.usage, UsageReport::NotReported), "usage was not NotReported");
        assert_eq!(p.served_model, None);
    }
}

// REQ-052 (T-205 classifier contract): a retry seen before a non-zero exit appends ` last_retry_status=429`.
#[test]
fn retry_status_is_appended_to_nonzero_failure() {
    let p = parse(&[RETRY_429, TEXT], Some(1));
    assert_eq!(outcome(&p), "Failure(nonzero_exit:kimi:1 last_retry_status=429)");
}

// REQ-052: the suffix carries the most recent retry's status, not the first.
#[test]
fn retry_suffix_uses_most_recent_status() {
    let p = parse(&[RETRY_429, RETRY_503], Some(1));
    assert_eq!(outcome(&p), "Failure(nonzero_exit:kimi:1 last_retry_status=503)");
}

// REQ-052: a retry seen with an empty exit-0 stream appends the suffix to the empty-stream code.
#[test]
fn retry_suffix_is_appended_to_empty_failure() {
    let p = parse(&[RETRY_429], Some(0));
    assert_eq!(
        outcome(&p),
        "Failure(empty_or_unrecognized_stream:kimi last_retry_status=429)"
    );
}

// REQ-052: Success carries no retry suffix even when a retry was seen.
#[test]
fn retry_then_text_with_exit_zero_is_plain_success() {
    let p = parse(&[RETRY_429, TEXT], Some(0));
    assert_eq!(outcome(&p), "Success");
}

// REQ-044: partial and malformed JSON lines are skipped and never abort the parse; the later valid line still gives Success.
#[test]
fn partial_and_malformed_lines_are_skipped() {
    let p = parse(
        &[
            r#"{"role":"assistant","content":"par"#,
            "not json at all",
            r#"{"role":"assistant","content":"tru"#,
            TEXT,
        ],
        Some(0),
    );
    assert_eq!(outcome(&p), "Success");
}

// REQ-044: a stream of only malformed lines with exit 0 is the empty failure, never a fabricated success.
#[test]
fn only_malformed_lines_is_empty_failure() {
    let p = parse(&["{not json", "}}}", "[1,2"], Some(0));
    assert_eq!(outcome(&p), "Failure(empty_or_unrecognized_stream:kimi)");
}

// REQ-044: parse_line returns None for a malformed line and does not panic.
#[test]
fn malformed_line_returns_no_event() {
    let mut p = KimiStreamParser::new();
    assert!(p.parse_line("{not json").is_none());
}

// REQ-044: an unknown meta type is ignored, so a stream with only that line is the empty failure.
#[test]
fn unknown_meta_type_is_ignored() {
    let p = parse(&[r#"{"role":"meta","type":"future.kind.v9","x":1}"#], Some(0));
    assert_eq!(outcome(&p), "Failure(empty_or_unrecognized_stream:kimi)");
}

// REQ-044: an unknown meta type between known lines does not change a Success.
#[test]
fn unknown_meta_type_between_text_still_succeeds() {
    let p = parse(
        &[r#"{"role":"meta","type":"future.kind.v9"}"#, TEXT],
        Some(0),
    );
    assert_eq!(outcome(&p), "Success");
}

// REQ-044: an unknown role is ignored, so it contributes no text.
#[test]
fn unknown_role_contributes_no_text() {
    let p = parse(&[r#"{"role":"narrator","content":"hi"}"#], Some(0));
    assert_eq!(outcome(&p), "Failure(empty_or_unrecognized_stream:kimi)");
}

// REQ-040/052: a tool result carries no outcome (vendor issue #4241), so a tool-only exit-0 stream has no text and is the empty failure.
#[test]
fn tool_result_alone_is_empty_failure() {
    let p = parse(
        &[r#"{"role":"tool","tool_call_id":"c1","content":"file body"}"#],
        Some(0),
    );
    assert_eq!(outcome(&p), "Failure(empty_or_unrecognized_stream:kimi)");
}

// REQ-040/052: an assistant line with empty content and a tool call has no text, so exit 0 is the empty failure.
#[test]
fn tool_call_with_empty_content_is_empty_failure() {
    let p = parse(
        &[r#"{"role":"assistant","content":"","tool_calls":[{"id":"c1","function":{"name":"Read","arguments":"{\"path\":\"/x/y.txt\"}"}}]}"#],
        Some(0),
    );
    assert_eq!(outcome(&p), "Failure(empty_or_unrecognized_stream:kimi)");
}

// REQ-040: a tool call followed by answer text is Success with usage NotReported.
#[test]
fn tool_call_then_text_is_success() {
    let p = parse(
        &[
            r#"{"role":"assistant","content":"","tool_calls":[{"id":"c1","function":{"name":"Read","arguments":"{\"path\":\"/x/y.txt\"}"}}]}"#,
            r#"{"role":"tool","tool_call_id":"c1","content":"file body"}"#,
            r#"{"role":"assistant","content":"The file says hello."}"#,
        ],
        Some(0),
    );
    assert_eq!(outcome(&p), "Success");
    assert!(matches!(p.usage, UsageReport::NotReported));
}

// REQ-052/044: a session.resume_hint meta line does not change the outcome; text plus exit 0 is Success.
#[test]
fn resume_hint_line_does_not_change_outcome() {
    let p = parse(
        &[
            r#"{"role":"meta","type":"system.version","version":"2.2.0"}"#,
            TEXT,
            r#"{"role":"meta","type":"session.resume_hint","session_id":"sess-7f3a"}"#,
        ],
        Some(0),
    );
    assert_eq!(outcome(&p), "Success");
}

// REQ-044: two different answers both succeed, and each carries NotReported usage (the spec's second-variant check; values are not asserted, see SPEC_AMBIGUITY).
#[test]
fn second_variant_with_different_answer_succeeds() {
    let p = parse(
        &[r#"{"role":"assistant","content":"A different answer, 7 apples."}"#],
        Some(0),
    );
    assert_eq!(outcome(&p), "Success");
    assert!(matches!(p.usage, UsageReport::NotReported));
    assert_eq!(p.served_model, None);
}
