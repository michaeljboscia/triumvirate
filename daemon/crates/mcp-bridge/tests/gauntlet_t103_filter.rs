// GAUNTLET filter suite for T-103, written blind from the spec (no implementation read).
//
// Role: filter. Crate: mcp-bridge (Qwen invocation builder, plan_auth, served-model check). The parser half lives in
// daemon/crates/agent-adapter/tests/gauntlet_t103_filter.rs.
//
// SPEC_AMBIGUITY: the value format of `--max-wall-time` is not stated (the spec says "S", research/03 shows "10m"). Tests
// assert the flag is present, its value is non-empty, and different time limits give different values.
// SPEC_AMBIGUITY: whether non-forbidden operator extra args are passed through is not stated; not tested.
// SPEC_AMBIGUITY: `--flag=value` spellings of forbidden args (for example `--approval-mode=yolo`) are not covered by the
// FORBIDDEN_EXTRA_ARGS list; not tested.
// SPEC_AMBIGUITY: whether a non-Qwen `req.model` (for example "glm-5") is rejected by the builder is not stated; not tested.
// SPEC_AMBIGUITY: plan_auth for an unknown plan string (neither qwencloud nor modelstudio) is not stated; not tested.
// SPEC_AMBIGUITY: the exact error text for a refused `modelstudio` plan is not stated; only that it is an Err.
// SPEC_AMBIGUITY: the program name is taken from BACKEND_STRUCTURE section 6 (`qwen`).
// SPEC_AMBIGUITY: SeatInvocation.env_remove for the credential scrub list is T-201's row, not asserted here.

use std::time::Duration;

use agent_adapter::qwen::QwenStreamParser;
use mcp_bridge::containment::{SeatInvocation, SeatRequest};
use mcp_bridge::qwen::{
    DEFAULT_QWEN_MODEL, EXCLUDED_TOOLS, FORBIDDEN_EXTRA_ARGS, MAX_PROMPT_BYTES,
    UNATTENDED_RETRY_ENV, build_qwen_invocation, check_served_model, plan_auth,
};

const EXPECTED_EXCLUDED: [&str; 15] = [
    "run_shell_command",
    "exec",
    "edit",
    "write_file",
    "notebook_edit",
    "agent",
    "save_memory",
    "manage_memory",
    "cron_create",
    "cron_delete",
    "create_sub_session",
    "send_message",
    "team_create",
    "task_create",
    "image_gen",
];

struct Dirs {
    cwd: tempfile::TempDir,
    home: tempfile::TempDir,
    out: tempfile::TempDir,
}

fn dirs() -> Dirs {
    Dirs {
        cwd: tempfile::tempdir().unwrap(),
        home: tempfile::tempdir().unwrap(),
        out: tempfile::tempdir().unwrap(),
    }
}

fn build_with(
    d: &Dirs,
    prompt: &str,
    model: Option<&str>,
    resume: Option<&str>,
    extra: &[String],
    max_turns: u32,
    limit: Duration,
) -> Result<SeatInvocation, String> {
    let req = SeatRequest {
        prompt,
        cwd: d.cwd.path(),
        seat_home: d.home.path(),
        output_dir: d.out.path(),
        model,
        resume_session_id: resume,
        extra_args: extra,
        max_turns,
        time_limit: limit,
    };
    build_qwen_invocation(&req)
}

fn build(d: &Dirs, prompt: &str) -> SeatInvocation {
    build_with(
        d,
        prompt,
        Some("qwen3-coder-plus"),
        None,
        &[],
        12,
        Duration::from_secs(600),
    )
    .expect("builds")
}

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(|s| s.as_str())
}

fn count(args: &[String], flag: &str) -> usize {
    args.iter().filter(|a| a.as_str() == flag).count()
}

// ---------------------------------------------------------------- constants

// REQ-024: the prompt limit is 64 KiB.
#[test]
fn test_should_cap_prompt_at_64_kib() {
    assert_eq!(MAX_PROMPT_BYTES, 65536);
}

