//! GAUNTLET net suite for T-103, written blind from the spec (no implementation read).
//!
//! SPEC_AMBIGUITY: ParsedAgentResult, TokenUsage, ToolCallRecord and WorkingStateEvent field names are not in the listed
//!   API. session_id from session_start, exact usage numbers, assistant text, and the ReadFile tool record under a
//!   READ_OPERAND_KEYS key cannot be asserted without guessing field names. Only served_model, outcome and the
//!   Exact/NotReported usage variant are asserted.
//! SPEC_AMBIGUITY: the outcome shape for a finish() failure is not fixed (Err(code) or Ok with SeatOutcome::Failure(code)).
//!   The helper accepts either form, but the code string must match exactly.
//! SPEC_AMBIGUITY: exit 130 and exit 1 (anything else) are "an error" per the spec. Not fixed as Failure or Err. Asserted
//!   only as not Success.
//! SPEC_AMBIGUITY: the REQ-044 key-casing mutation is asserted as "an uppercase key is not a recognized event" (inferred
//!   from REQ-044 and REQ-040, not stated as an exact result).
//! SPEC_AMBIGUITY: the "missing id" mutation cannot be observed through the listed API, so it is not tested.

use agent_adapter::qwen::{QwenStreamParser, PARSER_MODE};
use agent_adapter::{SeatOutcome, SeatParsed, UsageReport};

const SESSION_START: &str = r#"{"type":"system","subtype":"session_start","session_id":"7f3c9a2e-5b1d-4e8a-9c40-2d6f1e8b7a01","model":"qwen3-coder-plus"}"#;
const ASSISTANT_QWEN: &str = r#"{"type":"assistant","session_id":"7f3c9a2e-5b1d-4e8a-9c40-2d6f1e8b7a01","message":{"role":"assistant","model":"qwen3-coder-plus","content":[{"type":"text","text":"done"}],"usage":{"input_tokens":1400,"output_tokens":12}}}"#;
const RESULT_OK: &str = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":10,"result":"done","usage":{"input_tokens":2600,"output_tokens":52,"total_tokens":2652}}"#;
const RESULT_ERR: &str = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"duration_ms":10,"result":"quota"}"#;

// Second variant: a different served model and different numbers.
const SESSION_START_B: &str = r#"{"type":"system","subtype":"session_start","session_id":"aaaa1111-2222-4333-8444-555566667777","model":"qwen3-max"}"#;
const ASSISTANT_B: &str = r#"{"type":"assistant","session_id":"aaaa1111-2222-4333-8444-555566667777","message":{"role":"assistant","model":"qwen3-max","content":[{"type":"text","text":"other"}],"usage":{"input_tokens":77,"output_tokens":9}}}"#;
const RESULT_B: &str = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":5,"result":"other","usage":{"input_tokens":77,"output_tokens":9,"total_tokens":86}}"#;

// Served model that is not Qwen.
const ASSISTANT_GLM: &str = r#"{"type":"assistant","session_id":"7f3c9a2e-5b1d-4e8a-9c40-2d6f1e8b7a01","message":{"role":"assistant","model":"glm-5","content":[{"type":"text","text":"x"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;

fn run(lines: &[&str], exit: Option<i32>) -> Result<SeatParsed, String> {
    let mut p = QwenStreamParser::new();
    for l in lines {
        let _ = p.parse_line(l);
    }
    p.finish(exit)
}

/// The failure code of a finish() result, in either shape the spec allows.
fn failure_code(r: &Result<SeatParsed, String>) -> Option<String> {
    match r {
        Err(s) => Some(s.clone()),
        Ok(p) => match &p.outcome {
            SeatOutcome::Failure(s) => Some(s.clone()),
            _ => None,
        },
    }
}

fn is_success(r: &Result<SeatParsed, String>) -> bool {
    matches!(r, Ok(p) if matches!(p.outcome, SeatOutcome::Success))
}

// REQ-040: the parser mode constant is the spec value.
#[test]
fn t103_req040_parser_mode_constant() {
    assert_eq!(PARSER_MODE, "qwen-stream-json");
}

// REQ-040/052: a clean stream closed by a non-error result with exit 0 is a Success.
#[test]
fn t103_req052_clean_stream_exit_zero_is_success() {
    let r = run(&[SESSION_START, ASSISTANT_QWEN, RESULT_OK], Some(0));
    assert!(is_success(&r), "expected Success");
}

// REQ-043: served_model is read from the assistant message.model.
#[test]
fn t103_req043_served_model_from_message_model() {
    let parsed = run(&[SESSION_START, ASSISTANT_QWEN, RESULT_OK], Some(0)).expect("parses");
    assert_eq!(parsed.served_model.as_deref(), Some("qwen3-coder-plus"));
}

// REQ-040: a second fixture variant yields its own served model, not the first variant's.
#[test]
fn t103_req040_second_variant_yields_own_values() {
    let parsed = run(&[SESSION_START_B, ASSISTANT_B, RESULT_B], Some(0)).expect("parses");
    assert_eq!(parsed.served_model.as_deref(), Some("qwen3-max"));
    assert!(is_success(&Ok(parsed)));
}

// REQ-043: a stream answered by glm-5 reports glm-5 as the served model (the check helper refuses it).
#[test]
fn t103_req043_glm5_stream_reports_glm5_served_model() {
    let parsed = run(&[SESSION_START, ASSISTANT_GLM, RESULT_OK], Some(0)).expect("parses");
    assert_eq!(parsed.served_model.as_deref(), Some("glm-5"));
}

