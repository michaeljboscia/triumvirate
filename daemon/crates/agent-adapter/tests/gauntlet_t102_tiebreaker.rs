// GAUNTLET tiebreaker suite for T-102, written blind from the spec (no implementation read).

use agent_adapter::mistral::{MistralStreamParser, PARSER_MODE};
use agent_adapter::types::{ToolKind, WorkingState};
use agent_adapter::{SeatOutcome, UsageReport};

const STREAM_FIXTURE: &str = include_str!("fixtures/mistral-2.26.1-provisional-20261018.jsonl");
const EXPORT_FIXTURE: &str =
    include_str!("fixtures/mistral-2.26.1-export-provisional-20261018.json");

fn parse_stream(
    stream: &str,
    exit_code: Option<i32>,
    export: Option<&str>,
) -> agent_adapter::SeatParsed {
    let mut parser = MistralStreamParser::new();
    for line in stream.lines() {
        parser.parse_line(line);
    }
    parser.finish(exit_code, export).unwrap()
}

fn export(
    outcome: &str,
    session: &str,
    version: &str,
    usage: (u64, u64, u64, u64),
    cost: f64,
) -> String {
    format!(
        r#"{{"schema_version":1,"outcome":"{outcome}","stop_reason":"end_turn","error":null,"session_id":"{session}","usage":{{"input_tokens":{},"output_tokens":{},"cached_input_tokens":{},"total_tokens":{}}},"cost_usd":{cost},"steps":1,"config":{{"active_model":"redacted"}},"warnings":[],"vibe_version":"{version}","journal_dir":"/tmp/journal","started_at":"2026-10-18T14:00:00Z","ended_at":"2026-10-18T14:00:01Z","limits":{{"max_turns":12,"time_limit":600}}}}"#,
        usage.0, usage.1, usage.2, usage.3
    )
}

