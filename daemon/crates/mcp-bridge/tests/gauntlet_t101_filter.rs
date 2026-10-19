// GAUNTLET filter suite for T-101, written blind from the spec (no implementation read).
//
// SPEC_AMBIGUITY: SeatInvocation.env_add / env_remove for Kimi: BACKEND_STRUCTURE section 5 mentions KIMI_DISABLE_TELEMETRY=1
// under env_add but the builder doc says the env scrub is left to containment::child_env. No exact env_add/env_remove
// contract is stated for the builder, so no test asserts them.
// SPEC_AMBIGUITY: the exact error text after the code is not specified for forbidden extra args (only that they are
// rejected), so those tests assert Err only. `prompt_too_large_for_kimi` is asserted as the leading code.
// SPEC_AMBIGUITY: whether the 64 KiB cap counts bytes or chars is inferred from the constant name KIMI_PROMPT_LIMIT_BYTES.

use mcp_bridge::containment::SeatRequest;
use mcp_bridge::kimi::{
    KIMI_FORBIDDEN_EXTRA_ARGS, KIMI_PROGRAM, KIMI_PROMPT_LIMIT_BYTES, RO_AGENT_FILE,
    build_kimi_invocation,
};
use std::path::Path;
use std::time::Duration;

fn req<'a>(
    prompt: &'a str,
    cwd: &'a Path,
    home: &'a Path,
    out: &'a Path,
    model: Option<&'a str>,
    resume: Option<&'a str>,
    extra: &'a [String],
) -> SeatRequest<'a> {
    SeatRequest {
        prompt,
        cwd,
        seat_home: home,
        output_dir: out,
        model,
        resume_session_id: resume,
        extra_args: extra,
        max_turns: 10,
        time_limit: Duration::from_secs(60),
    }
}

