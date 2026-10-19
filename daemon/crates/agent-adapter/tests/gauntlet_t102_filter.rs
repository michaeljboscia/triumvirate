// GAUNTLET filter suite for T-102, written blind from the spec (no implementation read).
//
// Role: filter. Crate: agent-adapter (Mistral stream parser). The builder half lives in
// daemon/crates/mcp-bridge/tests/gauntlet_t102_filter.rs.
//
// SPEC_AMBIGUITY: REQ-052 / task reality test say a run with no export.json is "failure `missing_export:mistral`",
//   but `finish` returns Result<SeatParsed, String>. Whether that failure is `Err("missing_export:mistral..")` or
//   `Ok(SeatParsed { outcome: SeatOutcome::Failure("missing_export:mistral..") })` is not stated. The tests here accept
//   either and assert only that the code is present and that it is never a Success.
// SPEC_AMBIGUITY: the exact text of a Partial reason or of a Failure message for export outcomes (turn_limit,
//   infrastructure_failure, ...) is not stated. Tests assert the variant and a non-empty reason; one test also assumes
//   the Partial reason names the export outcome ("with the reason").
// SPEC_AMBIGUITY: how several assistant `message` entries are joined into `response_text` (newline? concatenation?) is not
//   stated. Multi-message tests assert order and containment; single-message tests assert exact equality (trimmed).
// SPEC_AMBIGUITY: which working-state event a given line yields (state value, detail text) is not stated beyond
//   "returns a working-state event when the line carries one", so only presence/absence and tool_name are asserted.
// SPEC_AMBIGUITY: an exit code of None (killed by signal) combined with a `finished` export is not specified; not tested.
// SPEC_AMBIGUITY: exit 0 with a `finished` export is Success, but exit 2 with a `finished` export (disagreement in the
//   other direction) is not specified; the test only forbids Success.
// SPEC_AMBIGUITY: the content of ToolCallRecord.tool for a read_file effect is not stated; only ToolKind::ReadFile and
//   args_json are asserted.

use agent_adapter::mistral::{MistralStreamParser, PARSER_MODE};
use agent_adapter::{SeatOutcome, SeatParsed, ToolKind, UsageReport};
use serde_json::{Value, json};

const PARTIAL_OUTCOMES: [&str; 7] = [
    "turn_limit",
    "token_limit",
    "price_limit",
    "deadline",
    "terminated",
    "length",
    "refusal",
];

fn envelope(id: &str, session: &str, ty: &str) -> Value {
    json!({
        "id": id,
        "sessionId": session,
        "turnId": "t-1",
        "createdAt": "2026-10-18T14:00:00Z",
        "updatedAt": "2026-10-18T14:00:01Z",
        "generationStatus": "completed",
        "relatedEntryId": null,
        "type": ty,
    })
}

fn message_line(id: &str, role: &str, text: &str) -> String {
    let mut v = envelope(id, "sess-stream-ignored", "message");
    v["role"] = json!(role);
    v["content"] = json!([{"type": "text", "text": text}]);
    v.to_string()
}

fn reasoning_line(id: &str, text: &str) -> String {
    let mut v = envelope(id, "sess-stream-ignored", "reasoning");
    v["content"] = json!([{"type": "text", "text": text}]);
    v.to_string()
}

fn read_file_line(id: &str, path: &str) -> String {
    let mut v = envelope(id, "sess-stream-ignored", "effect");
    v["title"] = json!("read_file");
    v["detail"] =
        json!({"toolName": "read_file", "kind": "file_read", "input": {"filePath": path}});
    v["state"] =
        json!({"status": "completed", "output": {"content": "x"}, "display": {"success": true}});
    v.to_string()
}

fn export_json(
    outcome: &str,
    session: &str,
    input: u64,
    output: u64,
    cached: u64,
    total: u64,
    cost: f64,
) -> String {
    json!({
        "schema_version": 1,
        "outcome": outcome,
        "stop_reason": "end_turn",
        "error": null,
        "session_id": session,
        "usage": {
            "input_tokens": input,
            "output_tokens": output,
            "cached_input_tokens": cached,
            "total_tokens": total,
        },
        "cost_usd": cost,
        "steps": 3,
        "config": {"active_model": "redacted"},
        "warnings": [],
        "vibe_version": "2.26.1",
        "journal_dir": "/th/homes/mistral/logs/session/x",
        "started_at": "2026-10-18T14:00:00Z",
        "ended_at": "2026-10-18T14:00:06Z",
        "limits": {"max_turns": 12, "time_limit": 600}
    })
    .to_string()
}

fn standard_export(outcome: &str) -> String {
    export_json(outcome, "vibe-sess-7f3a", 1834, 212, 640, 2046, 0.0041)
}

fn parse_all(lines: &[String]) -> MistralStreamParser {
    let mut sut = MistralStreamParser::new();
    for l in lines {
        let _ = sut.parse_line(l);
    }
    sut
}

fn finish(lines: &[String], exit: i32, export: Option<&str>) -> SeatParsed {
    parse_all(lines)
        .finish(Some(exit), export)
        .expect("finish should return Ok")
}

fn fixture_path(name: &str) -> String {
    format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name)
}

fn fixture_lines() -> Vec<String> {
    let text =
        std::fs::read_to_string(fixture_path("mistral-2.26.1-provisional-20261018.jsonl")).unwrap();
    text.lines().map(|l| l.to_string()).collect()
}

