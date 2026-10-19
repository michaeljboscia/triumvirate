// GAUNTLET tiebreaker suite for T-102, written blind from the spec (no implementation read).

use std::path::Path;
use std::time::Duration;

use mcp_bridge::containment::{SEAT_CREDENTIAL_SCRUB, SeatRequest};
use mcp_bridge::mistral::{
    ENABLED_TOOLS, FORBIDDEN_EXTRA_ARGS, MAX_PROMPT_BYTES, MISTRAL_PROGRAM,
    build_mistral_invocation,
};

fn request<'a>(prompt: &'a str, extra_args: &'a [String]) -> SeatRequest<'a> {
    SeatRequest {
        prompt,
        cwd: Path::new("/repo-under-review"),
        seat_home: Path::new("/isolated-seat-home"),
        output_dir: Path::new("/per-call-output"),
        model: Some("configured-in-seat-home-not-argv"),
        resume_session_id: None,
        extra_args,
        max_turns: 12,
        time_limit: Duration::from_secs(600),
    }
}

#[test]
// REQ-020, REQ-022: a fresh invocation exactly implements the read-only Mistral argv contract.
fn fresh_invocation_has_exact_program_argv_and_environment() {
    let invocation = build_mistral_invocation(&request("inspect this repository", &[])).unwrap();

    assert_eq!(invocation.program, MISTRAL_PROGRAM);
    assert_eq!(
        invocation.args,
        vec![
            "-p",
            "inspect this repository",
            "--output",
            "streaming",
            "--workdir",
            "/repo-under-review",
            "--output-dir",
            "/per-call-output",
            "--max-turns",
            "12",
            "--time-limit",
            "600",
            "--agent",
            "plan",
            "--enabled-tools",
            "read_file",
            "--enabled-tools",
            "grep",
            "--enabled-tools",
            "todo",
        ]
    );
    assert_eq!(
        invocation.env_add,
        vec![("VIBE_CLI".to_owned(), "python".to_owned())]
    );
    assert_eq!(
        invocation.env_remove,
        SEAT_CREDENTIAL_SCRUB
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(invocation.cwd, Path::new("/repo-under-review"));
}

#[test]
// REQ-022: resume uses the named vendor session and retains Python CLI selection.
fn resume_invocation_uses_only_the_requested_session_id() {
    let mut req = request("continue the review", &[]);
    req.resume_session_id = Some("vibe-session-specific");
    let invocation = build_mistral_invocation(&req).unwrap();

    assert_eq!(
        &invocation.args[invocation.args.len() - 2..],
        ["--resume", "vibe-session-specific"]
    );
    assert!(!invocation.args.iter().any(|arg| arg == "--continue"));
    assert_eq!(
        invocation.env_add,
        vec![("VIBE_CLI".to_owned(), "python".to_owned())]
    );
}

#[test]
// REQ-022: the allowlist is exactly the three source-verified read-only tools and contains no writer.
fn enabled_tools_are_exactly_the_read_only_allowlist() {
    assert_eq!(ENABLED_TOOLS, ["read_file", "grep", "todo"]);
    let invocation = build_mistral_invocation(&request("review", &[])).unwrap();
    let enabled = invocation
        .args
        .windows(2)
        .filter(|pair| pair[0] == "--enabled-tools")
        .map(|pair| pair[1].as_str())
        .collect::<Vec<_>>();

    assert_eq!(enabled, ENABLED_TOOLS);
    for writer in ["bash", "edit", "write_file", "task", "auto-approve"] {
        assert!(!invocation.args.iter().any(|arg| arg == writer));
    }
}

#[test]
// REQ-022: every reserved operator flag is rejected in bare and equals forms.
fn every_forbidden_extra_arg_form_is_rejected() {
    assert_eq!(FORBIDDEN_EXTRA_ARGS.len(), 16);
    for forbidden in FORBIDDEN_EXTRA_ARGS {
        for supplied in [forbidden.to_owned(), format!("{forbidden}=operator-value")] {
            let extras = [supplied.clone()];
            let error = build_mistral_invocation(&request("review", &extras)).unwrap_err();
            assert!(
                error.contains(forbidden),
                "error {error:?} must identify rejected flag {forbidden:?} supplied as {supplied:?}"
            );
        }
    }

    for supplied in ["-pattached", "-cattached"] {
        let extras = [supplied.to_owned()];
        let error = build_mistral_invocation(&request("review", &extras)).unwrap_err();
        assert!(error.contains(&supplied[..2]), "unexpected error: {error}");
    }
}

#[test]
// REQ-024: the maximum prompt size is measured in bytes and the exact limit is accepted.
fn prompt_at_exact_byte_limit_is_preserved() {
    let prompt = "x".repeat(MAX_PROMPT_BYTES);
    let invocation = build_mistral_invocation(&request(&prompt, &[])).unwrap();
    assert_eq!(invocation.args[1], prompt);
    assert_eq!(invocation.args[1].len(), 65_536);
}

#[test]
// REQ-024: one byte above the prompt limit fails with the specified code.
fn prompt_one_byte_over_limit_is_rejected() {
    let prompt = "x".repeat(MAX_PROMPT_BYTES + 1);
    let error = build_mistral_invocation(&request(&prompt, &[])).unwrap_err();
    assert!(error.starts_with("prompt_too_large_for_mistral"), "{error}");
}

#[test]
// REQ-024: Unicode prompt enforcement uses UTF-8 bytes rather than character count.
fn unicode_prompt_limit_is_enforced_by_bytes() {
    let prompt = "é".repeat((MAX_PROMPT_BYTES / 2) + 1);
    assert!(prompt.chars().count() < MAX_PROMPT_BYTES);
    assert_eq!(prompt.len(), MAX_PROMPT_BYTES + 2);
    let error = build_mistral_invocation(&request(&prompt, &[])).unwrap_err();
    assert!(error.starts_with("prompt_too_large_for_mistral"), "{error}");
}

#[test]
// REQ-022: safe operator arguments pass through without changing the protected contract.
fn safe_extra_args_are_appended_verbatim() {
    let extras = ["--max-price".to_owned(), "0.25".to_owned()];
    let invocation = build_mistral_invocation(&request("review", &extras)).unwrap();
    assert_eq!(&invocation.args[invocation.args.len() - 2..], extras);
}