// REQ-041: a result carrying a usage object with the Claude-shape keys is Exact.
#[test]
fn t103_req041_reported_usage_is_exact() {
    let parsed = run(&[SESSION_START, ASSISTANT_QWEN, RESULT_OK], Some(0)).expect("parses");
    assert!(matches!(parsed.usage, UsageReport::Exact(_)));
}

// REQ-041: a reported zero stays Exact, never NotReported.
#[test]
fn t103_req041_reported_zero_stays_exact() {
    let zero = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":1,"result":"","usage":{"input_tokens":0,"output_tokens":0,"total_tokens":0}}"#;
    let parsed = run(&[SESSION_START, zero], Some(0)).expect("parses");
    assert!(matches!(parsed.usage, UsageReport::Exact(_)));
}

// REQ-041: a result with no usage object persists as NotReported.
#[test]
fn t103_req041_missing_usage_is_not_reported() {
    let no_usage = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":1,"result":"done"}"#;
    let parsed = run(&[SESSION_START, no_usage], Some(0)).expect("parses");
    assert!(matches!(parsed.usage, UsageReport::NotReported));
}

// REQ-041: a usage object with none of the known keys is not a report.
#[test]
fn t103_req041_usage_without_known_keys_is_not_reported() {
    let odd = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":1,"result":"done","usage":{"unrelated":5}}"#;
    let parsed = run(&[SESSION_START, odd], Some(0)).expect("parses");
    assert!(matches!(parsed.usage, UsageReport::NotReported));
}

// REQ-052: exit 53 (turn cap) is Partial, never Success.
#[test]
fn t103_req052_exit_53_is_partial() {
    let r = run(&[SESSION_START, ASSISTANT_QWEN, RESULT_OK], Some(53));
    assert!(matches!(r, Ok(ref p) if matches!(p.outcome, SeatOutcome::Partial(_))));
}

// REQ-052: exit 55 (run budget) is Partial, never Success.
#[test]
fn t103_req052_exit_55_is_partial() {
    let r = run(&[SESSION_START, ASSISTANT_QWEN, RESULT_OK], Some(55));
    assert!(matches!(r, Ok(ref p) if matches!(p.outcome, SeatOutcome::Partial(_))));
}

// REQ-052: exit 0 with an error result is a failure, not a success.
#[test]
fn t103_req052_exit_zero_with_error_result_is_failure() {
    let r = run(&[SESSION_START, RESULT_ERR], Some(0));
    assert!(!is_success(&r), "error result must not be Success");
    assert!(failure_code(&r).is_some(), "error result must carry a failure");
}

// REQ-052: exit 0 with a stream that never reaches a result is not a success.
#[test]
fn t103_req052_exit_zero_without_result_is_not_success() {
    let r = run(&[SESSION_START, ASSISTANT_QWEN], Some(0));
    assert!(!is_success(&r), "stream without result must not be Success");
}

// REQ-052: exit 130 (SIGINT) is not a success.
#[test]
fn t103_req052_exit_130_is_not_success() {
    let r = run(&[SESSION_START, ASSISTANT_QWEN, RESULT_OK], Some(130));
    assert!(!is_success(&r));
}

// REQ-052: any other exit code (1) is not a success even with a clean stream.
#[test]
fn t103_req052_exit_one_is_not_success() {
    let r = run(&[SESSION_START, ASSISTANT_QWEN, RESULT_OK], Some(1));
    assert!(!is_success(&r));
}

// Spec edge: an empty stream with exit 0 is the exact failure code.
#[test]
fn t103_empty_stream_exit_zero_exact_failure_code() {
    let r = run(&[], Some(0));
    assert_eq!(
        failure_code(&r).as_deref(),
        Some("empty_or_unrecognized_stream:qwen")
    );
}

// REQ-044: blank, partial and malformed lines are skipped and yield no event.
#[test]
fn t103_req044_blank_partial_malformed_lines_skipped() {
    let mut p = QwenStreamParser::new();
    assert!(p.parse_line("").is_none(), "blank line yielded an event");
    assert!(p.parse_line("{\"type\":\"assistant\",\"message\":{").is_none(), "partial line yielded an event");
    assert!(p.parse_line("not json at all").is_none(), "malformed line yielded an event");
}

// REQ-044: an unknown event type is ignored and yields no event.
#[test]
fn t103_req044_unknown_event_type_ignored() {
    let mut p = QwenStreamParser::new();
    assert!(p.parse_line(r#"{"type":"telemetry_ping","value":1}"#).is_none());
}

// REQ-044 (key casing): uppercase keys are not recognized, so a stream of only those lines is unrecognized.
#[test]
fn t103_req044_key_casing_not_recognized() {
    let upper = r#"{"Type":"system","Subtype":"session_start","Session_Id":"x"}"#;
    let r = run(&[upper], Some(0));
    assert_eq!(
        failure_code(&r).as_deref(),
        Some("empty_or_unrecognized_stream:qwen")
    );
}

// REQ-044: a stream made only of malformed lines, exit 0, is the exact unrecognized-stream failure.
#[test]
fn t103_req044_only_malformed_lines_exit_zero_unrecognized() {
    let r = run(&["garbage", "{\"type\":", ""], Some(0));
    assert_eq!(
        failure_code(&r).as_deref(),
        Some("empty_or_unrecognized_stream:qwen")
    );
}