fn fixture_export() -> String {
    std::fs::read_to_string(fixture_path(
        "mistral-2.26.1-export-provisional-20261018.json",
    ))
    .unwrap()
}

fn is_failure(r: &Result<SeatParsed, String>) -> Option<String> {
    match r {
        Err(e) => Some(e.clone()),
        Ok(p) => match &p.outcome {
            SeatOutcome::Failure(m) => Some(m.clone()),
            _ => None,
        },
    }
}

fn outcome_label(o: &SeatOutcome) -> String {
    match o {
        SeatOutcome::Success => "Success".into(),
        SeatOutcome::Partial(r) => format!("Partial({r})"),
        SeatOutcome::Failure(m) => format!("Failure({m})"),
    }
}

// ---------- constants ----------

#[test]
fn test_should_name_the_parser_mode_mistral_streaming() {
    // REQ-063 / REQ-040: parser mode label
    assert_eq!(PARSER_MODE, "mistral-streaming");
}

#[test]
fn test_should_record_parser_mode_in_result() {
    // REQ-040: ParsedAgentResult carries parser_mode
    let sut = finish(
        &[message_line("e1", "assistant", "hi")],
        0,
        Some(&standard_export("finished")),
    );
    assert_eq!(sut.result.parser_mode, "mistral-streaming");
}

// ---------- happy path: provisional fixtures ----------

#[test]
fn test_should_extract_session_id_from_export_not_from_stream_when_fixture_parsed() {
    // REQ-042: session id comes from export.json (stream sessionId is "sess-stream-ignored")
    let sut = finish(&fixture_lines(), 0, Some(&fixture_export()));
    assert_eq!(sut.result.session_id.as_deref(), Some("vibe-sess-7f3a"));
}

#[test]
fn test_should_return_assistant_text_when_fixture_parsed() {
    // REQ-040: response text is the assistant message text; user and reasoning entries excluded
    let sut = finish(&fixture_lines(), 0, Some(&fixture_export()));
    let text = sut.result.response_text.clone();
    assert!(text.contains("Reading the file."), "text was {text:?}");
    assert!(
        text.contains("The add function is correct."),
        "text was {text:?}"
    );
    assert!(
        !text.contains("Review src/lib.rs"),
        "user message leaked: {text:?}"
    );
    assert!(
        !text.contains("I should read the file first."),
        "reasoning leaked: {text:?}"
    );
    assert!(
        text.find("Reading the file.").unwrap()
            < text.find("The add function is correct.").unwrap()
    );
}

#[test]
fn test_should_record_one_readfile_tool_call_with_camelcase_filepath_when_fixture_parsed() {
    // REQ-040 / REQ-063: ReadFile record, args_json carries `filePath` verbatim
    let sut = finish(&fixture_lines(), 0, Some(&fixture_export()));
    assert_eq!(sut.result.tool_calls.len(), 1);
    let rec = &sut.result.tool_calls[0];
    assert!(matches!(rec.kind, ToolKind::ReadFile));
    let args: Value =
        serde_json::from_str(rec.args_json.as_deref().unwrap()).expect("args_json is JSON");
    assert_eq!(args["filePath"], json!("/repo/src/lib.rs"));
}

#[test]
fn test_should_report_exact_usage_with_the_four_export_counts_when_fixture_parsed() {
    // REQ-042: usage from export.json: 1834 / 212 / 640 / 2046
    let sut = finish(&fixture_lines(), 0, Some(&fixture_export()));
    match sut.usage {
        UsageReport::Exact(u) => {
            assert_eq!(u.input, Some(1834));
            assert_eq!(u.output, Some(212));
            assert_eq!(u.cached, Some(640));
            assert_eq!(u.total, Some(2046));
        }
        UsageReport::NotReported => panic!("usage must be Exact"),
    }
}

#[test]
fn test_should_report_cost_and_cli_version_from_export_when_fixture_parsed() {
    // REQ-042 / REQ-040: cost_usd 0.0041 and vibe_version 2.26.1 come from export.json
    let sut = finish(&fixture_lines(), 0, Some(&fixture_export()));
    assert_eq!(sut.result.self_reported_cost_usd, Some(0.0041));
    assert_eq!(sut.result.cli_version.as_deref(), Some("2.26.1"));
}

#[test]
fn test_should_be_success_when_exit_zero_and_export_finished() {
    // REQ-052: exit 0 + finished => Success
    let sut = finish(&fixture_lines(), 0, Some(&fixture_export()));
    assert!(
        matches!(sut.outcome, SeatOutcome::Success),
        "was {}",
        outcome_label(&sut.outcome)
    );
}

// ---------- second variant: defeats constant-returning parsers ----------

