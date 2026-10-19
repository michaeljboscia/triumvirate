// GAUNTLET net suite for T-101, written blind from the spec (no implementation read).
// SPEC_AMBIGUITY: `containment` and `kimi` are assumed to be public modules of mcp_bridge. The mcp-bridge lib.rs is not in the package.
// SPEC_AMBIGUITY: the forbidden-extra-arg error carries no code in the spec, so those cases assert only is_err().
// SPEC_AMBIGUITY: the spec's "True for a forbidden flag, alone or in --flag=value form" has no function signature. The =value form is tested through build_kimi_invocation.
// SPEC_AMBIGUITY: the 64 KiB cap is read as inclusive of KIMI_PROMPT_LIMIT_BYTES (65_536 bytes passes, one more byte fails).
// SPEC_AMBIGUITY: the builder's env_add and env_remove for Kimi are not specified, so they are not asserted.

use std::path::PathBuf;
use std::time::Duration;

use mcp_bridge::containment::SeatRequest;
use mcp_bridge::kimi::{
    KIMI_FORBIDDEN_EXTRA_ARGS, KIMI_PROGRAM, KIMI_PROMPT_LIMIT_BYTES, RO_AGENT_FILE,
    build_kimi_invocation,
};

struct Fixture {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    home: PathBuf,
    out: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    let home = dir.path().join("homes").join("kimi");
    let out = dir.path().join("out");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    Fixture {
        _dir: dir,
        repo,
        home,
        out,
    }
}

fn request<'a>(
    f: &'a Fixture,
    prompt: &'a str,
    extra: &'a [String],
    resume: Option<&'a str>,
    model: Option<&'a str>,
) -> SeatRequest<'a> {
    SeatRequest {
        prompt,
        cwd: &f.repo,
        seat_home: &f.home,
        output_dir: &f.out,
        model,
        resume_session_id: resume,
        extra_args: extra,
        max_turns: 10,
        time_limit: Duration::from_secs(300),
    }
}

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn ro_agent_path(f: &Fixture) -> String {
    f.home.join(RO_AGENT_FILE).to_string_lossy().into_owned()
}

// REQ-021 (constants): program is `kimi`, the agent file is `ro-agent.md`, the cap is 64 KiB in bytes.
#[test]
fn constants_match_spec() {
    assert_eq!(KIMI_PROGRAM, "kimi");
    assert_eq!(RO_AGENT_FILE, "ro-agent.md");
    assert_eq!(KIMI_PROMPT_LIMIT_BYTES, 65_536);
}

// REQ-020/021 (forbidden list): the forbidden extra args are exactly the 17 names the spec lists.
#[test]
fn forbidden_list_is_exact() {
    let mut got: Vec<&str> = KIMI_FORBIDDEN_EXTRA_ARGS.to_vec();
    got.sort();
    let mut want = vec![
        "--continue", "-c", "--yolo", "-y", "--auto", "--plan", "--session", "-S", "-r",
        "--resume", "--output-format", "-p", "--prompt", "--agent", "--agent-file", "-m",
        "--model",
    ];
    want.sort();
    assert_eq!(got, want);
}

// REQ-021: creation argv is exactly `-p <p> --output-format stream-json --agent-file <seat_home>/ro-agent.md`.
#[test]
fn creation_argv_is_exact() {
    let f = fixture();
    let extra: Vec<String> = vec![];
    let inv = build_kimi_invocation(&request(&f, "hello", &extra, None, None))
        .expect("creation builds");
    let agent = ro_agent_path(&f);
    assert_eq!(inv.program, "kimi");
    assert_eq!(inv.cwd, f.repo);
    assert_eq!(
        inv.args,
        args(&["-p", "hello", "--output-format", "stream-json", "--agent-file", agent.as_str()])
    );
}

// REQ-021: creation with a model appends `-m <model>` after the agent file, before extra args.
#[test]
fn creation_argv_with_model_and_extra_is_ordered() {
    let f = fixture();
    let extra = args(&["--x-test-flag"]);
    let inv = build_kimi_invocation(&request(&f, "hi", &extra, None, Some("kimi-k2")))
        .expect("creation builds");
    let agent = ro_agent_path(&f);
    assert_eq!(
        inv.args,
        args(&[
            "-p",
            "hi",
            "--output-format",
            "stream-json",
            "--agent-file",
            agent.as_str(),
            "-m",
            "kimi-k2",
            "--x-test-flag",
        ])
    );
}

// REQ-021: resume argv is exactly `-p <p> --output-format stream-json --session <id>`, with no --agent-file.
#[test]
fn resume_argv_is_exact_and_has_no_agent_file() {
    let f = fixture();
    let extra: Vec<String> = vec![];
    let inv = build_kimi_invocation(&request(&f, "hi", &extra, Some("sess-abc"), None))
        .expect("resume builds");
    assert_eq!(
        inv.args,
        args(&["-p", "hi", "--output-format", "stream-json", "--session", "sess-abc"])
    );
    assert!(!inv.args.iter().any(|a| a == "--agent-file"));
}

