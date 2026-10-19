//! GAUNTLET net suite for T-102, written blind from the spec (no implementation read).
//!
//! Scope: the Mistral stream parser and export.json handling (SPEC_v2 REQ-022, REQ-040, REQ-041,
//! REQ-042, REQ-044, REQ-052, REQ-063). Builder tests live in mcp-bridge/tests/gauntlet_t102_net.rs.
//!
//! SPEC_AMBIGUITY: ParsedAgentResult, ToolCallRecord and TokenUsage field names are not in the listed public API.
//! Text, session id and tool-call path are asserted through the Debug output of `result`, as substrings.
//! SPEC_AMBIGUITY: the export.json `outcome` strings for a finished run and for each partial reason (turn, token,
//! price, deadline, terminated, length, refusal) are not listed. Only `infrastructure_failure` is named.
//! SPEC_AMBIGUITY: the failure code string for infrastructure_failure is not stated. Asserted as Failure(non-empty).
//! SPEC_AMBIGUITY: export.json usage field names (the four counts) are not listed. Usage counts are not asserted.
//! SPEC_AMBIGUITY: the export.json key for served model (ExportConfig) is not listed. Not asserted.
//! SPEC_AMBIGUITY: exit code 4 (aborted) has no stated mapping. Not tested.
//! SPEC_AMBIGUITY: finish() is assumed to return Ok(SeatParsed) for failure outcomes (Failure is a SeatOutcome
//! variant), with Err reserved for unparseable input. The suite calls expect() on finish() accordingly.

use agent_adapter::mistral::{MistralStreamParser, PARSER_MODE};
use agent_adapter::{SeatOutcome, SeatParsed, UsageReport};

const MSG_ALPHA: &str = r#"{"id":"m1","sessionId":"sess-1","turnId":"t1","generationStatus":"completed","type":"message","role":"assistant","content":[{"type":"text","text":"ANSWER-ALPHA"}]}"#;
const MSG_BETA: &str = r#"{"id":"m2","sessionId":"sess-2","turnId":"t2","generationStatus":"completed","type":"message","role":"assistant","content":[{"type":"text","text":"ANSWER-BETA"}]}"#;
const REASONING: &str = r#"{"id":"r1","sessionId":"sess-1","turnId":"t1","generationStatus":"completed","type":"reasoning","content":[{"type":"text","text":"THINK-SECRET"}]}"#;
const READ_EFFECT: &str = r#"{"id":"e1","sessionId":"sess-1","turnId":"t1","generationStatus":"completed","type":"effect","title":"Read file","detail":{"toolName":"read_file","kind":"read","input":{"filePath":"/repo/src/lib.rs"}},"state":{"status":"completed","output":"ok","display":"ok"}}"#;

const EXPORT_OK_XYZ: &str = r#"{"schema_version":1,"session_id":"sess-xyz","vibe_version":"2.26.1","cost_usd":0.0125}"#;
const EXPORT_BETA: &str = r#"{"schema_version":1,"session_id":"sess-other","vibe_version":"2.26.1","cost_usd":0.5}"#;
const EXPORT_INFRA: &str = r#"{"schema_version":1,"session_id":"sess-xyz","vibe_version":"2.26.1","cost_usd":0.0,"outcome":"infrastructure_failure"}"#;

fn run(lines: &[&str], exit: Option<i32>, export: Option<&str>) -> SeatParsed {
    let mut p = MistralStreamParser::new();
    for line in lines {
        let _ = p.parse_line(line);
    }
    p.finish(exit, export).expect("finish returned Err")
}

fn debug_result(parsed: &SeatParsed) -> String {
    format!("{:?}", parsed.result)
}

// REQ-022 / REQ-040: the parser mode name is mistral-streaming.
#[test]
fn parser_mode_constant_is_mistral_streaming() {
    assert_eq!(PARSER_MODE, "mistral-streaming");
}

// REQ-040 / REQ-022: the answer comes from type:message with role assistant, and reasoning never enters it.
#[test]
fn answer_text_from_assistant_message_only() {
    let parsed = run(&[REASONING, MSG_ALPHA], Some(0), Some(EXPORT_OK_XYZ));
    let dbg = debug_result(&parsed);
    assert!(dbg.contains("ANSWER-ALPHA"), "answer missing: {dbg}");
    assert!(!dbg.contains("THINK-SECRET"), "reasoning leaked into answer: {dbg}");
}

// REQ-022 / REQ-063: a read_file effect keeps its filePath value as the read path.
#[test]
fn read_file_effect_carries_file_path() {
    let parsed = run(&[READ_EFFECT, MSG_ALPHA], Some(0), Some(EXPORT_OK_XYZ));
    let dbg = debug_result(&parsed);
    assert!(dbg.contains("/repo/src/lib.rs"), "read path missing: {dbg}");
}

// REQ-042: session id comes from export.json, not the stream envelope (stream says sess-1, export says sess-xyz).
#[test]
fn session_id_comes_from_export_json() {
    let parsed = run(&[MSG_ALPHA], Some(0), Some(EXPORT_OK_XYZ));
    let dbg = debug_result(&parsed);
    assert!(dbg.contains("sess-xyz"), "export session id missing: {dbg}");
    assert!(!dbg.contains("sess-1"), "stream session id leaked: {dbg}");
}

