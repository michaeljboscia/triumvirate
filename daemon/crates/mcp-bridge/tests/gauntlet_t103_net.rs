//! GAUNTLET net suite for T-103, written blind from the spec (no implementation read).
//!
//! SPEC_AMBIGUITY: SeatInvocation.program is not pinned to a literal. Asserted only as a path whose file name is `qwen`.
//! SPEC_AMBIGUITY: the --exclude-tools shape (repeated flag or one comma list) is not pinned. Asserted as: every name
//!   appears as a whole token or as a comma-separated part of a token after the flag.
//! SPEC_AMBIGUITY: the --max-wall-time unit is not pinned. Asserted as the integer seconds of time_limit.
//! SPEC_AMBIGUITY: the error text for a forbidden extra arg and for the refused modelstudio plan is not in the spec. Asserted as Err only.
//! SPEC_AMBIGUITY: the plan endpoint URL is not in the package. Asserted as an https URL. The env var name comes from
//!   AMENDMENTS-2026-10-18.md A-9 (QWENCLOUD_TOKEN_PLAN_API_KEY).
//! SPEC_AMBIGUITY: child_env() and SeatProfile (seat_profile module) are not in the listed API, so the composed child env
//!   check (the child env never carries QWEN_CODE_UNATTENDED_RETRY even when the daemon env sets it) cannot be written.
//!   Only SeatInvocation.env_remove is asserted here.
//! SPEC_AMBIGUITY: the REQ-063 sight gate (required_sources) is not in the listed API. Not tested here.
//! SPEC_AMBIGUITY: plan_auth behavior for a plan value other than qwencloud, modelstudio or None is not in the spec. Not tested.

use std::time::Duration;

use mcp_bridge::containment::{SeatInvocation, SeatRequest};
use mcp_bridge::qwen::{
    build_qwen_invocation, check_served_model, plan_auth, DEFAULT_QWEN_MODEL, EXCLUDED_TOOLS,
    FORBIDDEN_EXTRA_ARGS, MAX_PROMPT_BYTES, UNATTENDED_RETRY_ENV,
};

fn build(
    prompt: &str,
    model: Option<&str>,
    resume: Option<&str>,
    extra: &[String],
    max_turns: u32,
    secs: u64,
) -> Result<SeatInvocation, String> {
    let cwd = tempfile::tempdir().expect("tempdir cwd");
    let home = tempfile::tempdir().expect("tempdir seat home");
    let out = tempfile::tempdir().expect("tempdir output");
    let req = SeatRequest {
        prompt,
        cwd: cwd.path(),
        seat_home: home.path(),
        output_dir: out.path(),
        model,
        resume_session_id: resume,
        extra_args: extra,
        max_turns,
        time_limit: Duration::from_secs(secs),
    };
    build_qwen_invocation(&req)
}

fn ok(prompt: &str) -> SeatInvocation {
    build(prompt, None, None, &[], 5, 120).expect("qwen builder accepts a normal prompt")
}

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(|s| s.as_str())
}

fn has_token(args: &[String], token: &str) -> bool {
    args.iter().any(|a| a == token)
}

// REQ-023: the pinned constants carry the spec values.
#[test]
fn t103_req023_constants_carry_spec_values() {
    assert_eq!(DEFAULT_QWEN_MODEL, "qwen3-coder-plus");
    assert_eq!(MAX_PROMPT_BYTES, 65536);
    assert_eq!(UNATTENDED_RETRY_ENV, "QWEN_CODE_UNATTENDED_RETRY");
    assert_eq!(EXCLUDED_TOOLS.len(), 15);
}

// REQ-023: --safe-mode and --approval-mode plan are on every call, fresh or resumed.
#[test]
fn t103_req023_read_only_flags_on_every_call() {
    let fresh = ok("look at the repo");
    assert!(has_token(&fresh.args, "--safe-mode"));
    assert_eq!(value_after(&fresh.args, "--approval-mode"), Some("plan"));

    let resumed = build("again", None, Some("sess-123"), &[], 5, 120).expect("resume builds");
    assert!(has_token(&resumed.args, "--safe-mode"));
    assert_eq!(value_after(&resumed.args, "--approval-mode"), Some("plan"));
}

// REQ-023: the argv names stream-json output and carries the prompt after -p.
#[test]
fn t103_req023_stream_json_output_and_prompt_after_p() {
    let inv = ok("summarize main.rs");
    assert_eq!(value_after(&inv.args, "--output-format"), Some("stream-json"));
    assert_eq!(value_after(&inv.args, "-p"), Some("summarize main.rs"));
}

// REQ-023: with no model requested, the builder pins -m to the default Qwen model.
#[test]
fn t103_req023_unrequested_model_pins_default() {
    let inv = ok("hi");
    assert_eq!(value_after(&inv.args, "-m"), Some("qwen3-coder-plus"));
}

// REQ-023: a requested model is passed as -m.
#[test]
fn t103_req023_requested_model_passed_through() {
    let inv = build("hi", Some("qwen3-max"), None, &[], 5, 120).expect("builds");
    assert_eq!(value_after(&inv.args, "-m"), Some("qwen3-max"));
}

// REQ-023: every name of the full exclude list is present after --exclude-tools.
#[test]
fn t103_req023_exclude_tools_full_list_present() {
    let inv = ok("hi");
    let i = inv
        .args
        .iter()
        .position(|a| a == "--exclude-tools")
        .expect("--exclude-tools present");
    let tail = &inv.args[i + 1..];
    for tool in EXCLUDED_TOOLS.iter().copied() {
        let found = tail
            .iter()
            .any(|a| a.as_str() == tool || a.split(',').any(|p| p == tool));
        assert!(found, "excluded tool missing from argv: {tool}");
    }
}

