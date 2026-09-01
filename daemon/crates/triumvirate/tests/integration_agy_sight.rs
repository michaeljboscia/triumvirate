//! End to end: does a LIVE agy turn produce the tool-call record the sight gate needs?
//!
//! Offline tests prove the parser handles captured fixtures. Fixtures go stale. This is the
//! test that catches Google changing the stream shape, which would silently re-lock
//! Antigravity out of the sight gate while the whole offline suite stayed green. That exact
//! failure mode, an agent unable to report what it did, is what this work exists to fix.
//!
//! Opt in with TRIUMVIRATE_LIVE_AGY=1. Spends subscription quota.

use std::process::Command;

fn live() -> bool {
    std::env::var("TRIUMVIRATE_LIVE_AGY").map(|v| v == "1").unwrap_or(false)
}

/// A live agy turn that must open a file records a read whose arguments name that file.
///
/// Asserts the full chain the gate depends on: live CLI, real parser, populated tool_calls,
/// arguments preserved, and a parser mode the gate trusts.
#[test]
#[ignore = "live: set TRIUMVIRATE_LIVE_AGY=1; spends subscription quota"]
fn e_agy_sight_01_a_live_turn_records_the_file_it_opened() {
    if !live() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("agy-sight-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let marker = "QUARTZ_MERIDIAN_88";
    let file = dir.join("evidence.txt");
    std::fs::write(&file, format!("the marker is {marker}\n")).expect("fixture");

    let out = Command::new("agy")
        .args([
            "--dangerously-skip-permissions",
            "--output-format",
            "stream-json",
            "--print-timeout",
            "4m",
            "--prompt",
        ])
        .arg(format!(
            "Read the file {} and reply with only the marker value it contains.",
            file.display()
        ))
        .current_dir(&dir)
        .output()
        .expect("agy must run");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut p = agent_adapter::AgyStreamParser::new();
    for line in stdout.lines() {
        p.parse_line(line);
    }
    assert!(p.saw_result(), "stream must reach a terminal result event");
    let parsed = p.finish();

    assert_eq!(
        parsed.parser_mode,
        agent_adapter::AGY_PARSER_MODE_STREAM,
        "must emit the mode the sight gate allowlist trusts"
    );
    assert!(
        !parsed.tool_calls.is_empty(),
        "a live agy turn that read a file MUST record tool calls, or Antigravity is locked \
         out of the sight gate again; response was: {}",
        parsed.response_text
    );
    let args: Vec<String> = parsed
        .tool_calls
        .iter()
        .filter_map(|c| c.args_json.clone())
        .collect();
    assert!(
        args.iter().any(|a| a.contains("evidence.txt")),
        "the opened path must appear in recorded arguments or required_sources cannot be \
         enforced for agy; got: {args:?}"
    );
    assert!(
        parsed.response_text.contains(marker),
        "agy should have actually read the file; got: {}",
        parsed.response_text
    );
    assert_eq!(parsed.session_id, None, "agy is single-turn (REQ-040/042)");
    let _ = std::fs::remove_dir_all(&dir);
}