fn agent_file(home: &Path) -> String {
    home.join(RO_AGENT_FILE).to_string_lossy().into_owned()
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn should_expose_spec_constants() {
    // REQ-021 / REQ-024 constants
    assert_eq!(KIMI_PROGRAM, "kimi");
    assert_eq!(RO_AGENT_FILE, "ro-agent.md");
    assert_eq!(KIMI_PROMPT_LIMIT_BYTES, 65_536);
}

#[test]
fn should_list_exactly_the_spec_forbidden_flags() {
    // BACKEND_STRUCTURE section 6 forbidden list for Kimi
    let want = [
        "--continue", "-c", "--yolo", "-y", "--auto", "--plan", "--session", "-S", "-r", "--resume",
        "--output-format", "-p", "--prompt", "--agent", "--agent-file", "-m", "--model",
    ];
    assert_eq!(KIMI_FORBIDDEN_EXTRA_ARGS.len(), want.len());
    for w in want {
        assert!(KIMI_FORBIDDEN_EXTRA_ARGS.contains(&w), "missing {w}");
    }
}

#[test]
fn should_build_exact_creation_argv() {
    // REQ-021 creation argv: -p, stream-json, --agent-file <home>/ro-agent.md
    let (cwd, home, out) = (Path::new("/work/repo"), Path::new("/seat/home"), Path::new("/seat/out"));
    let extra: Vec<String> = vec![];
    let inv = build_kimi_invocation(&req("hello", cwd, home, out, None, None, &extra)).unwrap();
    assert_eq!(inv.program, "kimi");
    assert_eq!(
        inv.args,
        vec!["-p", "hello", "--output-format", "stream-json", "--agent-file", "/seat/home/ro-agent.md"]
    );
    assert_eq!(inv.cwd, cwd.to_path_buf());
}

#[test]
fn should_build_exact_creation_argv_with_model() {
    // REQ-021: [-m <model>] after the agent file
    let (cwd, home, out) = (Path::new("/work/repo"), Path::new("/seat/home"), Path::new("/seat/out"));
    let extra: Vec<String> = vec![];
    let inv = build_kimi_invocation(&req("hi", cwd, home, out, Some("kimi-k2"), None, &extra)).unwrap();
    assert_eq!(
        inv.args,
        vec![
            "-p", "hi", "--output-format", "stream-json", "--agent-file", "/seat/home/ro-agent.md", "-m",
            "kimi-k2"
        ]
    );
}

#[test]
fn should_build_exact_resume_argv_without_agent_file() {
    // REQ-021: resume uses --session <id> and never --agent-file
    let (cwd, home, out) = (Path::new("/work/repo"), Path::new("/seat/home"), Path::new("/seat/out"));
    let extra: Vec<String> = vec![];
    let inv = build_kimi_invocation(&req("again", cwd, home, out, None, Some("sess-abc"), &extra)).unwrap();
    assert_eq!(
        inv.args,
        vec!["-p", "again", "--output-format", "stream-json", "--session", "sess-abc"]
    );
    assert!(!inv.args.iter().any(|a| a == "--agent-file"));
    assert!(!inv.args.iter().any(|a| a.contains("ro-agent.md")));
}

#[test]
fn should_build_resume_argv_with_model_after_session() {
    // BACKEND_STRUCTURE section 6 resume + [-m <model>]
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    let extra: Vec<String> = vec![];
    let inv = build_kimi_invocation(&req("p", cwd, home, out, Some("m1"), Some("s9"), &extra)).unwrap();
    assert_eq!(
        inv.args,
        vec!["-p", "p", "--output-format", "stream-json", "--session", "s9", "-m", "m1"]
    );
}

#[test]
fn should_never_emit_continue_flag() {
    // REQ-021: never --continue
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    let extra: Vec<String> = vec![];
    for resume in [None, Some("s1")] {
        let inv = build_kimi_invocation(&req("p", cwd, home, out, None, resume, &extra)).unwrap();
        assert!(!inv.args.iter().any(|a| a == "--continue" || a == "-c"));
    }
}

#[test]
fn should_append_allowed_extra_args_at_end() {
    // BACKEND_STRUCTURE section 6: [extra...] trails the argv
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    let extra = strs(&["--verbose"]);
    let inv = build_kimi_invocation(&req("p", cwd, home, out, Some("m"), None, &extra)).unwrap();
    assert_eq!(
        inv.args,
        vec!["-p", "p", "--output-format", "stream-json", "--agent-file", "/h/ro-agent.md", "-m", "m", "--verbose"]
    );
}

#[test]
fn should_reject_every_forbidden_flag_on_creation() {
    // REQ-020 forbidden extra args, alone
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    for flag in KIMI_FORBIDDEN_EXTRA_ARGS {
        let extra = strs(&[flag]);
        let r = build_kimi_invocation(&req("p", cwd, home, out, None, None, &extra));
        assert!(r.is_err(), "creation must reject {flag}");
    }
}

#[test]
fn should_reject_every_forbidden_flag_on_resume() {
    // REQ-020 forbidden extra args, resume path
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    for flag in KIMI_FORBIDDEN_EXTRA_ARGS {
        let extra = strs(&[flag]);
        let r = build_kimi_invocation(&req("p", cwd, home, out, None, Some("s"), &extra));
        assert!(r.is_err(), "resume must reject {flag}");
    }
}

#[test]
fn should_reject_forbidden_flag_in_equals_form() {
    // builder doc: forbidden flag "alone or in --flag=value form"
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    for flag in ["--session=abc", "--model=x", "--agent-file=/etc/x", "--output-format=json", "--continue=1", "--yolo=true"] {
        let extra = strs(&[flag]);
        let r = build_kimi_invocation(&req("p", cwd, home, out, None, None, &extra));
        assert!(r.is_err(), "must reject {flag}");
    }
}

#[test]
fn should_reject_forbidden_flag_among_allowed_extras() {
    // a forbidden flag after an allowed one is still rejected
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    let extra = strs(&["--verbose", "--continue"]);
    assert!(build_kimi_invocation(&req("p", cwd, home, out, None, None, &extra)).is_err());
}

#[test]
fn should_not_reject_flags_that_merely_share_a_prefix() {
    // "alone or in --flag=value form" means exact flag match, not prefix match
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    let extra = strs(&["--continued"]);
    let inv = build_kimi_invocation(&req("p", cwd, home, out, None, None, &extra)).unwrap();
    assert_eq!(inv.args.last().map(String::as_str), Some("--continued"));
}

#[test]
fn should_accept_prompt_of_exactly_64_kib() {
    // REQ-024 boundary: exactly the limit is allowed
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    let prompt = "a".repeat(KIMI_PROMPT_LIMIT_BYTES);
    let extra: Vec<String> = vec![];
    let inv = build_kimi_invocation(&req(&prompt, cwd, home, out, None, None, &extra)).unwrap();
    assert_eq!(inv.args[1].len(), 65_536);
}

#[test]
fn should_reject_prompt_one_byte_over_limit() {
    // REQ-024 boundary: limit + 1
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    let prompt = "a".repeat(KIMI_PROMPT_LIMIT_BYTES + 1);
    let extra: Vec<String> = vec![];
    let err = build_kimi_invocation(&req(&prompt, cwd, home, out, None, None, &extra)).unwrap_err();
    assert!(err.starts_with("prompt_too_large_for_kimi"), "got: {err}");
}

#[test]
fn should_reject_65_kib_prompt_with_code() {
    // reality_test: a 65 KiB prompt returns prompt_too_large_for_kimi
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    let prompt = "x".repeat(65 * 1024);
    let extra: Vec<String> = vec![];
    let err = build_kimi_invocation(&req(&prompt, cwd, home, out, None, Some("s"), &extra)).unwrap_err();
    assert!(err.starts_with("prompt_too_large_for_kimi"), "got: {err}");
}

#[test]
fn should_count_prompt_limit_in_bytes_not_chars() {
    // 21_846 three-byte chars = 65_538 bytes but only 21_846 chars
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    let prompt = "\u{20ac}".repeat(21_846);
    assert!(prompt.len() > KIMI_PROMPT_LIMIT_BYTES);
    let extra: Vec<String> = vec![];
    let err = build_kimi_invocation(&req(&prompt, cwd, home, out, None, None, &extra)).unwrap_err();
    assert!(err.starts_with("prompt_too_large_for_kimi"), "got: {err}");
}

#[test]
fn should_pass_prompt_through_untruncated_and_unaltered() {
    // REQ-024: never truncate; unicode and newlines intact
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    let prompt = "line1\nline2 \u{1F600} \"quoted\" --not-a-flag";
    let extra: Vec<String> = vec![];
    let inv = build_kimi_invocation(&req(prompt, cwd, home, out, None, None, &extra)).unwrap();
    assert_eq!(inv.args[0], "-p");
    assert_eq!(inv.args[1], prompt);
}

#[test]
fn should_bind_cwd_and_agent_file_to_request_values() {
    // REQ-021: cwd = the session's bound cwd; agent file under the given seat home
    for (cwd, home) in [("/a/b", "/x/homes/kimi"), ("/c", "/y/z/homes/kimi")] {
        let (cwd, home, out) = (Path::new(cwd), Path::new(home), Path::new("/o"));
        let extra: Vec<String> = vec![];
        let inv = build_kimi_invocation(&req("p", cwd, home, out, None, None, &extra)).unwrap();
        assert_eq!(inv.cwd, cwd.to_path_buf());
        assert!(inv.args.contains(&agent_file(home)));
    }
}

#[test]
fn should_be_deterministic_for_same_request() {
    // purity: same request, same argv
    let (cwd, home, out) = (Path::new("/w"), Path::new("/h"), Path::new("/o"));
    let extra = strs(&["--verbose"]);
    let a = build_kimi_invocation(&req("p", cwd, home, out, Some("m"), Some("s"), &extra)).unwrap();
    let b = build_kimi_invocation(&req("p", cwd, home, out, Some("m"), Some("s"), &extra)).unwrap();
    assert_eq!(a.args, b.args);
    assert_eq!(a.program, b.program);
}