#[test]
// REQ-040, REQ-042: the vendor fixtures populate every authoritative result field exactly.
fn fixture_populates_text_read_record_export_telemetry_and_success() {
    let parsed = parse_stream(STREAM_FIXTURE, Some(0), Some(EXPORT_FIXTURE));

    assert_eq!(
        parsed.result.response_text,
        "Reading the file.\nThe add function is correct."
    );
    assert_eq!(parsed.result.session_id.as_deref(), Some("vibe-sess-7f3a"));
    assert_eq!(parsed.result.parser_mode, PARSER_MODE);
    assert_eq!(parsed.result.cli_version.as_deref(), Some("2.26.1"));
    assert_eq!(parsed.result.self_reported_cost_usd, Some(0.0041));
    assert_eq!(parsed.result.tool_calls.len(), 1);
    let call = &parsed.result.tool_calls[0];
    assert_eq!(call.id.as_deref(), Some("e-004"));
    assert_eq!(call.tool, "read_file");
    assert!(matches!(call.kind, ToolKind::ReadFile));
    assert_eq!(call.success, Some(true));
    assert_eq!(
        call.args_json.as_deref(),
        Some(r#"{"filePath":"/repo/src/lib.rs"}"#)
    );
    assert!(matches!(parsed.outcome, SeatOutcome::Success));
    match parsed.usage {
        UsageReport::Exact(usage) => {
            assert_eq!(usage.input, Some(1834));
            assert_eq!(usage.output, Some(212));
            assert_eq!(usage.cached, Some(640));
            assert_eq!(usage.total, Some(2046));
        }
        UsageReport::NotReported => panic!("fixture export reports exact usage"),
    }
}

#[test]
// REQ-040: export identity and telemetry vary with a second fixture rather than using defaults.
fn second_variant_preserves_its_own_distinct_values() {
    let stream = r#"{"id":"different-message","sessionId":"ignored-stream-id","turnId":"turn-9","generationStatus":"completed","type":"message","role":"assistant","content":[{"type":"text","text":"Distinct answer Ω"}]}"#;
    let export = export(
        "finished",
        "different-session",
        "9.8.7",
        (7, 11, 3, 18),
        1.25,
    );
    let parsed = parse_stream(stream, Some(0), Some(&export));

    assert_eq!(parsed.result.response_text, "Distinct answer Ω");
    assert_eq!(
        parsed.result.session_id.as_deref(),
        Some("different-session")
    );
    assert_eq!(parsed.result.cli_version.as_deref(), Some("9.8.7"));
    assert_eq!(parsed.result.self_reported_cost_usd, Some(1.25));
    match parsed.usage {
        UsageReport::Exact(usage) => {
            assert_eq!(usage.input, Some(7));
            assert_eq!(usage.output, Some(11));
            assert_eq!(usage.cached, Some(3));
            assert_eq!(usage.total, Some(18));
        }
        UsageReport::NotReported => panic!("variant export reports exact usage"),
    }
}

#[test]
// REQ-042: a missing export is an explicit failure and never an empty success.
fn missing_export_is_failure_with_exact_code() {
    let parsed = parse_stream(STREAM_FIXTURE, Some(0), None);
    match parsed.outcome {
        SeatOutcome::Failure(code) => assert_eq!(code, "missing_export:mistral"),
        SeatOutcome::Success => panic!("missing export must not succeed"),
        SeatOutcome::Partial(reason) => panic!("missing export must not be partial: {reason}"),
    }
}

#[test]
// REQ-052: exit code 3 with a partial export outcome retains usable output and its reason.
fn exit_three_turn_limit_is_partial() {
    let export = export(
        "turn_limit",
        "partial-session",
        "2.26.1",
        (1, 2, 0, 3),
        0.01,
    );
    let parsed = parse_stream(STREAM_FIXTURE, Some(3), Some(&export));
    match parsed.outcome {
        SeatOutcome::Partial(reason) => assert_eq!(reason, "turn_limit"),
        SeatOutcome::Success => panic!("turn limit must be partial"),
        SeatOutcome::Failure(code) => panic!("turn limit must retain usable output: {code}"),
    }
    assert_eq!(
        parsed.result.response_text,
        "Reading the file.\nThe add function is correct."
    );
}

#[test]
// REQ-052: structured infrastructure failure overrides a contradictory zero exit code.
fn infrastructure_failure_export_overrides_exit_zero() {
    let export = export(
        "infrastructure_failure",
        "failed-session",
        "2.26.1",
        (1, 0, 0, 1),
        0.0,
    );
    let parsed = parse_stream(STREAM_FIXTURE, Some(0), Some(&export));
    match parsed.outcome {
        SeatOutcome::Failure(code) => assert_eq!(code, "infrastructure_failure"),
        SeatOutcome::Success => panic!("structured failure must override exit zero"),
        SeatOutcome::Partial(reason) => panic!("infrastructure failure is not partial: {reason}"),
    }
}

#[test]
// REQ-052: usage and configuration exit code 1 is a failure when the outcome is unrecognised.
fn exit_one_with_unknown_outcome_is_failure() {
    let export = export("future_vendor_outcome", "s1", "2.26.1", (1, 0, 0, 1), 0.0);
    let parsed = parse_stream("", Some(1), Some(&export));
    assert!(matches!(parsed.outcome, SeatOutcome::Failure(_)));
}

#[test]
// REQ-052: infrastructure exit code 2 is a failure when the outcome is unrecognised.
fn exit_two_with_unknown_outcome_is_failure() {
    let export = export("future_vendor_outcome", "s2", "2.26.1", (1, 0, 0, 1), 0.0);
    let parsed = parse_stream("", Some(2), Some(&export));
    assert!(matches!(parsed.outcome, SeatOutcome::Failure(_)));
}

#[test]
// REQ-040: reasoning and user messages never contaminate the assistant answer.
fn only_assistant_text_is_appended_to_the_answer() {
    let export = export("finished", "s3", "2.26.1", (1, 1, 0, 2), 0.0);
    let parsed = parse_stream(STREAM_FIXTURE, Some(0), Some(&export));
    assert!(!parsed.result.response_text.contains("Review src/lib.rs"));
    assert!(
        !parsed
            .result
            .response_text
            .contains("I should read the file first")
    );
    assert_eq!(
        parsed.result.response_text,
        "Reading the file.\nThe add function is correct."
    );
}

#[test]
// REQ-044: malformed JSON, partial lines, and unknown entry types are ignored without hiding valid entries.
fn malformed_partial_and_unknown_lines_do_not_corrupt_valid_output() {
    let valid = r#"{"id":"ok","sessionId":"ignored","turnId":"t","generationStatus":"completed","type":"message","role":"assistant","content":[{"type":"text","text":"survives"}]}"#;
    let stream = format!(
        "{{not json}}\n{{\"id\":\"partial\",\"type\":\"message\"\n{{\"id\":\"new\",\"type\":\"vendor_future\",\"content\":[{{\"type\":\"text\",\"text\":\"must not leak\"}}]}}\n{valid}"
    );
    let export = export("finished", "s4", "2.26.1", (1, 1, 0, 2), 0.0);
    let parsed = parse_stream(&stream, Some(0), Some(&export));
    assert_eq!(parsed.result.response_text, "survives");
    assert_eq!(parsed.result.tool_calls.len(), 0);
}

#[test]
// REQ-044: vendor keys are case-sensitive, so incorrectly cased fields cannot create evidence.
fn incorrectly_cased_fields_are_not_accepted_as_valid_entries() {
    let stream = r#"{"id":"wrong-case","sessionid":"forged","turnid":"t","generationstatus":"completed","Type":"message","Role":"assistant","Content":[{"Type":"text","Text":"must not appear"}]}"#;
    let export = export("finished", "authoritative", "2.26.1", (0, 0, 0, 0), 0.0);
    let parsed = parse_stream(stream, Some(0), Some(&export));
    assert_eq!(parsed.result.response_text, "");
    assert_eq!(parsed.result.session_id.as_deref(), Some("authoritative"));
    assert_eq!(parsed.result.tool_calls.len(), 0);
}

#[test]
// REQ-040, REQ-063: a completed effect emits read evidence with camelCase filePath preserved.
fn completed_read_effect_emits_working_state_and_tool_evidence() {
    let line = STREAM_FIXTURE.lines().nth(3).unwrap();
    let mut parser = MistralStreamParser::new();
    let event = parser
        .parse_line(line)
        .expect("completed effect must emit an event");
    assert_eq!(event.agent, "mistral");
    assert!(matches!(event.state, WorkingState::ToolCallCompleted));
    assert_eq!(event.tool_name.as_deref(), Some("read_file"));
    assert_eq!(
        event.tool_args_json.as_deref(),
        Some(r#"{"filePath":"/repo/src/lib.rs"}"#)
    );
}