// TECH_STACK: the default pinned model is qwen3-coder-plus and starts with qwen.
#[test]
fn test_should_default_to_qwen3_coder_plus() {
    assert_eq!(DEFAULT_QWEN_MODEL, "qwen3.7-plus");
    assert!(DEFAULT_QWEN_MODEL.starts_with("qwen"));
}

// REQ-023: the exclude list is exactly the 15 named tools in order.
#[test]
fn test_should_list_exact_exclude_tools() {
    assert_eq!(EXCLUDED_TOOLS, &EXPECTED_EXCLUDED[..]);
}

// BACKEND_STRUCTURE section 6: forbidden extra args are exactly the Qwen column, plus `--auth-type` (A-13: the builder owns it).
#[test]
fn test_should_list_exact_forbidden_extra_args() {
    let expected = [
        "--continue",
        "--yolo",
        "-y",
        "--approval-mode",
        "--resume",
        "--output-format",
        "-o",
        "-p",
        "--prompt",
        "-m",
        "--model",
        "--safe-mode",
        "--auth-type",
        "--exclude-tools",
        "--allowed-tools",
    ];
    assert_eq!(FORBIDDEN_EXTRA_ARGS, &expected[..]);
}

// research/03: the retry env var name.
#[test]
fn test_should_name_the_unattended_retry_env_var() {
    assert_eq!(UNATTENDED_RETRY_ENV, "QWEN_CODE_UNATTENDED_RETRY");
}

// ---------------------------------------------------------------- argv contract (REQ-020, REQ-023)

// BACKEND_STRUCTURE section 6: the program is qwen.
#[test]
fn test_should_run_the_qwen_program() {
    let d = dirs();
    assert_eq!(build(&d, "hello").program, "qwen");
}

// REQ-023: -p carries the prompt verbatim.
#[test]
fn test_should_pass_prompt_after_dash_p() {
    let d = dirs();
    let sut = build(&d, "Review src/main.rs please");
    assert_eq!(
        value_after(&sut.args, "-p"),
        Some("Review src/main.rs please")
    );
}

// REQ-023: a prompt that looks like a flag is still just the value of -p.
#[test]
fn test_should_keep_flag_shaped_prompt_as_value_of_dash_p() {
    let d = dirs();
    let sut = build(&d, "--yolo");
    assert_eq!(value_after(&sut.args, "-p"), Some("--yolo"));
}

// REQ-023: output format is stream-json.
#[test]
fn test_should_request_stream_json_output() {
    let d = dirs();
    assert_eq!(
        value_after(&build(&d, "x").args, "--output-format"),
        Some("stream-json")
    );
}

// REQ-023: --safe-mode is on every call.
#[test]
fn test_should_always_pass_safe_mode() {
    let d = dirs();
    for resume in [None, Some("7f3c9a2e-5b1d-4e8a-9c40-2d6f1e8b7a01")] {
        let sut = build_with(
            &d,
            "x",
            Some("qwen3-coder-plus"),
            resume,
            &[],
            5,
            Duration::from_secs(60),
        )
        .unwrap();
        assert_eq!(count(&sut.args, "--safe-mode"), 1);
    }
}

// REQ-023: --approval-mode plan is on every call, exactly once.
#[test]
fn test_should_always_pass_approval_mode_plan() {
    let d = dirs();
    for resume in [None, Some("abc")] {
        let sut = build_with(&d, "x", None, resume, &[], 5, Duration::from_secs(60)).unwrap();
        assert_eq!(count(&sut.args, "--approval-mode"), 1);
        assert_eq!(value_after(&sut.args, "--approval-mode"), Some("plan"));
    }
}

// REQ-023: --exclude-tools carries the full comma-joined list.
#[test]
fn test_should_pass_full_exclude_tools_list() {
    let d = dirs();
    let sut = build(&d, "x");
    let expected = EXPECTED_EXCLUDED.join(",");
    assert_eq!(count(&sut.args, "--exclude-tools"), 1);
    assert_eq!(
        value_after(&sut.args, "--exclude-tools"),
        Some(expected.as_str())
    );
}

