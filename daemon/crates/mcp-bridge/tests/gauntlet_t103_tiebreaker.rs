// GAUNTLET tiebreaker suite for T-103, written blind from the spec (no implementation read).
// SPEC_AMBIGUITY: The exact error text for each forbidden extra argument is not specified beyond being code-first.
// SPEC_AMBIGUITY: The exact modelstudio refusal text from plan_auth is not specified.

use std::path::Path;
use std::time::Duration;

use mcp_bridge::containment::SeatRequest;
use mcp_bridge::qwen::{
    DEFAULT_QWEN_MODEL, EXCLUDED_TOOLS, FORBIDDEN_EXTRA_ARGS, MAX_PROMPT_BYTES,
    UNATTENDED_RETRY_ENV, build_qwen_invocation, check_served_model, plan_auth,
};

fn request<'a>(
    prompt: &'a str,
    model: Option<&'a str>,
    resume_session_id: Option<&'a str>,
    extra_args: &'a [String],
) -> SeatRequest<'a> {
    SeatRequest {
        prompt,
        cwd: Path::new("/repo"),
        seat_home: Path::new("/seat/qwen"),
        output_dir: Path::new("/output"),
        model,
        resume_session_id,
        extra_args,
        max_turns: 7,
        time_limit: Duration::from_secs(91),
    }
}

fn value_after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let index = args.iter().position(|arg| arg == flag).unwrap();
    args.get(index + 1).unwrap()
}

fn occurrences(args: &[String], needle: &str) -> usize {
    args.iter().filter(|arg| arg.as_str() == needle).count()
}

#[test]
fn req_023_builder_emits_the_complete_read_only_contract() {
    // REQ-023: Every invocation carries safe mode, plan approval, the complete denylist, budgets, and a pinned model.
    let invocation = build_qwen_invocation(&request("inspect", None, None, &[])).unwrap();

    assert_eq!(invocation.program, "qwen");
    assert_eq!(invocation.cwd, Path::new("/repo"));
    assert_eq!(value_after(&invocation.args, "-p"), "inspect");
    assert_eq!(
        value_after(&invocation.args, "--output-format"),
        "stream-json"
    );
    assert_eq!(occurrences(&invocation.args, "--safe-mode"), 1);
    assert_eq!(value_after(&invocation.args, "-m"), DEFAULT_QWEN_MODEL);
    assert_eq!(value_after(&invocation.args, "--max-session-turns"), "7");
    assert_eq!(value_after(&invocation.args, "--max-wall-time"), "91");
    assert_eq!(value_after(&invocation.args, "--approval-mode"), "plan");
    assert_eq!(occurrences(&invocation.args, "--exclude-tools"), 1);
    assert_eq!(
        value_after(&invocation.args, "--exclude-tools"),
        EXCLUDED_TOOLS.join(",")
    );
    assert!(!invocation.args.iter().any(|arg| arg == "--yolo"));
    assert!(!invocation.args.iter().any(|arg| arg == "--continue"));
}

#[test]
fn req_023_builder_uses_explicit_model_and_resume_id() {
    // REQ-023: Explicit model pinning and explicit UUID resume are preserved exactly.
    let invocation = build_qwen_invocation(&request(
        "inspect",
        Some("qwen3.7-plus"),
        Some("7f3c9a2e-5b1d-4e8a-9c40-2d6f1e8b7a01"),
        &[],
    ))
    .unwrap();

    assert_eq!(value_after(&invocation.args, "-m"), "qwen3.7-plus");
    assert_eq!(
        value_after(&invocation.args, "--resume"),
        "7f3c9a2e-5b1d-4e8a-9c40-2d6f1e8b7a01"
    );
    assert_eq!(occurrences(&invocation.args, "--resume"), 1);
}