#[test]
fn test_should_yield_second_variant_values_not_fixture_constants() {
    // reality test: a second variant yields its own different values
    let lines = vec![
        message_line("a1", "user", "Check main.rs"),
        read_file_line("a2", "/other/path/main.rs"),
        message_line("a3", "assistant", "Different answer entirely."),
    ];
    let export = export_json("finished", "vibe-sess-ZZ99", 7, 3, 1, 10, 0.5);
    let sut = finish(&lines, 0, Some(&export));
    assert_eq!(
        sut.result.response_text.trim(),
        "Different answer entirely."
    );
    assert_eq!(sut.result.session_id.as_deref(), Some("vibe-sess-ZZ99"));
    assert_eq!(sut.result.tool_calls.len(), 1);
    let args: Value =
        serde_json::from_str(sut.result.tool_calls[0].args_json.as_deref().unwrap()).unwrap();
    assert_eq!(args["filePath"], json!("/other/path/main.rs"));
    match sut.usage {
        UsageReport::Exact(u) => {
            assert_eq!(
                (u.input, u.output, u.cached, u.total),
                (Some(7), Some(3), Some(1), Some(10))
            );
        }
        UsageReport::NotReported => panic!("usage must be Exact"),
    }
    assert_eq!(sut.result.self_reported_cost_usd, Some(0.5));
}

#[test]
fn test_should_track_each_distinct_session_id_from_export() {
    // property: session_id always equals the export's session_id, whatever the stream says
    for (i, sid) in ["s-1", "vibe-abc-123", "0", "uuid-0000-1111-2222"]
        .iter()
        .enumerate()
    {
        let mut v = envelope("e", &format!("stream-{i}"), "message");
        v["role"] = json!("assistant");
        v["content"] = json!([{"type": "text", "text": "ok"}]);
        let export = export_json("finished", sid, 1, 1, 0, 2, 0.0);
        let sut = finish(&[v.to_string()], 0, Some(&export));
        assert_eq!(sut.result.session_id.as_deref(), Some(*sid));
    }
}

// ---------- ordering property ----------

#[test]
fn test_should_keep_assistant_messages_in_stream_order() {
    // property: N assistant messages appear in order in response_text
    for n in 1..=6usize {
        let texts: Vec<String> = (0..n).map(|i| format!("segment-{i}-zq")).collect();
        let lines: Vec<String> = texts
            .iter()
            .enumerate()
            .map(|(i, t)| message_line(&format!("m{i}"), "assistant", t))
            .collect();
        let sut = finish(&lines, 0, Some(&standard_export("finished")));
        let mut last = 0usize;
        for t in &texts {
            let pos = sut
                .result
                .response_text
                .find(t.as_str())
                .unwrap_or_else(|| panic!("missing {t}"));
            assert!(pos >= last, "out of order for n={n}");
            last = pos;
        }
    }
}

#[test]
fn test_should_equal_exact_text_for_single_assistant_message() {
    // REQ-040: one assistant message yields exactly its text
    let sut = finish(
        &[message_line("m1", "assistant", "Exactly this.")],
        0,
        Some(&standard_export("finished")),
    );
    assert_eq!(sut.result.response_text.trim(), "Exactly this.");
}

#[test]
fn test_should_exclude_reasoning_from_answer() {
    // spec: reasoning entries are never appended to the answer
    let lines = vec![
        reasoning_line("r1", "SECRET-REASONING-TEXT"),
        message_line("m1", "assistant", "visible answer"),
        reasoning_line("r2", "MORE-REASONING"),
    ];
    let sut = finish(&lines, 0, Some(&standard_export("finished")));
    assert!(!sut.result.response_text.contains("SECRET-REASONING-TEXT"));
    assert!(!sut.result.response_text.contains("MORE-REASONING"));
    assert!(sut.result.response_text.contains("visible answer"));
}

#[test]
fn test_should_exclude_user_messages_from_answer() {
    // spec: only role assistant carries the answer
    let lines = vec![
        message_line("m0", "user", "USER-PROMPT-ECHO"),
        message_line("m1", "assistant", "answer"),
    ];
    let sut = finish(&lines, 0, Some(&standard_export("finished")));
    assert!(!sut.result.response_text.contains("USER-PROMPT-ECHO"));
}

#[test]
fn test_should_preserve_multiple_text_blocks_in_one_assistant_message() {
    // spec: content is a list of blocks; text blocks all count
    let mut v = envelope("m1", "s", "message");
    v["role"] = json!("assistant");
    v["content"] =
        json!([{"type": "text", "text": "part-one"}, {"type": "text", "text": "part-two"}]);
    let sut = finish(&[v.to_string()], 0, Some(&standard_export("finished")));
    let t = &sut.result.response_text;
    assert!(
        t.contains("part-one") && t.contains("part-two"),
        "text was {t:?}"
    );
    assert!(t.find("part-one").unwrap() < t.find("part-two").unwrap());
}

#[test]
fn test_should_handle_unicode_assistant_text() {
    // edge: unicode survives parsing exactly
    let want = "caf\u{e9} \u{1F600} \u{05E9}\u{05DC}\u{05D5}\u{05DD}";
    let sut = finish(
        &[message_line("m1", "assistant", want)],
        0,
        Some(&standard_export("finished")),
    );
    assert_eq!(sut.result.response_text.trim(), want);
}

// ---------- tool calls ----------