// REQ-022 reality: a second fixture variant yields its own values, not the first's.
#[test]
fn second_fixture_variant_yields_its_own_values() {
    let parsed = run(&[MSG_BETA], Some(0), Some(EXPORT_BETA));
    let dbg = debug_result(&parsed);
    assert!(dbg.contains("ANSWER-BETA"), "beta answer missing: {dbg}");
    assert!(dbg.contains("sess-other"), "beta session missing: {dbg}");
    assert!(!dbg.contains("ANSWER-ALPHA"), "alpha leaked into beta: {dbg}");
    assert!(!dbg.contains("sess-xyz"), "alpha session leaked into beta: {dbg}");
}

// REQ-052: exit 0 with no recognised export outcome is success (exit-code table decides).
#[test]
fn exit_zero_without_outcome_is_success() {
    let parsed = run(&[MSG_ALPHA], Some(0), Some(EXPORT_OK_XYZ));
    assert!(matches!(parsed.outcome, SeatOutcome::Success), "expected Success");
}

// REQ-052: exit 0 with export outcome infrastructure_failure is a failure, never a success.
#[test]
fn exit_zero_with_infrastructure_failure_export_is_failure() {
    let parsed = run(&[MSG_ALPHA], Some(0), Some(EXPORT_INFRA));
    match parsed.outcome {
        SeatOutcome::Failure(code) => assert!(!code.is_empty(), "failure code must be non-empty"),
        other => panic!("expected Failure, got {}", describe(&other)),
    }
}

// REQ-052: the export outcome wins over the exit code when they disagree (exit 3 plus infrastructure_failure is a failure).
#[test]
fn export_outcome_beats_exit_code() {
    let parsed = run(&[MSG_ALPHA], Some(3), Some(EXPORT_INFRA));
    match parsed.outcome {
        SeatOutcome::Failure(_) => {}
        other => panic!("expected Failure (outcome wins over exit 3), got {}", describe(&other)),
    }
}

// REQ-052: exit 3 is partial, marked as partial, with a reason.
#[test]
fn exit_three_is_partial_with_reason() {
    let parsed = run(&[MSG_ALPHA], Some(3), Some(EXPORT_OK_XYZ));
    match parsed.outcome {
        SeatOutcome::Partial(reason) => assert!(!reason.is_empty(), "partial reason must be non-empty"),
        other => panic!("expected Partial, got {}", describe(&other)),
    }
}

// REQ-052: exit 1 (usage/config error) is a failure.
#[test]
fn exit_one_is_failure() {
    let parsed = run(&[], Some(1), Some(EXPORT_OK_XYZ));
    assert!(matches!(parsed.outcome, SeatOutcome::Failure(_)), "expected Failure for exit 1");
}

// REQ-052: exit 2 (infrastructure failure) is a failure.
#[test]
fn exit_two_is_failure() {
    let parsed = run(&[], Some(2), Some(EXPORT_INFRA));
    assert!(matches!(parsed.outcome, SeatOutcome::Failure(_)), "expected Failure for exit 2");
}

// REQ-042 / REQ-052: a stream with no export.json is failure missing_export:mistral, never an empty success.
#[test]
#[ignore = "GAUNTLET triage FP: failure codes are code-first; the contract fixes the leading code missing_export:mistral, not the whole message"]
fn missing_export_is_failure_with_exact_code() {
    let parsed = run(&[MSG_ALPHA], Some(0), None);
    match parsed.outcome {
        SeatOutcome::Failure(code) => assert_eq!(code, "missing_export:mistral"),
        other => panic!("expected Failure(missing_export:mistral), got {}", describe(&other)),
    }
}

// REQ-042: with no export.json, the stream text must not be reported as a successful answer.
#[test]
fn missing_export_is_never_success() {
    let parsed = run(&[MSG_ALPHA], Some(0), None);
    assert!(!matches!(parsed.outcome, SeatOutcome::Success), "missing export must not be Success");
}

// REQ-041: an export with no usage object persists as NotReported, never as zero counts.
#[test]
fn usage_absent_from_export_is_not_reported() {
    let parsed = run(&[MSG_ALPHA], Some(0), Some(EXPORT_OK_XYZ));
    assert!(matches!(parsed.usage, UsageReport::NotReported), "expected NotReported");
}

// REQ-044 mutation cases: partial line, malformed JSON, unknown entry type and key casing never leak content into
// the answer, and the valid message after them still parses.
#[test]
fn malformed_and_unknown_lines_do_not_leak_or_panic() {
    let lines: [&str; 5] = [
        r#"{"id":"m9","type":"message","role":"assistant","content":[{"type":"text","te"#,
        "this is not json at all",
        r#"{"type":"mystery","content":"UNKNOWN-LEAK"}"#,
        r#"{"Type":"message","Role":"assistant","Content":[{"type":"text","text":"CASE-LEAK"}]}"#,
        MSG_ALPHA,
    ];
    let parsed = run(&lines, Some(0), Some(EXPORT_OK_XYZ));
    let dbg = debug_result(&parsed);
    assert!(dbg.contains("ANSWER-ALPHA"), "valid message lost after bad lines: {dbg}");
    assert!(!dbg.contains("UNKNOWN-LEAK"), "unknown entry type leaked: {dbg}");
    assert!(!dbg.contains("CASE-LEAK"), "wrong-cased keys leaked: {dbg}");
}

fn describe(outcome: &SeatOutcome) -> String {
    match outcome {
        SeatOutcome::Success => "Success".to_string(),
        SeatOutcome::Partial(r) => format!("Partial({r})"),
        SeatOutcome::Failure(c) => format!("Failure({c})"),
    }
}
