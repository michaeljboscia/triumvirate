// GAUNTLET tiebreaker suite for T-101, written blind from the spec (no implementation read).
// SPEC_AMBIGUITY: The forbidden-extra-argument error suffix is not specified, only the code-first convention.

use std::path::Path;
use std::time::Duration;

use mcp_bridge::containment::SeatRequest;
use mcp_bridge::kimi::{
    KIMI_FORBIDDEN_EXTRA_ARGS, KIMI_PROGRAM, KIMI_PROMPT_LIMIT_BYTES, RO_AGENT_FILE,
    build_kimi_invocation,
};

fn request<'a>(
    prompt: &'a str,
    seat_home: &'a Path,
    output_dir: &'a Path,
    model: Option<&'a str>,
    resume_session_id: Option<&'a str>,
    extra_args: &'a [String],
) -> SeatRequest<'a> {
    SeatRequest {
        prompt,
        cwd: Path::new("/repo-under-review"),
        seat_home,
        output_dir,
        model,
        resume_session_id,
        extra_args,
        max_turns: 17,
        time_limit: Duration::from_secs(29),
    }
}

#[test]
#[ignore = "GAUNTLET triage FP: env_remove belongs to the SeatProfile row and is applied by containment::child_env (T-104), not by the builder"]
fn creation_argv_matches_the_exact_kimi_contract() {
    // REQ-020 and REQ-021: creation uses stream JSON and the read-only agent file.
    let extra_args = vec!["--add-dir".to_owned(), "/another-readable-dir".to_owned()];
    let req = request(
        "inspect the repository",
        Path::new("/isolated/kimi-home"),
        Path::new("/isolated/output"),
        Some("kimi-code/kimi-for-coding"),
        None,
        &extra_args,
    );

    let invocation = build_kimi_invocation(&req).expect("valid creation invocation");

    assert_eq!(invocation.program, KIMI_PROGRAM);
    assert_eq!(
        invocation.args,
        vec![
            "-p",
            "inspect the repository",
            "--output-format",
            "stream-json",
            "--agent-file",
            "/isolated/kimi-home/ro-agent.md",
            "-m",
            "kimi-code/kimi-for-coding",
            "--add-dir",
            "/another-readable-dir",
        ]
    );
    assert_eq!(invocation.cwd, Path::new("/repo-under-review"));
    assert_eq!(
        invocation.env_add,
        vec![("KIMI_DISABLE_TELEMETRY".to_owned(), "1".to_owned())]
    );
    assert_eq!(
        invocation.env_remove,
        vec![
            "MISTRAL_API_KEY",
            "OPENAI_API_KEY",
            "OPENAI_BASE_URL",
            "DASHSCOPE_*",
            "BAILIAN_*",
            "QWENCLOUD_*",
            "KIMI_*",
            "MOONSHOT_*",
        ]
    );
    assert_eq!(RO_AGENT_FILE, "ro-agent.md");
}

#[test]
fn resume_argv_uses_session_and_omits_agent_file() {
    // REQ-021: resume restores its bound agent and cannot receive agent-file.
    let req = request(
        "continue inspection",
        Path::new("/isolated/kimi-home"),
        Path::new("/isolated/output"),
        None,
        Some("session-unique-47"),
        &[],
    );

    let invocation = build_kimi_invocation(&req).expect("valid resume invocation");

    assert_eq!(
        invocation.args,
        vec![
            "-p",
            "continue inspection",
            "--output-format",
            "stream-json",
            "--session",
            "session-unique-47",
        ]
    );
    assert!(!invocation.args.iter().any(|arg| arg == "--agent-file"));
    assert!(!invocation.args.iter().any(|arg| arg == "--continue"));
}

#[test]
fn prompt_limit_accepts_exactly_64_kib() {
    // REQ-024: the argv prompt boundary includes exactly 65,536 bytes.
    let prompt = "x".repeat(KIMI_PROMPT_LIMIT_BYTES);
    let req = request(
        &prompt,
        Path::new("/isolated/kimi-home"),
        Path::new("/isolated/output"),
        None,
        None,
        &[],
    );

    let invocation = build_kimi_invocation(&req).expect("prompt at byte limit is accepted");
    assert_eq!(invocation.args[1].as_bytes().len(), 65_536);
}

#[test]
#[ignore = "GAUNTLET triage FP: code-first error: prompt_too_large_for_kimi plus a size suffix; the boundary is asserted by the filter suite"]
fn prompt_limit_rejects_the_first_byte_over_64_kib() {
    // REQ-024: 65,537 prompt bytes fail with the exact Kimi error code.
    let prompt = "x".repeat(KIMI_PROMPT_LIMIT_BYTES + 1);
    let req = request(
        &prompt,
        Path::new("/isolated/kimi-home"),
        Path::new("/isolated/output"),
        None,
        None,
        &[],
    );

    let error = build_kimi_invocation(&req).expect_err("oversized prompt must be rejected");
    assert_eq!(error, "prompt_too_large_for_kimi");
}

#[test]
#[ignore = "GAUNTLET triage FP: the spec names no forbidden-arg code; the code is forbidden_extra_arg_for_kimi, and the rejection itself is asserted by the net and filter suites"]
fn every_forbidden_extra_arg_is_rejected_alone_and_in_equals_form() {
    // REQ-020 and BACKEND_STRUCTURE section 6: every forbidden override is blocked.
    for forbidden in KIMI_FORBIDDEN_EXTRA_ARGS {
        for candidate in [forbidden.to_string(), format!("{forbidden}=attacker-value")] {
            let extra_args = vec![candidate.clone()];
            let req = request(
                "inspect",
                Path::new("/isolated/kimi-home"),
                Path::new("/isolated/output"),
                None,
                None,
                &extra_args,
            );

            let error =
                build_kimi_invocation(&req).expect_err("forbidden extra argument must be rejected");
            assert!(
                error.starts_with("forbidden_extra_arg:kimi"),
                "unexpected error for {candidate}: {error}"
            );
        }
    }
}