// REQ-023: -m is the pinned model given by the request.
#[test]
fn test_should_pin_requested_qwen_model() {
    let d = dirs();
    let sut = build_with(
        &d,
        "x",
        Some("qwen3.7-plus"),
        None,
        &[],
        5,
        Duration::from_secs(60),
    )
    .unwrap();
    assert_eq!(value_after(&sut.args, "-m"), Some("qwen3.7-plus"));
}

// REQ-023: with no model in the request, -m is the default pinned model (never absent).
#[test]
fn test_should_pin_default_model_when_none_given() {
    let d = dirs();
    let sut = build_with(&d, "x", None, None, &[], 5, Duration::from_secs(60)).unwrap();
    assert_eq!(value_after(&sut.args, "-m"), Some(DEFAULT_QWEN_MODEL));
    assert_eq!(count(&sut.args, "-m"), 1);
}

// REQ-023: --max-session-turns carries max_turns.
#[test]
fn test_should_pass_max_session_turns() {
    let d = dirs();
    let a = build_with(&d, "x", None, None, &[], 7, Duration::from_secs(60)).unwrap();
    let b = build_with(&d, "x", None, None, &[], 31, Duration::from_secs(60)).unwrap();
    assert_eq!(value_after(&a.args, "--max-session-turns"), Some("7"));
    assert_eq!(value_after(&b.args, "--max-session-turns"), Some("31"));
}

// REQ-023/REQ-051: --max-wall-time is present, non-empty, and follows the request's time limit.
#[test]
fn test_should_pass_max_wall_time_that_tracks_the_limit() {
    let d = dirs();
    let a = build_with(&d, "x", None, None, &[], 5, Duration::from_secs(90)).unwrap();
    let b = build_with(&d, "x", None, None, &[], 5, Duration::from_secs(1800)).unwrap();
    let va = value_after(&a.args, "--max-wall-time").expect("flag present");
    let vb = value_after(&b.args, "--max-wall-time").expect("flag present");
    assert!(!va.is_empty() && !vb.is_empty());
    assert_ne!(va, vb);
}

// REQ-023: fresh session has no --resume.
#[test]
fn test_should_omit_resume_for_fresh_session() {
    let d = dirs();
    assert_eq!(count(&build(&d, "x").args, "--resume"), 0);
}

// REQ-023: resume passes --resume <id>.
#[test]
fn test_should_pass_resume_with_session_id() {
    let d = dirs();
    let sut = build_with(
        &d,
        "x",
        None,
        Some("7f3c9a2e-5b1d-4e8a-9c40-2d6f1e8b7a01"),
        &[],
        5,
        Duration::from_secs(60),
    )
    .unwrap();
    assert_eq!(count(&sut.args, "--resume"), 1);
    assert_eq!(
        value_after(&sut.args, "--resume"),
        Some("7f3c9a2e-5b1d-4e8a-9c40-2d6f1e8b7a01")
    );
}

// REQ-023: the builder never emits --continue, --yolo, -y or a writer approval mode, for any request shape.
#[test]
fn test_should_never_emit_writer_or_continue_flags() {
    let d = dirs();
    for resume in [None, Some("id-1")] {
        for model in [None, Some("qwen3-coder-plus")] {
            let sut = build_with(
                &d,
                "review",
                model,
                resume,
                &[],
                9,
                Duration::from_secs(120),
            )
            .unwrap();
            for banned in [
                "--continue",
                "-c",
                "--yolo",
                "-y",
                "--allowed-tools",
                "--auto-approve",
            ] {
                assert_eq!(
                    count(&sut.args, banned),
                    0,
                    "{banned} present with resume={resume:?}"
                );
            }
            assert!(
                !sut.args
                    .iter()
                    .any(|a| a == "yolo" || a == "auto-edit" || a == "default")
            );
        }
    }
}