// REQ-023: --max-session-turns carries max_turns and --max-wall-time carries the time limit in seconds.
#[test]
fn t103_req023_turn_cap_and_wall_time_values() {
    let inv = build("hi", None, None, &[], 7, 90).expect("builds");
    assert_eq!(value_after(&inv.args, "--max-session-turns"), Some("7"));
    assert_eq!(value_after(&inv.args, "--max-wall-time"), Some("90"));
}

// REQ-023: --resume appears only when a session id is given, and carries that id.
#[test]
fn t103_req023_resume_flag_only_when_resuming() {
    let fresh = ok("hi");
    assert!(!has_token(&fresh.args, "--resume"));

    let resumed = build("hi", None, Some("sess-123"), &[], 5, 120).expect("builds");
    assert_eq!(value_after(&resumed.args, "--resume"), Some("sess-123"));
}

// REQ-023: the argv never contains a writer or unattended flag on any call.
#[test]
fn t103_req023_never_emits_yolo_or_continue() {
    let fresh = ok("hi");
    let resumed = build("hi", None, Some("sess-1"), &[], 5, 120).expect("builds");
    for inv in [&fresh, &resumed] {
        assert!(!has_token(&inv.args, "--yolo"));
        assert!(!has_token(&inv.args, "-y"));
        assert!(!has_token(&inv.args, "--continue"));
    }
}

// REQ-023: the builder strips QWEN_CODE_UNATTENDED_RETRY from the inherited env and never adds it.
#[test]
fn t103_req023_unattended_retry_stripped_never_added() {
    let inv = ok("hi");
    assert!(inv.env_remove.iter().any(|k| k == UNATTENDED_RETRY_ENV));
    assert!(!inv.env_add.iter().any(|(k, _)| k == UNATTENDED_RETRY_ENV));
}

// REQ-024: a prompt of exactly MAX_PROMPT_BYTES bytes is accepted.
#[test]
fn t103_req024_prompt_at_limit_accepted() {
    let prompt = "a".repeat(MAX_PROMPT_BYTES);
    assert!(build(&prompt, None, None, &[], 5, 120).is_ok());
}

// REQ-024: one byte over MAX_PROMPT_BYTES is refused with the code first.
#[test]
fn t103_req024_prompt_one_byte_over_refused() {
    let prompt = "a".repeat(MAX_PROMPT_BYTES + 1);
    let err = build(&prompt, None, None, &[], 5, 120).err().expect("refused");
    assert!(
        err.starts_with("prompt_too_large_for_qwen"),
        "wrong code: {err}"
    );
}

// REQ-024: a 65 KiB prompt is refused with prompt_too_large_for_qwen.
#[test]
fn t103_req024_65_kib_prompt_refused() {
    let prompt = "b".repeat(65 * 1024);
    let err = build(&prompt, None, None, &[], 5, 120).err().expect("refused");
    assert!(
        err.starts_with("prompt_too_large_for_qwen"),
        "wrong code: {err}"
    );
}

// REQ-024: the limit counts bytes, so 2-byte characters reach the limit at half the character count.
#[test]
fn t103_req024_limit_counts_bytes_not_chars() {
    let at_limit = "\u{e9}".repeat(MAX_PROMPT_BYTES / 2);
    assert_eq!(at_limit.len(), MAX_PROMPT_BYTES);
    assert!(build(&at_limit, None, None, &[], 5, 120).is_ok());

    let over = format!("{at_limit}x");
    let err = build(&over, None, None, &[], 5, 120).err().expect("refused");
    assert!(err.starts_with("prompt_too_large_for_qwen"), "wrong code: {err}");
}

// REQ-023 (scope): every forbidden extra arg is rejected by the builder.
#[test]
fn t103_req023_every_forbidden_extra_arg_rejected() {
    for arg in FORBIDDEN_EXTRA_ARGS.iter().copied() {
        let extra = vec![arg.to_string()];
        let res = build("hi", None, None, &extra, 5, 120);
        assert!(res.is_err(), "forbidden extra arg accepted: {arg}");
    }
}

// D-07: the default plan (None) is qwencloud, and the key var is QWENCLOUD_TOKEN_PLAN_API_KEY (A-9).
#[test]
fn t103_d07_default_plan_is_qwencloud() {
    let default = plan_auth(None).expect("default plan resolves");
    let named = plan_auth(Some("qwencloud")).expect("qwencloud resolves");
    assert_eq!(default, named);
    assert_eq!(default.0, "QWENCLOUD_TOKEN_PLAN_API_KEY");
    assert!(default.1.starts_with("https://"), "endpoint not https: {}", default.1);
}

// TECH_STACK / contract change: modelstudio is refused, never mapped to an env var.
#[test]
fn t103_plan_modelstudio_refused() {
    assert!(plan_auth(Some("modelstudio")).is_err());
}

// REQ-043: a Qwen model passes the served-model check.
#[test]
fn t103_req043_qwen_served_model_passes() {
    assert!(check_served_model("qwen3-coder-plus").is_ok());
}

// REQ-043: a fixture answered by glm-5 returns the exact code and model name.
#[test]
fn t103_req043_glm5_served_model_fails_exact_code() {
    let err = check_served_model("glm-5").err().expect("non-qwen refused");
    assert_eq!(err, "qwen_seat_served_non_qwen_model: glm-5");
}

// REQ-043: another lab's model (kimi-k2) is refused with the same code.
#[test]
fn t103_req043_other_lab_model_refused_exact_code() {
    let err = check_served_model("kimi-k2").err().expect("non-qwen refused");
    assert_eq!(err, "qwen_seat_served_non_qwen_model: kimi-k2");
}
