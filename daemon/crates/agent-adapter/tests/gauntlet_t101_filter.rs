// GAUNTLET filter suite for T-101, written blind from the spec (no implementation read).
//
// SPEC_AMBIGUITY: how multiple assistant text chunks are joined into response_text (separator) is unspecified; multi-chunk
// tests assert containment and order only. TokenUsage/self_reported_cost_usd for Kimi are asserted as None.
// SPEC_AMBIGUITY: the " last_retry_status=<n>" suffix is specified for "both failure codes"; read here as nonzero_exit and
// no_exit_code. Whether empty_or_unrecognized_stream also carries it is not asserted.
// SPEC_AMBIGUITY: whether whitespace-only assistant content counts as text is unspecified; not asserted.

use agent_adapter::kimi::{KimiStreamParser, PARSER_MODE};
use agent_adapter::{SeatOutcome, SeatParsed, ToolKind, UsageReport, WorkingState};
use std::path::PathBuf;

fn fixture_lines() -> Vec<String> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/kimi-2.2.0-provisional-20261018.jsonl");
    std::fs::read_to_string(p).unwrap().lines().map(String::from).collect()
}

fn run(lines: &[String], exit: Option<i32>) -> Result<SeatParsed, String> {
    let mut p = KimiStreamParser::new();
    for l in lines {
        let _ = p.parse_line(l);
    }
    p.finish(exit)
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn args_of(p: &SeatParsed, i: usize) -> String {
    p.result.tool_calls[i].args_json.clone().expect("args_json present")
}

/// A stream: read of `path`, tool result, final `answer`, optional resume hint with `sid`.
fn stream(path: &str, answer: &str, sid: Option<&str>) -> Vec<String> {
    let mut v = vec![
        r#"{"role":"meta","type":"system.version","version":"2.2.0"}"#.to_string(),
        format!(
            r#"{{"role":"assistant","content":"","tool_calls":[{{"type":"function","id":"call_1","function":{{"name":"Read","arguments":"{{\"path\":\"{path}\"}}"}}}}]}}"#
        ),
        r#"{"role":"tool","tool_call_id":"call_1","content":"file body"}"#.to_string(),
        format!(r#"{{"role":"assistant","content":"{answer}"}}"#),
    ];
    if let Some(id) = sid {
        v.push(format!(
            r#"{{"role":"meta","type":"session.resume_hint","session_id":"{id}","command":"kimi -r {id}","content":"To resume this session, run: kimi -r {id}"}}"#
        ));
    }
    v
}

fn failure_of(p: &SeatParsed) -> String {
    match &p.outcome {
        SeatOutcome::Failure(m) => m.clone(),
        _ => panic!("expected Failure"),
    }
}

#[test]
fn should_expose_parser_mode() {
    // PARSER_MODE constant
    assert_eq!(PARSER_MODE, "kimi-stream-json");
}

#[test]
fn should_parse_fixture_into_success_with_exact_values() {
    // reality_test: text, one Read record with path, session id, NotReported
    let parsed = run(&fixture_lines(), Some(0)).unwrap();
    assert!(matches!(parsed.outcome, SeatOutcome::Success));
    assert!(matches!(parsed.usage, UsageReport::NotReported));
    assert!(parsed.served_model.is_none());
    let r = &parsed.result;
    assert_eq!(r.response_text, "main.rs defines an empty main function.");
    assert_eq!(r.session_id.as_deref(), Some("sess-kimi-fixture-0001"));
    assert_eq!(r.cli_version.as_deref(), Some("2.2.0"));
    assert_eq!(r.parser_mode, "kimi-stream-json");
    assert!(r.token_usage.is_none());
    assert!(r.self_reported_cost_usd.is_none());
    assert_eq!(r.tool_calls.len(), 1);
    let tc = &r.tool_calls[0];
    assert_eq!(tc.tool, "Read");
    assert_eq!(tc.kind, ToolKind::ReadFile);
    assert_eq!(tc.id.as_deref(), Some("call_read_1"));
    assert!(args_of(&parsed, 0).contains("/repo/src/main.rs"));
}

#[test]
fn should_record_exactly_one_read_tool_call_for_fixture() {
    // reality_test: one ToolCallRecord for the single tool call, path under a READ_OPERAND_KEYS key
    let parsed = run(&fixture_lines(), Some(0)).unwrap();
    assert_eq!(parsed.result.tool_calls.len(), 1);
    let v: serde_json::Value = serde_json::from_str(&args_of(&parsed, 0)).unwrap();
    assert_eq!(v["path"], "/repo/src/main.rs");
}

#[test]
fn should_yield_different_values_for_second_variant() {
    // reality_test: variant with other path, answer, session id defeats constant-returning parsers
    let lines = stream("/other/lib/zeta.py", "zeta is a python module.", Some("sess-variant-0002"));
    let parsed = run(&lines, Some(0)).unwrap();
    let r = &parsed.result;
    assert_eq!(r.response_text, "zeta is a python module.");
    assert_eq!(r.session_id.as_deref(), Some("sess-variant-0002"));
    assert_eq!(r.tool_calls.len(), 1);
    assert_eq!(r.tool_calls[0].tool, "Read");
    assert_eq!(r.tool_calls[0].kind, ToolKind::ReadFile);
    assert!(args_of(&parsed, 0).contains("/other/lib/zeta.py"));
    assert!(!args_of(&parsed, 0).contains("/repo/src/main.rs"));
}

#[test]
fn should_hold_fixture_invariants_for_many_generated_streams() {
    // property: any (path, answer, id) lands in the result and the previous stream's id does not
    for i in 0..25 {
        let path = format!("/gen/dir{i}/file_{}.rs", i * 7 + 3);
        let answer = format!("answer number {} zz{}", i, i * 13);
        let sid = format!("sess-gen-{i:04}");
        let parsed = run(&stream(&path, &answer, Some(&sid)), Some(0)).unwrap();
        assert_eq!(parsed.result.response_text, answer);
        assert_eq!(parsed.result.session_id.as_deref(), Some(sid.as_str()));
        assert_eq!(parsed.result.tool_calls.len(), 1);
        assert!(args_of(&parsed, 0).contains(&path));
    }
}

#[test]
fn should_report_usage_not_reported_never_exact() {
    // REQ-041: Kimi stdout usage is NotReported, never zero
    for exit in [Some(0), Some(1), None] {
        if let Ok(p) = run(&fixture_lines(), exit) {
            assert!(matches!(p.usage, UsageReport::NotReported));
        }
    }
}

#[test]
fn should_not_invent_session_id_without_resume_hint() {
    // spec: without session.resume_hint the id is None, never invented
    let without = run(&stream("/p/a.rs", "ans", None), Some(0)).unwrap();
    assert!(matches!(without.outcome, SeatOutcome::Success));
    assert_eq!(without.result.session_id, None);
    assert_eq!(without.result.response_text, "ans");
}

#[test]
fn should_take_session_id_only_from_resume_hint() {
    // session id comes from session.resume_hint; tool_call ids must not become session ids
    let parsed = run(&stream("/p/a.rs", "ans", Some("sess-xyz-9")), Some(0)).unwrap();
    assert_eq!(parsed.result.session_id.as_deref(), Some("sess-xyz-9"));
}

#[test]
fn should_skip_partial_trailing_line_and_keep_rest() {
    // REQ-044: partial trailing line skipped, rest parses
    let mut lines = fixture_lines();
    lines.push(r#"{"role":"assistant","content":"cut off mid"#.to_string());
    let parsed = run(&lines, Some(0)).unwrap();
    assert!(matches!(parsed.outcome, SeatOutcome::Success));
    assert_eq!(parsed.result.response_text, "main.rs defines an empty main function.");
    assert_eq!(parsed.result.session_id.as_deref(), Some("sess-kimi-fixture-0001"));
}

#[test]
fn should_skip_malformed_json_line_in_the_middle() {
    // REQ-044: malformed JSON line skipped, never aborts
    let mut lines = fixture_lines();
    lines.insert(2, "this is {not json".to_string());
    lines.insert(3, "".to_string());
    lines.insert(4, "}{".to_string());
    let parsed = run(&lines, Some(0)).unwrap();
    assert!(matches!(parsed.outcome, SeatOutcome::Success));
    assert_eq!(parsed.result.response_text, "main.rs defines an empty main function.");
    assert_eq!(parsed.result.tool_calls.len(), 1);
    assert_eq!(parsed.result.session_id.as_deref(), Some("sess-kimi-fixture-0001"));
}

#[test]
fn should_ignore_unknown_meta_type() {
    // REQ-044: unknown meta type ignored
    let mut lines = fixture_lines();
    lines.insert(1, r#"{"role":"meta","type":"future.unknown","payload":{"a":1}}"#.to_string());
    let parsed = run(&lines, Some(0)).unwrap();
    assert!(matches!(parsed.outcome, SeatOutcome::Success));
    assert_eq!(parsed.result.response_text, "main.rs defines an empty main function.");
    assert_eq!(parsed.result.session_id.as_deref(), Some("sess-kimi-fixture-0001"));
}

#[test]
fn should_ignore_unknown_role() {
    // spec: unknown roles are ignored
    let mut lines = fixture_lines();
    lines.insert(1, r#"{"role":"developer","content":"sneaky role text"}"#.to_string());
    let parsed = run(&lines, Some(0)).unwrap();
    assert_eq!(parsed.result.response_text, "main.rs defines an empty main function.");
}

#[test]
fn should_fail_empty_stream_with_exit_zero() {
    // reality_test: empty stream exit 0 -> empty_or_unrecognized_stream:kimi
    let parsed = run(&[], Some(0)).unwrap();
    assert_eq!(failure_of(&parsed), "empty_or_unrecognized_stream:kimi");
}

#[test]
fn should_fail_non_kimi_garbage_with_exit_zero() {
    // spec: zero recognized lines means empty or not kimi's
    let parsed = run(&s(&["hello world", "{\"type\":\"result\",\"is_error\":false}"]), Some(0)).unwrap();
    assert_eq!(failure_of(&parsed), "empty_or_unrecognized_stream:kimi");
}

#[test]
fn should_fail_exit_zero_with_no_assistant_text() {
    // finish doc: exit 0 with no text is Failure
    let lines = s(&[
        r#"{"role":"meta","type":"system.version","version":"2.2.0"}"#,
        r#"{"role":"assistant","content":""}"#,
    ]);
    let parsed = run(&lines, Some(0)).unwrap();
    assert_eq!(failure_of(&parsed), "empty_or_unrecognized_stream:kimi");
}

#[test]
fn should_map_nonzero_exit_to_failure_with_code() {
    // finish doc: nonzero_exit:kimi:<code>, even with usable text
    for code in [1, 2, 137] {
        let parsed = run(&fixture_lines(), Some(code)).unwrap();
        assert_eq!(failure_of(&parsed), format!("nonzero_exit:kimi:{code}"));
    }
}

#[test]
fn should_map_nonzero_exit_on_empty_stream_to_nonzero_failure() {
    // finish doc: a non-zero exit is nonzero_exit:kimi:<code>
    let parsed = run(&[], Some(1)).unwrap();
    assert_eq!(failure_of(&parsed), "nonzero_exit:kimi:1");
}

#[test]
fn should_map_missing_exit_code_to_no_exit_code_failure() {
    // finish doc: killed by signal -> no_exit_code:kimi
    let parsed = run(&fixture_lines(), None).unwrap();
    assert_eq!(failure_of(&parsed), "no_exit_code:kimi");
}

#[test]
fn should_append_last_retry_status_to_nonzero_exit() {
    // finish doc: " last_retry_status=<n>" appended when a retry was seen
    let mut lines = fixture_lines();
    lines.insert(1, r#"{"role":"meta","type":"turn.step.retrying","status_code":429,"attempt":1}"#.to_string());
    let parsed = run(&lines, Some(1)).unwrap();
    assert_eq!(failure_of(&parsed), "nonzero_exit:kimi:1 last_retry_status=429");
}

#[test]
fn should_append_last_retry_status_to_no_exit_code() {
    // finish doc: both failure codes get the suffix
    let mut lines = fixture_lines();
    lines.insert(1, r#"{"role":"meta","type":"turn.step.retrying","status_code":403}"#.to_string());
    let parsed = run(&lines, None).unwrap();
    assert_eq!(failure_of(&parsed), "no_exit_code:kimi last_retry_status=403");
}

#[test]
fn should_use_most_recent_retry_status() {
    // struct doc: last_retry_status is the MOST RECENT turn.step.retrying
    let mut lines = fixture_lines();
    lines.insert(1, r#"{"role":"meta","type":"turn.step.retrying","status_code":500}"#.to_string());
    lines.insert(2, r#"{"role":"meta","type":"turn.step.retrying","status_code":429}"#.to_string());
    let parsed = run(&lines, Some(2)).unwrap();
    assert_eq!(failure_of(&parsed), "nonzero_exit:kimi:2 last_retry_status=429");
}

#[test]
fn should_not_append_retry_suffix_when_none_seen() {
    // finish doc: suffix only when a retry was seen
    let parsed = run(&fixture_lines(), Some(3)).unwrap();
    assert_eq!(failure_of(&parsed), "nonzero_exit:kimi:3");
}

#[test]
fn should_succeed_when_retry_seen_but_turn_completed() {
    // a retried step that later completes with exit 0 is still Success
    let mut lines = fixture_lines();
    lines.insert(1, r#"{"role":"meta","type":"turn.step.retrying","status_code":429}"#.to_string());
    let parsed = run(&lines, Some(0)).unwrap();
    assert!(matches!(parsed.outcome, SeatOutcome::Success));
}

#[test]
fn should_keep_assistant_chunks_in_stream_order() {
    // struct doc: text_chunks in stream order
    let lines = s(&[
        r#"{"role":"assistant","content":"FIRSTCHUNK"}"#,
        r#"{"role":"assistant","content":"SECONDCHUNK"}"#,
    ]);
    let parsed = run(&lines, Some(0)).unwrap();
    let d = parsed.result.response_text.clone();
    let a = d.find("FIRSTCHUNK").expect("first chunk");
    let b = d.find("SECONDCHUNK").expect("second chunk");
    assert!(a < b);
}

#[test]
fn should_record_multiple_tool_calls_from_one_assistant_message() {
    // line shape: tool_calls is an array
    let lines = s(&[
        r#"{"role":"assistant","content":"","tool_calls":[{"id":"c1","function":{"name":"Read","arguments":"{\"path\":\"/m/one.rs\"}"}},{"id":"c2","function":{"name":"Read","arguments":"{\"path\":\"/m/two.rs\"}"}}]}"#,
        r#"{"role":"assistant","content":"done"}"#,
    ]);
    let parsed = run(&lines, Some(0)).unwrap();
    assert_eq!(parsed.result.tool_calls.len(), 2);
    assert!(args_of(&parsed, 0).contains("/m/one.rs"));
    assert!(args_of(&parsed, 1).contains("/m/two.rs"));
    assert_eq!(parsed.result.tool_calls[0].id.as_deref(), Some("c1"));
    assert_eq!(parsed.result.tool_calls[1].id.as_deref(), Some("c2"));
}

#[test]
fn should_leave_tool_success_unknown() {
    // research/01 #4241: tool result lines carry no outcome. A-14 / T-302 (live capture): a result that numbers its
    // lines IS the evidence the read succeeded, so only an unnumbered result (an error message) stays unknown.
    let parsed = run(&fixture_lines(), Some(0)).unwrap();
    assert_eq!(parsed.result.tool_calls[0].success, Some(true));
    let lines = s(&[
        r#"{"role":"assistant","content":"","tool_calls":[{"id":"e1","function":{"name":"Read","arguments":"{\"path\":\"/m/missing.rs\"}"}}]}"#,
        r#"{"role":"tool","tool_call_id":"e1","content":"Error: file not found"}"#,
        r#"{"role":"assistant","content":"done"}"#,
    ]);
    let parsed = run(&lines, Some(0)).unwrap();
    assert_eq!(parsed.result.tool_calls[0].success, None);
    assert!(parsed.result.tool_calls[0].returned_lines.is_empty());
}

#[test]
fn should_tolerate_crlf_terminated_lines() {
    // NDJSON robustness: trailing \r does not break a line
    let lines: Vec<String> = fixture_lines().into_iter().map(|l| format!("{l}\r")).collect();
    let parsed = run(&lines, Some(0)).unwrap();
    assert!(matches!(parsed.outcome, SeatOutcome::Success));
    assert_eq!(parsed.result.session_id.as_deref(), Some("sess-kimi-fixture-0001"));
    assert_eq!(parsed.result.response_text, "main.rs defines an empty main function.");
}

#[test]
fn should_not_leak_state_between_parser_instances() {
    // new() starts with nothing seen
    let _ = run(&fixture_lines(), Some(0)).unwrap();
    let parsed = run(&[], Some(0)).unwrap();
    assert_eq!(failure_of(&parsed), "empty_or_unrecognized_stream:kimi");
}

#[test]
fn should_preserve_unicode_answer_text() {
    // text passes through untouched
    let parsed = run(&stream("/u/x.rs", "caf\\u00e9 done", Some("sess-u")), Some(0)).unwrap();
    assert_eq!(parsed.result.response_text, "caf\u{e9} done");
}

// ---- Round 2 (survivor feedback): contract-derived ToolKind, growing counts, tool_result pairing ----

fn one_call(name: &str) -> Vec<String> {
    vec![
        format!(
            r#"{{"role":"assistant","content":"","tool_calls":[{{"type":"function","id":"c1","function":{{"name":"{name}","arguments":"{{}}"}}}}]}}"#
        ),
        r#"{"role":"assistant","content":"done"}"#.to_string(),
    ]
}

fn kind_of(name: &str) -> (String, ToolKind) {
    let parsed = run(&one_call(name), Some(0)).unwrap();
    assert_eq!(parsed.result.tool_calls.len(), 1, "one record for {name}");
    let tc = &parsed.result.tool_calls[0];
    (tc.tool.clone(), tc.kind.clone())
}

#[test]
fn should_map_read_to_readfile_kind() {
    // reference_tools.md: Read is a file read
    assert_eq!(kind_of("Read"), ("Read".to_string(), ToolKind::ReadFile));
}

#[test]
fn should_map_write_to_writefile_kind() {
    // reference_tools.md: Write creates or overwrites a file
    assert_eq!(kind_of("Write"), ("Write".to_string(), ToolKind::WriteFile));
}

#[test]
fn should_map_edit_to_editfile_kind() {
    // reference_tools.md: Edit is string replacement
    assert_eq!(kind_of("Edit"), ("Edit".to_string(), ToolKind::EditFile));
}

#[test]
fn should_map_bash_to_bash_kind() {
    // reference_tools.md: Bash executes a shell command
    assert_eq!(kind_of("Bash"), ("Bash".to_string(), ToolKind::Bash));
}

#[test]
fn should_map_grep_to_grep_kind() {
    // reference_tools.md: Grep is ripgrep search
    assert_eq!(kind_of("Grep"), ("Grep".to_string(), ToolKind::Grep));
}

#[test]
fn should_map_glob_to_glob_kind() {
    // reference_tools.md: Glob matches files by pattern
    assert_eq!(kind_of("Glob"), ("Glob".to_string(), ToolKind::Glob));
}

#[test]
fn should_map_ask_user_question_to_request_user_input_kind() {
    // task contract: AskUserQuestion is interactive
    assert_eq!(
        kind_of("AskUserQuestion"),
        ("AskUserQuestion".to_string(), ToolKind::RequestUserInput)
    );
}

#[test]
fn should_map_unlisted_tool_to_unknown_kind() {
    // a name outside the mapped set must not be classed as a read or write
    assert_eq!(kind_of("FetchURL"), ("FetchURL".to_string(), ToolKind::Unknown));
}

#[test]
fn should_keep_distinct_kinds_across_one_message_in_order() {
    // each call keeps its own kind, in stream order
    let lines = s(&[
        r#"{"role":"assistant","content":"","tool_calls":[{"id":"a","function":{"name":"Grep","arguments":"{}"}},{"id":"b","function":{"name":"Write","arguments":"{}"}},{"id":"c","function":{"name":"Read","arguments":"{}"}}]}"#,
        r#"{"role":"assistant","content":"x"}"#,
    ]);
    let r = run(&lines, Some(0)).unwrap().result;
    let kinds: Vec<ToolKind> = r.tool_calls.iter().map(|c| c.kind.clone()).collect();
    assert_eq!(kinds, vec![ToolKind::Grep, ToolKind::WriteFile, ToolKind::ReadFile]);
}

#[test]
fn should_record_five_tool_calls_across_messages() {
    // counts grow past one
    let mut lines = Vec::new();
    for i in 0..5 {
        lines.push(format!(
            r#"{{"role":"assistant","content":"","tool_calls":[{{"id":"c{i}","function":{{"name":"Read","arguments":"{{\"path\":\"/f/{i}.rs\"}}"}}}}]}}"#
        ));
        lines.push(format!(r#"{{"role":"tool","tool_call_id":"c{i}","content":"body {i}"}}"#));
    }
    lines.push(r#"{"role":"assistant","content":"all read"}"#.to_string());
    let parsed = run(&lines, Some(0)).unwrap();
    assert!(matches!(parsed.outcome, SeatOutcome::Success));
    assert_eq!(parsed.result.tool_calls.len(), 5);
    for i in 0..5 {
        assert_eq!(parsed.result.tool_calls[i].id.as_deref(), Some(format!("c{i}").as_str()));
        assert!(args_of(&parsed, i).contains(&format!("/f/{i}.rs")));
    }
}

#[test]
fn should_emit_tool_call_started_event_per_call() {
    // 01-kimi-code-contract: assistant tool_call maps to ToolCallStarted
    let mut p = KimiStreamParser::new();
    let ev = p
        .parse_line(r#"{"role":"assistant","content":"","tool_calls":[{"id":"c1","function":{"name":"Read","arguments":"{}"}}]}"#)
        .expect("event for tool call");
    assert!(matches!(ev.state, WorkingState::ToolCallStarted));
    assert_eq!(ev.tool_name.as_deref(), Some("Read"));
}

#[test]
fn should_emit_tool_call_completed_for_each_of_several_results() {
    // 01-kimi-code-contract: tool result maps to ToolCallCompleted, every time (counts past one)
    let mut p = KimiStreamParser::new();
    let _ = p.parse_line(
        r#"{"role":"assistant","content":"","tool_calls":[{"id":"a","function":{"name":"Read","arguments":"{}"}},{"id":"b","function":{"name":"Grep","arguments":"{}"}},{"id":"c","function":{"name":"Glob","arguments":"{}"}}]}"#,
    );
    for id in ["a", "b", "c"] {
        let ev = p
            .parse_line(&format!(r#"{{"role":"tool","tool_call_id":"{id}","content":"r"}}"#))
            .expect("completed event");
        assert!(matches!(ev.state, WorkingState::ToolCallCompleted), "result {id}");
    }
}

#[test]
fn should_pair_tool_result_with_its_call_by_id() {
    // tool result pairs to the call with the same tool_call_id, not the first or last call
    let mut p = KimiStreamParser::new();
    let _ = p.parse_line(
        r#"{"role":"assistant","content":"","tool_calls":[{"id":"a","function":{"name":"Read","arguments":"{}"}},{"id":"b","function":{"name":"Grep","arguments":"{}"}},{"id":"c","function":{"name":"Glob","arguments":"{}"}}]}"#,
    );
    let ev = p.parse_line(r#"{"role":"tool","tool_call_id":"b","content":"r"}"#).expect("event");
    assert!(matches!(ev.state, WorkingState::ToolCallCompleted));
    assert_eq!(ev.tool_name.as_deref(), Some("Grep"));
    let ev = p.parse_line(r#"{"role":"tool","tool_call_id":"a","content":"r"}"#).expect("event");
    assert_eq!(ev.tool_name.as_deref(), Some("Read"));
}

#[test]
fn should_not_pair_tool_result_with_unknown_id() {
    // a result whose id matches no call must not borrow another call's name
    let mut p = KimiStreamParser::new();
    let _ = p.parse_line(
        r#"{"role":"assistant","content":"","tool_calls":[{"id":"a","function":{"name":"Read","arguments":"{}"}}]}"#,
    );
    if let Some(ev) = p.parse_line(r#"{"role":"tool","tool_call_id":"zzz","content":"r"}"#) {
        assert_ne!(ev.tool_name.as_deref(), Some("Read"));
    }
}

#[test]
fn should_keep_tool_success_none_after_several_results() {
    // #4241: no outcome in tool result lines, for every call
    let lines = s(&[
        r#"{"role":"assistant","content":"","tool_calls":[{"id":"a","function":{"name":"Read","arguments":"{}"}},{"id":"b","function":{"name":"Bash","arguments":"{}"}}]}"#,
        r#"{"role":"tool","tool_call_id":"a","content":"ok"}"#,
        r#"{"role":"tool","tool_call_id":"b","content":"Error: failed"}"#,
        r#"{"role":"assistant","content":"x"}"#,
    ]);
    let r = run(&lines, Some(0)).unwrap().result;
    assert_eq!(r.tool_calls.len(), 2);
    assert!(r.tool_calls.iter().all(|c| c.success.is_none()));
}

#[test]
fn should_emit_event_for_each_of_several_retries() {
    // retry lines are surfaced each time (counts past one)
    let mut p = KimiStreamParser::new();
    for n in [429, 429, 500] {
        let ev = p.parse_line(&format!(r#"{{"role":"meta","type":"turn.step.retrying","status_code":{n}}}"#));
        assert!(ev.is_some(), "retry {n}");
    }
}

#[test]
fn should_report_last_retry_after_many_retries_and_many_lines() {
    // last_retry_status survives a long stream with several retries; the most recent in stream order wins
    let mut lines = fixture_lines();
    for n in [500, 502, 429, 503, 429] {
        lines.insert(2, format!(r#"{{"role":"meta","type":"turn.step.retrying","status_code":{n}}}"#));
    }
    // inserting at index 2 each time reverses order, so the last in stream order is the first inserted: 500
    let parsed = run(&lines, Some(7)).unwrap();
    assert_eq!(failure_of(&parsed), "nonzero_exit:kimi:7 last_retry_status=500");
}

#[test]
fn should_fail_unrecognized_stream_made_only_of_unknown_lines() {
    // unknown role and unknown meta only: nothing recognized, exit 0 is a failure
    let lines = s(&[
        r#"{"role":"meta","type":"future.a"}"#,
        r#"{"role":"meta","type":"future.b"}"#,
        r#"{"role":"observer","content":"hi"}"#,
    ]);
    assert_eq!(failure_of(&run(&lines, Some(0)).unwrap()), "empty_or_unrecognized_stream:kimi");
}

#[test]
fn should_succeed_with_text_after_many_meta_and_tool_lines() {
    // many recognized lines of mixed kinds, then text: Success
    let mut lines = Vec::new();
    for _ in 0..4 {
        lines.push(r#"{"role":"meta","type":"turn.step.retrying","status_code":429}"#.to_string());
        lines.push(r#"{"role":"tool","tool_call_id":"nope","content":"r"}"#.to_string());
    }
    lines.push(r#"{"role":"assistant","content":"final"}"#.to_string());
    let parsed = run(&lines, Some(0)).unwrap();
    assert!(matches!(parsed.outcome, SeatOutcome::Success));
    assert_eq!(parsed.result.response_text, "final");
}