// REQ-021: every forbidden extra arg is rejected on a creation call (bare name).
#[test]
fn every_forbidden_extra_is_rejected_on_creation() {
    let f = fixture();
    for flag in KIMI_FORBIDDEN_EXTRA_ARGS {
        let extra = args(&[flag]);
        let res = build_kimi_invocation(&request(&f, "hi", &extra, None, None));
        assert!(res.is_err(), "forbidden extra {flag} was accepted on creation");
    }
}

// REQ-021: every forbidden extra arg is rejected on a resume call (bare name).
#[test]
fn every_forbidden_extra_is_rejected_on_resume() {
    let f = fixture();
    for flag in KIMI_FORBIDDEN_EXTRA_ARGS {
        let extra = args(&[flag]);
        let res = build_kimi_invocation(&request(&f, "hi", &extra, Some("sess-abc"), None));
        assert!(res.is_err(), "forbidden extra {flag} was accepted on resume");
    }
}

// REQ-021: a forbidden flag in `--flag=value` form is rejected (the spec's "alone or in --flag=value form").
#[test]
fn forbidden_flag_in_equals_form_is_rejected() {
    let f = fixture();
    for raw in ["--continue=1", "--agent-file=/etc/x.md", "--model=gpt", "--session=abc", "--output-format=text"] {
        let extra = args(&[raw]);
        let res = build_kimi_invocation(&request(&f, "hi", &extra, None, None));
        assert!(res.is_err(), "equals-form {raw} was accepted");
    }
}

// REQ-021: a non-forbidden extra arg is passed through as the last argv entry.
#[test]
fn allowed_extra_arg_passes_through() {
    let f = fixture();
    let extra = args(&["--x-test-flag"]);
    let inv = build_kimi_invocation(&request(&f, "hi", &extra, None, None)).expect("builds");
    assert_eq!(inv.args.last().map(String::as_str), Some("--x-test-flag"));
}

// REQ-021: a prompt that merely contains a forbidden flag name is the prompt, not an extra arg, so it is accepted and travels as the -p value.
#[test]
fn prompt_text_that_names_a_forbidden_flag_is_accepted() {
    let f = fixture();
    let extra: Vec<String> = vec![];
    let inv = build_kimi_invocation(&request(&f, "--continue please", &extra, None, None))
        .expect("prompt text is not an extra arg");
    assert_eq!(inv.args[0], "-p");
    assert_eq!(inv.args[1], "--continue please");
}

// REQ-024: a 65 KiB prompt (65 * 1024 bytes) returns `prompt_too_large_for_kimi` on creation.
#[test]
fn sixty_five_kib_prompt_is_too_large_on_creation() {
    let f = fixture();
    let prompt = "a".repeat(65 * 1024);
    let extra: Vec<String> = vec![];
    let err = build_kimi_invocation(&request(&f, &prompt, &extra, None, None))
        .expect_err("65 KiB must be refused");
    assert!(err.starts_with("prompt_too_large_for_kimi"), "got: {err}");
}

// REQ-024: the same 65 KiB prompt is refused on resume too.
#[test]
fn sixty_five_kib_prompt_is_too_large_on_resume() {
    let f = fixture();
    let prompt = "a".repeat(65 * 1024);
    let extra: Vec<String> = vec![];
    let err = build_kimi_invocation(&request(&f, &prompt, &extra, Some("sess-abc"), None))
        .expect_err("65 KiB must be refused");
    assert!(err.starts_with("prompt_too_large_for_kimi"), "got: {err}");
}

// REQ-024 (boundary): exactly KIMI_PROMPT_LIMIT_BYTES passes and one byte more is refused.
#[test]
fn prompt_limit_boundary_is_exact() {
    let f = fixture();
    let extra: Vec<String> = vec![];
    let at_limit = "a".repeat(KIMI_PROMPT_LIMIT_BYTES);
    assert!(build_kimi_invocation(&request(&f, &at_limit, &extra, None, None)).is_ok());
    let over = "a".repeat(KIMI_PROMPT_LIMIT_BYTES + 1);
    let err = build_kimi_invocation(&request(&f, &over, &extra, None, None))
        .expect_err("one byte over the cap must be refused");
    assert!(err.starts_with("prompt_too_large_for_kimi"), "got: {err}");
}

// REQ-024 (bytes, not chars): 32_768 two-byte chars is exactly 65_536 bytes and passes; one more ASCII byte is refused.
#[test]
fn prompt_limit_counts_bytes_not_chars() {
    let f = fixture();
    let extra: Vec<String> = vec![];
    let at_limit = "\u{e9}".repeat(32_768);
    assert_eq!(at_limit.len(), KIMI_PROMPT_LIMIT_BYTES);
    assert!(build_kimi_invocation(&request(&f, &at_limit, &extra, None, None)).is_ok());
    let over = format!("{at_limit}a");
    let err = build_kimi_invocation(&request(&f, &over, &extra, None, None))
        .expect_err("one byte over the cap must be refused");
    assert!(err.starts_with("prompt_too_large_for_kimi"), "got: {err}");
}