#[test]
fn req_023_builder_strips_unattended_retry_and_scrubs_credentials() {
    // REQ-023 and REQ-033: Retry-forever and every shared credential pattern are removed from the child environment.
    let invocation = build_qwen_invocation(&request("inspect", None, None, &[])).unwrap();

    assert!(
        invocation
            .env_remove
            .iter()
            .any(|name| name == UNATTENDED_RETRY_ENV)
    );
    for name in [
        "MISTRAL_API_KEY",
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "DASHSCOPE_*",
        "BAILIAN_*",
        "QWENCLOUD_*",
        "KIMI_*",
        "MOONSHOT_*",
    ] {
        assert!(
            invocation.env_remove.iter().any(|entry| entry == name),
            "missing {name}"
        );
    }
    assert!(
        !invocation
            .env_add
            .iter()
            .any(|(name, _)| name == UNATTENDED_RETRY_ENV)
    );
}

#[test]
fn req_020_every_forbidden_operator_argument_is_rejected() {
    // REQ-020: Operator arguments cannot override builder-owned or read-only controls.
    for forbidden in FORBIDDEN_EXTRA_ARGS {
        let extras = vec![(*forbidden).to_owned()];
        let error = build_qwen_invocation(&request("inspect", None, None, &extras)).unwrap_err();
        assert!(
            !error.is_empty(),
            "{forbidden} must return a code-first error"
        );
    }
}

#[test]
fn req_020_unowned_extra_arguments_are_appended_exactly() {
    // REQ-020: An operator argument outside the forbidden set is retained without changing owned controls.
    let extras = vec!["--include-partial-messages".to_owned()];
    let invocation = build_qwen_invocation(&request("inspect", None, None, &extras)).unwrap();

    assert_eq!(
        occurrences(&invocation.args, "--include-partial-messages"),
        1
    );
    assert_eq!(value_after(&invocation.args, "--approval-mode"), "plan");
}

#[test]
fn req_024_prompt_limit_accepts_exact_boundary() {
    // REQ-024: A prompt exactly MAX_PROMPT_BYTES bytes is accepted without truncation.
    let prompt = "x".repeat(MAX_PROMPT_BYTES);
    let invocation = build_qwen_invocation(&request(&prompt, None, None, &[])).unwrap();

    assert_eq!(
        value_after(&invocation.args, "-p").as_bytes().len(),
        MAX_PROMPT_BYTES
    );
}

#[test]
fn req_024_prompt_limit_rejects_first_byte_over_boundary() {
    // REQ-024: The first byte above 64 KiB is rejected with the specified code.
    let prompt = "x".repeat(MAX_PROMPT_BYTES + 1);
    let error = build_qwen_invocation(&request(&prompt, None, None, &[])).unwrap_err();

    assert_eq!(error, "prompt_too_large_for_qwen");
}

#[test]
fn req_024_prompt_limit_is_measured_in_utf8_bytes() {
    // REQ-024: The limit is bytes rather than Unicode scalar values.
    let prompt = "é".repeat((MAX_PROMPT_BYTES / 2) + 1);
    let error = build_qwen_invocation(&request(&prompt, None, None, &[])).unwrap_err();

    assert_eq!(error, "prompt_too_large_for_qwen");
}

#[test]
fn req_043_served_model_accepts_qwen_and_rejects_other_lab() {
    // REQ-043: A Qwen model passes and a GLM model returns the exact cross-lab error.
    assert_eq!(check_served_model("qwen3-coder-plus"), Ok(()));
    assert_eq!(
        check_served_model("glm-5"),
        Err("qwen_seat_served_non_qwen_model: glm-5".to_owned())
    );
}

#[test]
fn d_07_default_plan_auth_is_qwencloud_international() {
    // D-07 and TECH_STACK Qwen table: None and qwencloud select the same env key and endpoint.
    let expected = (
        "QWENCLOUD_TOKEN_PLAN_API_KEY",
        "https://token-plan.maas.qwencloudapi.com/compatible-mode/v1",
    );

    assert_eq!(plan_auth(None), Ok(expected));
    assert_eq!(plan_auth(Some("qwencloud")), Ok(expected));
}

#[test]
fn d_07_modelstudio_and_unknown_plans_are_refused() {
    // D-07: ModelStudio is a scope-changing refusal, and an unknown plan is never silently mapped.
    assert!(plan_auth(Some("modelstudio")).is_err());
    assert!(plan_auth(Some("unknown-plan")).is_err());
}