#[test]
fn test_should_keep_each_read_file_effect_as_its_own_record_in_order() {
    // REQ-063: each read_file effect becomes a record with its own filePath
    let lines = vec![
        read_file_line("e1", "/a/one.rs"),
        read_file_line("e2", "/a/two.rs"),
        read_file_line("e3", "/a/three.rs"),
    ];
    let sut = finish(&lines, 0, Some(&standard_export("finished")));
    let paths: Vec<String> = sut
        .result
        .tool_calls
        .iter()
        .map(|r| {
            serde_json::from_str::<Value>(r.args_json.as_deref().unwrap()).unwrap()["filePath"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(paths, vec!["/a/one.rs", "/a/two.rs", "/a/three.rs"]);
}

#[test]
fn test_should_not_rename_filepath_key_to_snake_case() {
    // spec: filePath kept verbatim so the sight gate finds it (key casing mutation)
    let sut = finish(
        &[read_file_line("e1", "/x/y.rs")],
        0,
        Some(&standard_export("finished")),
    );
    assert!(
        sut.result.tool_calls[0]
            .args_json
            .as_deref()
            .unwrap()
            .contains("\"filePath\"")
    );
    assert!(
        !sut.result.tool_calls[0]
            .args_json
            .as_deref()
            .unwrap()
            .contains("file_path")
    );
}

#[test]
fn test_should_not_record_tool_calls_for_message_or_reasoning_entries() {
    // spec: only effect entries are tool uses
    let lines = vec![
        message_line("m1", "assistant", "a"),
        reasoning_line("r1", "b"),
        message_line("m2", "user", "c"),
    ];
    let sut = finish(&lines, 0, Some(&standard_export("finished")));
    assert!(sut.result.tool_calls.is_empty());
}

// ---------- parse_line robustness (REQ-044 mutations) ----------

#[test]
fn test_should_skip_malformed_json_line_and_parse_the_rest() {
    // REQ-044: malformed JSON line skipped
    let lines = vec![
        message_line("m1", "assistant", "before"),
        "{this is not json".to_string(),
        message_line("m2", "assistant", "after"),
    ];
    let sut = finish(&lines, 0, Some(&standard_export("finished")));
    assert!(sut.result.response_text.contains("before"));
    assert!(sut.result.response_text.contains("after"));
}

#[test]
fn test_should_skip_partial_trailing_line() {
    // REQ-044: truncated final line skipped, earlier content intact
    let full = message_line("m2", "assistant", "cut-off-text");
    let partial = full[..full.len() / 2].to_string();
    let lines = vec![message_line("m1", "assistant", "kept-text"), partial];
    let sut = finish(&lines, 0, Some(&standard_export("finished")));
    assert!(sut.result.response_text.contains("kept-text"));
    assert!(!sut.result.response_text.contains("cut-off-text"));
}

#[test]
fn test_should_ignore_unknown_entry_type() {
    // REQ-044: unknown entry type ignored; known entries around it still parse
    let mut unknown = envelope("u1", "s", "hologram");
    unknown["role"] = json!("assistant");
    unknown["content"] = json!([{"type": "text", "text": "UNKNOWN-TYPE-TEXT"}]);
    let lines = vec![
        message_line("m1", "assistant", "one"),
        unknown.to_string(),
        message_line("m2", "assistant", "two"),
    ];
    let sut = finish(&lines, 0, Some(&standard_export("finished")));
    assert!(!sut.result.response_text.contains("UNKNOWN-TYPE-TEXT"));
    assert!(sut.result.response_text.contains("one") && sut.result.response_text.contains("two"));
    assert!(sut.result.tool_calls.is_empty());
}

#[test]
fn test_should_not_panic_on_odd_lines() {
    // REQ-044: junk lines never panic and never produce an answer
    let mut sut = MistralStreamParser::new();
    for l in [
        "",
        "   ",
        "\t",
        "null",
        "123",
        "[]",
        "\"str\"",
        "{}",
        "{\"type\":",
        "{\"type\":null}",
        "{\"type\":\"message\"}",
        "\u{0}",
    ] {
        let _ = sut.parse_line(l);
    }
    let parsed = sut
        .finish(Some(0), Some(&standard_export("finished")))
        .expect("ok");
    assert_eq!(parsed.result.response_text.trim(), "");
    assert!(parsed.result.tool_calls.is_empty());
}

#[test]
fn test_should_ignore_message_with_missing_content() {
    // edge: assistant message without content adds nothing
    let mut v = envelope("m1", "s", "message");
    v["role"] = json!("assistant");
    let lines = vec![v.to_string(), message_line("m2", "assistant", "real")];
    let sut = finish(&lines, 0, Some(&standard_export("finished")));
    assert_eq!(sut.result.response_text.trim(), "real");
}

#[test]
fn test_should_ignore_effect_without_detail() {
    // edge: effect with no detail does not become a ReadFile record with a path
    let mut v = envelope("e1", "s", "effect");
    v["title"] = json!("read_file");
    let sut = finish(&[v.to_string()], 0, Some(&standard_export("finished")));
    assert!(
        sut.result.tool_calls.iter().all(|r| !r
            .args_json
            .as_deref()
            .unwrap_or("")
            .contains("filePath"))
    );
}

#[test]
fn test_should_return_working_state_event_for_effect_line() {
    // spec: parse_line returns an event when the line carries one (tool use)
    let mut sut = MistralStreamParser::new();
    let ev = sut.parse_line(&read_file_line("e1", "/a/b.rs"));
    let ev = ev.expect("a tool-use line must yield an event");
    assert_eq!(ev.tool_name.as_deref(), Some("read_file"));
}

#[test]
fn test_should_return_no_event_for_unparseable_line() {
    // spec: no event when the line carries none
    let mut sut = MistralStreamParser::new();
    assert!(sut.parse_line("{not json").is_none());
    assert!(sut.parse_line("").is_none());
}

// ---------- export edge cases ----------

#[test]
fn test_should_not_fabricate_session_id_when_export_lacks_one() {
    // REQ-044 / REQ-040: absence of a session id is None, never the stream's sessionId
    let export = json!({
        "schema_version": 1, "outcome": "finished",
        "usage": {"input_tokens": 1, "output_tokens": 1, "cached_input_tokens": 0, "total_tokens": 2},
        "cost_usd": 0.0, "vibe_version": "2.26.1"
    })
    .to_string();
    let sut = finish(&[message_line("m1", "assistant", "hi")], 0, Some(&export));
    assert_eq!(sut.result.session_id, None);
}

#[test]
fn test_should_report_not_reported_when_export_has_no_usage() {
    // REQ-041: missing usage is NotReported, never zero
    let export = json!({"schema_version": 1, "outcome": "finished", "session_id": "s-1", "vibe_version": "2.26.1"})
        .to_string();
    let sut = finish(&[message_line("m1", "assistant", "hi")], 0, Some(&export));
    assert!(matches!(sut.usage, UsageReport::NotReported));
}

#[test]
fn test_should_keep_reported_zero_usage_as_exact_zero() {
    // REQ-041: a reported zero stays Exact(0)
    let export = export_json("finished", "s-1", 0, 0, 0, 0, 0.0);
    let sut = finish(&[message_line("m1", "assistant", "hi")], 0, Some(&export));
    match sut.usage {
        UsageReport::Exact(u) => assert_eq!(
            (u.input, u.output, u.cached, u.total),
            (Some(0), Some(0), Some(0), Some(0))
        ),
        UsageReport::NotReported => panic!("reported zero must be Exact(0)"),
    }
}

#[test]
fn test_should_have_no_cost_when_export_lacks_cost() {
    // REQ-042: cost absent => None, not Some(0.0)
    let export = json!({"schema_version": 1, "outcome": "finished", "session_id": "s-1", "vibe_version": "2.26.1"})
        .to_string();
    let sut = finish(&[message_line("m1", "assistant", "hi")], 0, Some(&export));
    assert_eq!(sut.result.self_reported_cost_usd, None);
}

#[test]
fn test_should_never_succeed_on_unparseable_export() {
    // REQ-042: a garbage export is not a success
    let r = parse_all(&[message_line("m1", "assistant", "hi")]).finish(Some(0), Some("{not json"));
    assert!(
        is_failure(&r).is_some(),
        "garbage export must be a failure, not Success"
    );
}

#[test]
fn test_should_never_succeed_on_empty_export_text() {
    // REQ-042: an empty export file is not a success
    let r = parse_all(&[message_line("m1", "assistant", "hi")]).finish(Some(0), Some(""));
    assert!(is_failure(&r).is_some());
}

// ---------- missing export ----------

#[test]
fn test_should_fail_missing_export_mistral_when_no_export_and_exit_zero() {
    // REQ-042: stream with no export.json is failure missing_export:mistral, never an empty success
    let r = parse_all(&fixture_lines()).finish(Some(0), None);
    let msg = is_failure(&r).expect("must be a failure");
    assert!(
        msg.starts_with("missing_export:mistral"),
        "code must come first, was {msg:?}"
    );
}

#[test]
fn test_should_fail_missing_export_mistral_when_no_export_and_exit_three() {
    // REQ-042 / REQ-052: partial exit does not rescue a missing export
    let r = parse_all(&fixture_lines()).finish(Some(3), None);
    let msg = is_failure(&r).expect("must be a failure");
    assert!(msg.starts_with("missing_export:mistral"), "was {msg:?}");
}

#[test]
fn test_should_fail_missing_export_mistral_for_empty_stream() {
    // REQ-042: empty stream plus no export is a failure
    let r = MistralStreamParser::new().finish(Some(0), None);
    let msg = is_failure(&r).expect("must be a failure");
    assert!(msg.starts_with("missing_export:mistral"), "was {msg:?}");
}

#[test]
fn test_should_fail_missing_export_mistral_for_each_exit_code() {
    // REQ-042: exit code never turns a missing export into success
    for code in [0, 1, 2, 3, 4] {
        let r = parse_all(&fixture_lines()).finish(Some(code), None);
        let msg = is_failure(&r).unwrap_or_else(|| panic!("exit {code} with no export must fail"));
        assert!(
            msg.starts_with("missing_export:mistral"),
            "exit {code}: {msg:?}"
        );
    }
}

// ---------- exit code / outcome table (REQ-052) ----------

#[test]
fn test_should_mark_partial_for_each_partial_outcome_with_exit_three() {
    // REQ-052: exit 3 + {turn_limit, token_limit, price_limit, deadline, terminated, length, refusal} => Partial
    for o in PARTIAL_OUTCOMES {
        let sut = finish(
            &[message_line("m1", "assistant", "partial answer")],
            3,
            Some(&standard_export(o)),
        );
        match sut.outcome {
            SeatOutcome::Partial(reason) => assert!(!reason.is_empty(), "{o}: reason empty"),
            other => panic!("{o}: expected Partial, was {}", outcome_label(&other)),
        }
        assert!(
            sut.result.response_text.contains("partial answer"),
            "{o}: usable output must be kept"
        );
    }
}

#[test]
fn test_should_name_the_outcome_in_the_partial_reason() {
    // REQ-052: Partial carries "the reason"; the export outcome is the natural reason
    for o in PARTIAL_OUTCOMES {
        let sut = finish(
            &[message_line("m1", "assistant", "x")],
            3,
            Some(&standard_export(o)),
        );
        match sut.outcome {
            SeatOutcome::Partial(reason) => {
                assert!(reason.contains(o), "{o}: reason was {reason:?}")
            }
            other => panic!("{o}: expected Partial, was {}", outcome_label(&other)),
        }
    }
}

#[test]
fn test_should_fail_when_exit_zero_but_export_says_infrastructure_failure() {
    // REQ-052: exit 0 + export infrastructure_failure => Failure (export wins)
    let sut = finish(
        &fixture_lines(),
        0,
        Some(&standard_export("infrastructure_failure")),
    );
    match sut.outcome {
        SeatOutcome::Failure(m) => assert!(!m.is_empty()),
        other => panic!("expected Failure, was {}", outcome_label(&other)),
    }
}

#[test]
fn test_should_fail_for_exit_two_and_infrastructure_failure() {
    // REQ-052: exit 2 => infrastructure failure
    let sut = finish(
        &fixture_lines(),
        2,
        Some(&standard_export("infrastructure_failure")),
    );
    assert!(
        matches!(sut.outcome, SeatOutcome::Failure(_)),
        "was {}",
        outcome_label(&sut.outcome)
    );
}

#[test]
fn test_should_fail_for_exit_one_usage_error_and_config_error() {
    // REQ-052: exit 1 (usage_error, config_error) => Failure
    for o in ["usage_error", "config_error"] {
        let sut = finish(&[], 1, Some(&standard_export(o)));
        assert!(
            matches!(sut.outcome, SeatOutcome::Failure(_)),
            "{o}: was {}",
            outcome_label(&sut.outcome)
        );
    }
}

#[test]
fn test_should_let_export_outcome_win_when_exit_zero_and_partial_outcome() {
    // REQ-052: export outcome wins over exit code on disagreement (exit 0 + turn_limit is not Success)
    let sut = finish(
        &[message_line("m1", "assistant", "x")],
        0,
        Some(&standard_export("turn_limit")),
    );
    assert!(
        matches!(sut.outcome, SeatOutcome::Partial(_)),
        "was {}",
        outcome_label(&sut.outcome)
    );
}

#[test]
fn test_should_not_report_success_for_exit_two_with_finished_export() {
    // REQ-052: exit 2 must not become Success even if the export says finished
    let r = parse_all(&[message_line("m1", "assistant", "x")])
        .finish(Some(2), Some(&standard_export("finished")));
    match r {
        Err(_) => {}
        Ok(p) => assert!(
            !matches!(p.outcome, SeatOutcome::Success),
            "exit 2 must not be Success"
        ),
    }
}

#[test]
fn test_should_use_exit_table_when_outcome_is_unrecognised() {
    // REQ-052: with no recognised outcome the exit-code table decides
    let table: [(i32, &str); 4] = [
        (0, "success"),
        (1, "failure"),
        (2, "failure"),
        (3, "partial"),
    ];
    for (code, want) in table {
        let export = export_json("some_future_outcome", "s-1", 1, 1, 0, 2, 0.0);
        let sut = finish(&[message_line("m1", "assistant", "x")], code, Some(&export));
        let got = match sut.outcome {
            SeatOutcome::Success => "success",
            SeatOutcome::Partial(_) => "partial",
            SeatOutcome::Failure(_) => "failure",
        };
        assert_eq!(got, want, "exit {code} with unrecognised outcome");
    }
}

#[test]
fn test_should_use_exit_table_when_outcome_field_is_missing() {
    // REQ-052: no outcome at all => exit-code table
    let export =
        json!({"schema_version": 1, "session_id": "s-1", "vibe_version": "2.26.1"}).to_string();
    let ok = finish(&[message_line("m1", "assistant", "x")], 0, Some(&export));
    assert!(
        matches!(ok.outcome, SeatOutcome::Success),
        "exit 0: {}",
        outcome_label(&ok.outcome)
    );
    let partial = finish(&[message_line("m1", "assistant", "x")], 3, Some(&export));
    assert!(
        matches!(partial.outcome, SeatOutcome::Partial(_)),
        "exit 3: {}",
        outcome_label(&partial.outcome)
    );
    let fail = finish(&[message_line("m1", "assistant", "x")], 2, Some(&export));
    assert!(
        matches!(fail.outcome, SeatOutcome::Failure(_)),
        "exit 2: {}",
        outcome_label(&fail.outcome)
    );
}

#[test]
fn test_should_keep_usage_and_session_even_when_outcome_is_failure() {
    // REQ-042: export is read whichever way the run ends
    let sut = finish(
        &[],
        2,
        Some(&export_json(
            "infrastructure_failure",
            "sess-fail-1",
            5,
            0,
            0,
            5,
            0.01,
        )),
    );
    assert_eq!(sut.result.session_id.as_deref(), Some("sess-fail-1"));
    assert!(matches!(sut.usage, UsageReport::Exact(_)));
}

#[test]
fn test_should_keep_usage_even_when_outcome_is_partial() {
    // REQ-042: partial runs still report their usage
    let export = export_json("token_limit", "sess-p", 100, 50, 10, 150, 0.2);
    let sut = finish(&[message_line("m1", "assistant", "x")], 3, Some(&export));
    match sut.usage {
        UsageReport::Exact(u) => assert_eq!(
            (u.input, u.output, u.cached, u.total),
            (Some(100), Some(50), Some(10), Some(150))
        ),
        UsageReport::NotReported => panic!("usage must be Exact"),
    }
}

// ---------- determinism ----------

#[test]
fn test_should_give_identical_results_for_identical_input() {
    // property: pure function of (lines, exit, export)
    let a = finish(&fixture_lines(), 0, Some(&fixture_export()));
    let b = finish(&fixture_lines(), 0, Some(&fixture_export()));
    assert_eq!(a.result.response_text, b.result.response_text);
    assert_eq!(a.result.session_id, b.result.session_id);
    assert_eq!(a.result.tool_calls.len(), b.result.tool_calls.len());
}

#[test]
fn test_should_not_leak_state_between_parser_instances() {
    // multi-call: a fresh parser starts empty
    let first = finish(
        &[message_line("m1", "assistant", "FIRST-RUN")],
        0,
        Some(&standard_export("finished")),
    );
    assert!(first.result.response_text.contains("FIRST-RUN"));
    let second = finish(
        &[message_line("m2", "assistant", "SECOND-RUN")],
        0,
        Some(&standard_export("finished")),
    );
    assert!(!second.result.response_text.contains("FIRST-RUN"));
}

// ---------- round 2: survivor feedback (tool kinds, exit table, status, usage guard) ----------

fn effect_line(tool: &str, kind: &str, status: &str) -> String {
    let mut v = envelope("ex1", "sess-stream-ignored", "effect");
    v["title"] = json!(tool);
    v["detail"] = json!({"toolName": tool, "kind": kind, "input": {"filePath": "/p/q.rs"}});
    v["state"] = json!({"status": status, "output": {}, "display": {"success": true}});
    v.to_string()
}

fn kind_label(k: &ToolKind) -> &'static str {
    match k {
        ToolKind::ReadFile => "ReadFile",
        ToolKind::WriteFile => "WriteFile",
        ToolKind::EditFile => "EditFile",
        ToolKind::Bash => "Bash",
        ToolKind::Grep => "Grep",
        ToolKind::Glob => "Glob",
        ToolKind::RequestUserInput => "RequestUserInput",
        ToolKind::Unknown => "Unknown",
    }
}

fn kind_of(tool: &str, kind: &str) -> &'static str {
    let sut = finish(
        &[effect_line(tool, kind, "completed")],
        0,
        Some(&standard_export("finished")),
    );
    assert_eq!(
        sut.result.tool_calls.len(),
        1,
        "{tool}/{kind}: one record expected"
    );
    kind_label(&sut.result.tool_calls[0].kind)
}

#[test]
fn test_should_classify_tool_kind_by_vendor_tool_name() {
    // research/02 built-in tool modules: name decides the ToolKind
    let table = [
        ("read_file", "ReadFile"),
        ("grep", "Grep"),
        ("write_file", "WriteFile"),
        ("edit", "EditFile"),
        ("bash", "Bash"),
        ("git_bash", "Bash"),
        ("experimental_bash", "Bash"),
        ("windows_shell", "Bash"),
        ("ask_user_question", "RequestUserInput"),
    ];
    for (tool, want) in table {
        // neutral detail.kind so the name alone decides
        assert_eq!(kind_of(tool, "other"), want, "tool {tool}");
    }
}

#[test]
fn test_should_classify_tool_kind_from_detail_kind_when_name_is_unknown() {
    // research/02: detail.kind "file_read" / "shell" are the fallbacks
    assert_eq!(kind_of("mystery_tool", "file_read"), "ReadFile");
    assert_eq!(kind_of("mystery_tool", "shell"), "Bash");
}

#[test]
fn test_should_classify_unknown_tool_with_unknown_kind_as_unknown() {
    // edge: nothing recognised => Unknown
    assert_eq!(kind_of("mystery_tool", "mystery_kind"), "Unknown");
}

#[test]
fn test_should_not_classify_non_read_tools_as_readfile() {
    // property: write/edit/shell/grep records are never ReadFile (they must not earn sight credit)
    for tool in ["write_file", "edit", "bash", "grep", "ask_user_question"] {
        assert_ne!(kind_of(tool, "other"), "ReadFile", "tool {tool}");
    }
}

fn success_of(status: &str) -> Option<bool> {
    let sut = finish(
        &[effect_line("read_file", "file_read", status)],
        0,
        Some(&standard_export("finished")),
    );
    sut.result.tool_calls[0].success
}

#[test]
fn test_should_map_effect_status_to_tool_success() {
    // spec: state.status completed => success true; failed/error/rejected => success false
    assert_eq!(success_of("completed"), Some(true));
    for s in ["failed", "error", "rejected"] {
        assert_eq!(success_of(s), Some(false), "status {s}");
    }
}

fn outcome_kind(o: &SeatOutcome) -> &'static str {
    match o {
        SeatOutcome::Success => "success",
        SeatOutcome::Partial(_) => "partial",
        SeatOutcome::Failure(_) => "failure",
    }
}