// REQ-023: the exclude list never grows a read tool (the seat must still read).
#[test]
fn test_should_not_exclude_read_tools() {
    for read_tool in ["read_file", "grep_search", "glob", "list_directory"] {
        assert!(
            !EXCLUDED_TOOLS.contains(&read_tool),
            "{read_tool} must stay usable"
        );
    }
}

// REQ-020: cwd is the request's cwd.
#[test]
fn test_should_run_in_request_cwd() {
    let d = dirs();
    let sut = build(&d, "x");
    assert_eq!(sut.cwd, d.cwd.path().to_path_buf());
}

// REQ-020: same request gives the same argv.
#[test]
fn test_should_be_deterministic() {
    let d = dirs();
    let a = build(&d, "same");
    let b = build(&d, "same");
    assert_eq!(a.args, b.args);
    assert_eq!(a.program, b.program);
}

// REQ-020: a different prompt changes only the prompt value.
#[test]
fn test_should_change_only_prompt_value_for_different_prompt() {
    let d = dirs();
    let a = build(&d, "alpha");
    let b = build(&d, "bravo");
    assert_eq!(a.args.len(), b.args.len());
    let diff: Vec<_> = a
        .args
        .iter()
        .zip(b.args.iter())
        .filter(|(x, y)| x != y)
        .collect();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].0, "alpha");
    assert_eq!(diff[0].1, "bravo");
}

// ---------------------------------------------------------------- env (research/03)

// research/03: env_add never sets the unattended retry var.
#[test]
fn test_should_never_add_unattended_retry_env() {
    let d = dirs();
    let sut = build(&d, "x");
    assert!(
        !sut.env_add
            .iter()
            .any(|(k, _)| k == "QWEN_CODE_UNATTENDED_RETRY")
    );
}

// research/03: env_remove strips the unattended retry var from the inherited env.
#[test]
fn test_should_strip_unattended_retry_env_from_inherited_env() {
    let d = dirs();
    for resume in [None, Some("id")] {
        let sut = build_with(&d, "x", None, resume, &[], 5, Duration::from_secs(60)).unwrap();
        assert!(
            sut.env_remove
                .iter()
                .any(|k| k == "QWEN_CODE_UNATTENDED_RETRY"),
            "env_remove was {:?}",
            sut.env_remove
        );
    }
}

// research/03: the retry var is also never smuggled in through args.
#[test]
fn test_should_not_mention_unattended_retry_in_args() {
    let d = dirs();
    let sut = build(&d, "x");
    assert!(!sut.args.iter().any(|a| a.contains("UNATTENDED_RETRY")));
}

// ---------------------------------------------------------------- prompt limit (REQ-024)

// REQ-024: a 65 KiB prompt is refused with the code first.
#[test]
fn test_should_refuse_65_kib_prompt() {
    let d = dirs();
    let prompt = "a".repeat(65 * 1024);
    let err = build_with(&d, &prompt, None, None, &[], 5, Duration::from_secs(60))
        .expect_err("must refuse");
    assert!(err.starts_with("prompt_too_large_for_qwen"), "got {err}");
}

// REQ-024: exactly 64 KiB is accepted and is passed whole, never truncated.
#[test]
fn test_should_accept_exactly_64_kib_without_truncation() {
    let d = dirs();
    let prompt = "b".repeat(MAX_PROMPT_BYTES);
    let sut = build_with(&d, &prompt, None, None, &[], 5, Duration::from_secs(60))
        .expect("boundary accepted");
    assert_eq!(
        value_after(&sut.args, "-p").map(|p| p.len()),
        Some(MAX_PROMPT_BYTES)
    );
}

// REQ-024: one byte over the limit is refused.
#[test]
fn test_should_refuse_one_byte_over_the_limit() {
    let d = dirs();
    let prompt = "c".repeat(MAX_PROMPT_BYTES + 1);
    let err = build_with(&d, &prompt, None, None, &[], 5, Duration::from_secs(60))
        .expect_err("must refuse");
    assert!(err.starts_with("prompt_too_large_for_qwen"), "got {err}");
}