#[test]
fn test_should_follow_exit_table_row_by_row_when_export_names_no_outcome() {
    // BACKEND_STRUCTURE section 6 / research/02: 0 finished, 1 usage/config, 2 infra, 3 partial, 4 aborted
    let export =
        json!({"schema_version": 1, "session_id": "s-1", "vibe_version": "2.26.1"}).to_string();
    let table = [
        (0, "success"),
        (1, "failure"),
        (2, "failure"),
        (3, "partial"),
        (4, "failure"),
    ];
    for (code, want) in table {
        let sut = finish(&[message_line("m1", "assistant", "x")], code, Some(&export));
        assert_eq!(outcome_kind(&sut.outcome), want, "exit {code}");
    }
}

#[test]
fn test_should_treat_empty_outcome_string_as_no_outcome() {
    // finished/empty rule: "" is not a recognised outcome, the exit table decides
    let table = [
        (0, "success"),
        (1, "failure"),
        (2, "failure"),
        (3, "partial"),
    ];
    for (code, want) in table {
        let export = export_json("", "s-1", 1, 1, 0, 2, 0.0);
        let sut = finish(&[message_line("m1", "assistant", "x")], code, Some(&export));
        assert_eq!(
            outcome_kind(&sut.outcome),
            want,
            "exit {code} with empty outcome"
        );
    }
}

#[test]
fn test_should_treat_null_outcome_as_no_outcome() {
    // finished/empty rule: null outcome falls back to the exit table
    let export = json!({"schema_version": 1, "outcome": null, "session_id": "s-1"}).to_string();
    let a = finish(&[message_line("m1", "assistant", "x")], 0, Some(&export));
    assert_eq!(outcome_kind(&a.outcome), "success");
    let b = finish(&[message_line("m1", "assistant", "x")], 2, Some(&export));
    assert_eq!(outcome_kind(&b.outcome), "failure");
}

#[test]
fn test_should_succeed_for_finished_outcome_with_exit_zero_and_keep_answer() {
    // finished arm: Success keeps the answer
    let sut = finish(
        &[message_line("m1", "assistant", "done")],
        0,
        Some(&standard_export("finished")),
    );
    assert_eq!(outcome_kind(&sut.outcome), "success");
    assert!(sut.result.response_text.contains("done"));
}

#[test]
fn test_should_give_non_empty_failure_message_for_each_failing_row() {
    // REQ-052: a Failure always says why
    let export = json!({"schema_version": 1, "session_id": "s-1"}).to_string();
    for code in [1, 2, 4] {
        let sut = finish(&[], code, Some(&export));
        match sut.outcome {
            SeatOutcome::Failure(m) => {
                assert!(!m.trim().is_empty(), "exit {code}: empty failure message")
            }
            other => panic!(
                "exit {code}: expected Failure, was {}",
                outcome_label(&other)
            ),
        }
    }
}