// REQ-024: the limit counts bytes, not characters (32769 two-byte chars is 65538 bytes).
#[test]
fn test_should_count_bytes_not_chars_for_prompt_limit() {
    let d = dirs();
    let prompt = "\u{e9}".repeat(MAX_PROMPT_BYTES / 2 + 1);
    assert!(prompt.chars().count() <= MAX_PROMPT_BYTES);
    assert!(prompt.len() > MAX_PROMPT_BYTES);
    let err = build_with(&d, &prompt, None, None, &[], 5, Duration::from_secs(60))
        .expect_err("must refuse");
    assert!(err.starts_with("prompt_too_large_for_qwen"), "got {err}");
}

// REQ-024: a multi-byte prompt exactly at the limit is accepted.
#[test]
fn test_should_accept_multibyte_prompt_at_exact_limit() {
    let d = dirs();
    let prompt = "\u{e9}".repeat(MAX_PROMPT_BYTES / 2);
    assert_eq!(prompt.len(), MAX_PROMPT_BYTES);
    assert!(build_with(&d, &prompt, None, None, &[], 5, Duration::from_secs(60)).is_ok());
}

// REQ-024: an empty prompt does not trip the size check with the too-large code.
#[test]
fn test_should_not_report_too_large_for_empty_prompt() {
    let d = dirs();
    if let Err(e) = build_with(&d, "", None, None, &[], 5, Duration::from_secs(60)) {
        assert!(!e.starts_with("prompt_too_large_for_qwen"), "got {e}");
    }
}

// ---------------------------------------------------------------- forbidden extra args (REQ-020)

// REQ-020: every forbidden operator arg is rejected.
#[test]
fn test_should_reject_each_forbidden_extra_arg() {
    let d = dirs();
    for flag in FORBIDDEN_EXTRA_ARGS {
        let extra = vec![flag.to_string()];
        let r = build_with(&d, "x", None, None, &extra, 5, Duration::from_secs(60));
        assert!(r.is_err(), "{flag} must be rejected");
    }
}

// REQ-020: a forbidden arg is rejected even when it carries a value after it.
#[test]
fn test_should_reject_forbidden_arg_with_value() {
    let d = dirs();
    let cases: [&[&str]; 4] = [
        &["--approval-mode", "yolo"],
        &["--model", "glm-5"],
        &["--exclude-tools", "none"],
        &["--allowed-tools", "run_shell_command"],
    ];
    for case in cases {
        let extra: Vec<String> = case.iter().map(|s| s.to_string()).collect();
        assert!(
            build_with(&d, "x", None, None, &extra, 5, Duration::from_secs(60)).is_err(),
            "{case:?}"
        );
    }
}

// REQ-020: a forbidden arg hidden after a harmless one is still rejected.
#[test]
fn test_should_reject_forbidden_arg_after_other_args() {
    let d = dirs();
    let extra = vec!["--debug".to_string(), "--yolo".to_string()];
    assert!(build_with(&d, "x", None, None, &extra, 5, Duration::from_secs(60)).is_err());
}

// ---------------------------------------------------------------- plan_auth (D-07, TECH_STACK)

const QWENCLOUD_ENDPOINT: &str = "https://token-plan.maas.qwencloudapi.com/compatible-mode/v1";

// TECH_STACK: the default plan is qwencloud.
#[test]
fn test_should_default_plan_to_qwencloud() {
    assert_eq!(
        plan_auth(None),
        Ok(("QWENCLOUD_TOKEN_PLAN_API_KEY", QWENCLOUD_ENDPOINT))
    );
}

// TECH_STACK: an explicit qwencloud plan equals the default.
#[test]
fn test_should_map_explicit_qwencloud_plan() {
    assert_eq!(
        plan_auth(Some("qwencloud")),
        Ok(("QWENCLOUD_TOKEN_PLAN_API_KEY", QWENCLOUD_ENDPOINT))
    );
    assert_eq!(plan_auth(Some("qwencloud")), plan_auth(None));
}