#[test]
fn test_should_give_non_empty_partial_reason_when_exit_three_without_outcome() {
    // REQ-052: Partial always carries a reason, even when the export names none
    let export = json!({"schema_version": 1, "session_id": "s-1"}).to_string();
    let sut = finish(&[message_line("m1", "assistant", "x")], 3, Some(&export));
    match sut.outcome {
        SeatOutcome::Partial(r) => assert!(!r.trim().is_empty()),
        other => panic!("expected Partial, was {}", outcome_label(&other)),
    }
}

fn usage_export(usage: Value) -> String {
    json!({"schema_version": 1, "outcome": "finished", "session_id": "s-1", "vibe_version": "2.26.1", "usage": usage})
        .to_string()
}

#[test]
fn test_should_report_exact_usage_when_only_input_count_is_present() {
    // REQ-041: any one count present => Exact
    let sut = finish(&[], 0, Some(&usage_export(json!({"input_tokens": 9}))));
    match sut.usage {
        UsageReport::Exact(u) => assert_eq!(u.input, Some(9)),
        UsageReport::NotReported => panic!("one count present must be Exact"),
    }
}

#[test]
fn test_should_report_exact_usage_when_only_output_count_is_present() {
    // REQ-041: any one count present => Exact
    let sut = finish(&[], 0, Some(&usage_export(json!({"output_tokens": 4}))));
    match sut.usage {
        UsageReport::Exact(u) => assert_eq!(u.output, Some(4)),
        UsageReport::NotReported => panic!("one count present must be Exact"),
    }
}

#[test]
fn test_should_report_exact_usage_when_only_total_count_is_present() {
    // REQ-041: any one count present => Exact
    let sut = finish(&[], 0, Some(&usage_export(json!({"total_tokens": 13}))));
    match sut.usage {
        UsageReport::Exact(u) => assert_eq!(u.total, Some(13)),
        UsageReport::NotReported => panic!("one count present must be Exact"),
    }
}

#[test]
fn test_should_report_not_reported_when_usage_object_has_no_counts() {
    // REQ-041: empty usage object => NotReported, never zeros
    let sut = finish(&[], 0, Some(&usage_export(json!({}))));
    assert!(matches!(sut.usage, UsageReport::NotReported));
}

#[test]
fn test_should_report_not_reported_when_usage_is_null() {
    // REQ-041: null usage => NotReported
    let sut = finish(&[], 0, Some(&usage_export(Value::Null)));
    assert!(matches!(sut.usage, UsageReport::NotReported));
}