// TECH_STACK: modelstudio is refused, not mapped to its BAILIAN var.
#[test]
fn test_should_refuse_modelstudio_plan() {
    let r = plan_auth(Some("modelstudio"));
    assert!(r.is_err(), "modelstudio must be refused, got {r:?}");
}

// TECH_STACK: no plan value ever yields the ModelStudio env var or endpoint.
#[test]
fn test_should_not_leak_modelstudio_env_or_endpoint() {
    for plan in [None, Some("qwencloud"), Some("modelstudio")] {
        if let Ok((env, url)) = plan_auth(plan) {
            assert!(!env.contains("BAILIAN"), "{plan:?} -> {env}");
            assert!(!url.contains("ap-southeast-1"), "{plan:?} -> {url}");
        }
    }
}

// ---------------------------------------------------------------- served model (REQ-043)

// REQ-043: Qwen models pass.
#[test]
fn test_should_accept_qwen_models() {
    for m in [
        "qwen3-coder-plus",
        "qwen3.7-plus",
        "qwen",
        "qwen3-coder-plus-2026",
    ] {
        assert_eq!(check_served_model(m), Ok(()), "{m}");
    }
}

// REQ-043: glm-5 fails with the exact code and the model name.
#[test]
fn test_should_reject_glm_5_with_exact_message() {
    assert_eq!(
        check_served_model("glm-5"),
        Err("qwen_seat_served_non_qwen_model: glm-5".to_string())
    );
}

// REQ-043: the other third-party lab models on the plan fail with the model in the message.
#[test]
fn test_should_reject_other_lab_models() {
    for m in [
        "kimi-k2.5",
        "MiniMax-M2.5",
        "gpt-5",
        "deepseek-v4",
        "claude-opus",
    ] {
        assert_eq!(
            check_served_model(m),
            Err(format!("qwen_seat_served_non_qwen_model: {m}")),
            "{m}"
        );
    }
}

// REQ-043: the check is a prefix check, not a contains check.
#[test]
fn test_should_require_qwen_prefix_not_substring() {
    for m in [
        "not-qwen3",
        "xqwen3-coder-plus",
        "glm-qwen",
        " qwen3-coder-plus",
    ] {
        assert!(check_served_model(m).is_err(), "{m} must fail");
    }
}

// REQ-043: an empty served model fails loud.
#[test]
fn test_should_reject_empty_served_model() {
    let e = check_served_model("").expect_err("empty must fail");
    assert!(e.starts_with("qwen_seat_served_non_qwen_model"), "got {e}");
}

// REQ-043 end to end: a glm-5 stream's served model fails the check; a qwen stream's passes.
#[test]
fn test_should_fail_check_for_glm_5_stream_and_pass_for_qwen_stream() {
    let glm = [
        r#"{"type":"system","subtype":"session_start","session_id":"s1","model":"glm-5"}"#,
        r#"{"type":"assistant","session_id":"s1","message":{"role":"assistant","model":"glm-5","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        r#"{"type":"result","subtype":"success","session_id":"s1","is_error":false,"result":"hi","usage":{"input_tokens":1,"output_tokens":1}}"#,
    ];
    let qwen: Vec<String> = glm
        .iter()
        .map(|l| l.replace("glm-5", "qwen3-coder-plus"))
        .collect();

    let served = |lines: &[&str]| {
        let mut p = QwenStreamParser::new();
        for l in lines {
            let _ = p.parse_line(l);
        }
        p.finish(Some(0))
            .expect("finishes")
            .served_model
            .expect("served model present")
    };
    let glm_model = served(&glm);
    assert_eq!(
        check_served_model(&glm_model),
        Err("qwen_seat_served_non_qwen_model: glm-5".to_string())
    );

    let qwen_refs: Vec<&str> = qwen.iter().map(|s| s.as_str()).collect();
    let qwen_model = served(&qwen_refs);
    assert_eq!(qwen_model, "qwen3-coder-plus");
    assert_eq!(check_served_model(&qwen_model), Ok(()));
}
