//! GAUNTLET net suite for T-102, written blind from the spec (no implementation read).
//!
//! Scope: the Mistral invocation builder (SPEC_v2 REQ-020, REQ-022, REQ-024) and the
//! forbidden extra-arg rule (REQ-020). Parser tests live in agent-adapter/tests/gauntlet_t102_net.rs.
//!
//! SPEC_AMBIGUITY: the unit of `--time-limit S` is not stated. The suite assumes whole seconds
//! (Duration::from_secs(600) renders as `600`).
//! SPEC_AMBIGUITY: the Mistral `env_remove` contents are not stated exactly (SEAT_CREDENTIAL_SCRUB is named
//! only through T-201). Not asserted.
//! SPEC_AMBIGUITY: the error code for a forbidden extra arg is not stated. Forbidden-arg tests assert Err only.

use std::path::PathBuf;
use std::time::Duration;

use mcp_bridge::containment::{SeatInvocation, SeatRequest};
use mcp_bridge::mistral::{
    ENABLED_TOOLS, FORBIDDEN_EXTRA_ARGS, MAX_PROMPT_BYTES, MISTRAL_PROGRAM,
    build_mistral_invocation,
};

struct Fx {
    cwd: tempfile::TempDir,
    home: tempfile::TempDir,
    out: tempfile::TempDir,
}

impl Fx {
    fn new() -> Self {
        Self {
            cwd: tempfile::tempdir().expect("cwd tempdir"),
            home: tempfile::tempdir().expect("home tempdir"),
            out: tempfile::tempdir().expect("out tempdir"),
        }
    }

    fn build(
        &self,
        prompt: &str,
        extra: &[String],
        model: Option<&str>,
        resume: Option<&str>,
    ) -> Result<SeatInvocation, String> {
        let req = SeatRequest {
            prompt,
            cwd: self.cwd.path(),
            seat_home: self.home.path(),
            output_dir: self.out.path(),
            model,
            resume_session_id: resume,
            extra_args: extra,
            max_turns: 7,
            time_limit: Duration::from_secs(600),
            mode: mcp_bridge::containment::SeatMode::ReadOnly,
        };
        build_mistral_invocation(&req)
    }

    fn cwd_str(&self) -> String {
        self.cwd.path().to_string_lossy().into_owned()
    }

    fn out_str(&self) -> String {
        self.out.path().to_string_lossy().into_owned()
    }
}

/// The value that follows `flag` in argv, or None when the flag is absent.
fn value_after(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

/// Every value that follows an occurrence of `flag`, in argv order.
fn all_values_after(args: &[String], flag: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, a)| a.as_str() == flag)
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect()
}

fn env_value(inv: &SeatInvocation, key: &str) -> Vec<String> {
    inv.env_add
        .iter()
        .filter(|(k, _)| k.as_str() == key)
        .map(|(_, v)| v.clone())
        .collect()
}

// REQ-022: the program is the vibe binary.
#[test]
fn req022_program_is_vibe() {
    assert_eq!(MISTRAL_PROGRAM, "vibe");
    let fx = Fx::new();
    let inv = fx.build("hello", &[], None, None).expect("build");
    assert_eq!(inv.program, "vibe");
}

// REQ-022: child env always carries VIBE_CLI=python, exactly once, whatever the call shape.
#[test]
fn req022_env_add_sets_vibe_cli_python_always() {
    let fx = Fx::new();
    let fresh = fx.build("hello", &[], None, None).expect("fresh");
    assert_eq!(env_value(&fresh, "VIBE_CLI"), vec!["python".to_string()]);

    let resumed = fx
        .build("hello", &[], Some("mistral-x"), Some("sess-abc"))
        .expect("resume");
    assert_eq!(env_value(&resumed, "VIBE_CLI"), vec!["python".to_string()]);
}

// REQ-022: the read-only allowlist is exactly three --enabled-tools flags, in order read_file, grep, todo.
#[test]
fn req022_three_enabled_tools_in_order() {
    assert_eq!(ENABLED_TOOLS, ["read_file", "grep", "todo"]);
    let fx = Fx::new();
    let inv = fx.build("hello", &[], None, None).expect("build");
    assert_eq!(
        all_values_after(&inv.args, "--enabled-tools"),
        vec![
            "read_file".to_string(),
            "grep".to_string(),
            "todo".to_string()
        ]
    );
}

// REQ-022: `--agent plan` is passed as the read-only label.
#[test]
fn req022_agent_label_is_plan() {
    let fx = Fx::new();
    let inv = fx.build("hello", &[], None, None).expect("build");
    assert_eq!(value_after(&inv.args, "--agent"), Some("plan".to_string()));
}

// REQ-022: argv carries the prompt, streaming output, workdir, per-call output dir, turn cap and time limit.
#[test]
fn req022_core_argv_values_exact() {
    let fx = Fx::new();
    let inv = fx.build("hello", &[], None, None).expect("build");
    assert_eq!(value_after(&inv.args, "-p"), Some("hello".to_string()));
    assert_eq!(
        value_after(&inv.args, "--output"),
        Some("streaming".to_string())
    );
    assert_eq!(value_after(&inv.args, "--workdir"), Some(fx.cwd_str()));
    assert_eq!(value_after(&inv.args, "--output-dir"), Some(fx.out_str()));
    assert_eq!(value_after(&inv.args, "--max-turns"), Some("7".to_string()));
    assert_eq!(
        value_after(&inv.args, "--time-limit"),
        Some("600".to_string())
    );
}

// REQ-022: the invocation cwd is the repo the seat reads.
#[test]
fn req022_invocation_cwd_is_repo() {
    let fx = Fx::new();
    let inv = fx.build("hello", &[], None, None).expect("build");
    assert_eq!(inv.cwd, PathBuf::from(fx.cwd_str()));
}

// REQ-022: no writer flag is ever built at launch.
#[test]
fn req022_no_writer_or_unsafe_flags_built() {
    let fx = Fx::new();
    let inv = fx
        .build("hello", &[], Some("mistral-x"), Some("sess-abc"))
        .expect("build");
    for banned in [
        "--trust",
        "--worktree",
        "--continue",
        "--auto-approve",
        "--yolo",
        "-c",
    ] {
        assert!(
            !inv.args.iter().any(|a| a == banned),
            "argv must not contain {banned}: {:?}",
            inv.args
        );
    }
}

// REQ-022: a fresh session carries no --resume.
#[test]
fn req022_fresh_session_has_no_resume_flag() {
    let fx = Fx::new();
    let inv = fx.build("hello", &[], None, None).expect("build");
    assert!(!inv.args.iter().any(|a| a == "--resume"), "{:?}", inv.args);
}

// REQ-022: a resume passes the vendor session id as `--resume <id>`.
#[test]
fn req022_resume_passes_session_id() {
    let fx = Fx::new();
    let inv = fx
        .build("hello", &[], None, Some("sess-abc"))
        .expect("build");
    assert_eq!(
        value_after(&inv.args, "--resume"),
        Some("sess-abc".to_string())
    );
}

// REQ-022: the model is not on the argv (the seat home config selects it).
#[test]
fn req022_model_never_on_argv() {
    let fx = Fx::new();
    let inv = fx
        .build("hello", &[], Some("mistral-large-2"), None)
        .expect("build");
    assert!(
        !inv.args.iter().any(|a| a.contains("mistral-large-2")),
        "{:?}",
        inv.args
    );
    assert!(
        !inv.args.iter().any(|a| a == "-m" || a == "--model"),
        "{:?}",
        inv.args
    );
}

// REQ-024: the constant is the 64 KiB limit.
#[test]
fn req024_limit_constant_is_64_kib() {
    assert_eq!(MAX_PROMPT_BYTES, 65536);
}

// REQ-024: a prompt of exactly the limit is accepted.
#[test]
fn req024_prompt_at_exact_limit_accepted() {
    let fx = Fx::new();
    let prompt = "a".repeat(65536);
    let inv = fx
        .build(&prompt, &[], None, None)
        .expect("65536 bytes must be accepted");
    assert_eq!(value_after(&inv.args, "-p").map(|p| p.len()), Some(65536));
}

// REQ-024: one byte over the limit is rejected with the code-first error, and nothing is spawned.
#[test]
fn req024_one_byte_over_rejected_with_code() {
    let fx = Fx::new();
    let prompt = "a".repeat(65537);
    let err = fx
        .build(&prompt, &[], None, None)
        .expect_err("65537 bytes must be rejected");
    assert!(
        err.starts_with("prompt_too_large_for_mistral"),
        "error must start with prompt_too_large_for_mistral, got: {err}"
    );
}

// REQ-024: a 65 KiB prompt is rejected, never truncated.
#[test]
fn req024_65_kib_prompt_rejected() {
    let fx = Fx::new();
    let prompt = "b".repeat(65 * 1024);
    let err = fx
        .build(&prompt, &[], None, None)
        .expect_err("65 KiB must be rejected");
    assert!(
        err.starts_with("prompt_too_large_for_mistral"),
        "got: {err}"
    );
}

// REQ-024: the limit is measured in bytes, not characters. 32769 two-byte characters are 65538 bytes.
#[test]
fn req024_limit_counts_bytes_not_chars() {
    let fx = Fx::new();
    let prompt = "\u{e9}".repeat(32769);
    assert_eq!(prompt.len(), 65538);
    let err = fx
        .build(&prompt, &[], None, None)
        .expect_err("65538 bytes must be rejected");
    assert!(
        err.starts_with("prompt_too_large_for_mistral"),
        "got: {err}"
    );
}

// REQ-020: the forbidden extra-arg list has the 16 entries the spec names and includes the writer flags.
#[test]
fn req020_forbidden_list_contents() {
    assert_eq!(FORBIDDEN_EXTRA_ARGS.len(), 16);
    for must in [
        "--continue",
        "-c",
        "--trust",
        "--worktree",
        "--auto-approve",
        "--yolo",
        "--agent",
        "--enabled-tools",
        "--disabled-tools",
        "--output",
        "--output-dir",
        "--workdir",
        "--resume",
        "-p",
        "--prompt",
        "--prompt-file",
    ] {
        assert!(
            FORBIDDEN_EXTRA_ARGS.contains(&must),
            "missing forbidden entry {must}"
        );
    }
}

// REQ-020: every forbidden long flag is refused bare and in its --flag=value form.
#[test]
fn req020_every_forbidden_long_flag_refused_bare_and_with_value() {
    let fx = Fx::new();
    for flag in FORBIDDEN_EXTRA_ARGS.iter().filter(|f| f.starts_with("--")) {
        let bare = vec![flag.to_string()];
        assert!(
            fx.build("hi", &bare, None, None).is_err(),
            "bare {flag} must be refused"
        );
        let eq = vec![format!("{flag}=x")];
        assert!(
            fx.build("hi", &eq, None, None).is_err(),
            "{flag}=x must be refused"
        );
    }
}

// REQ-020: a single-dash short flag is refused with its value attached (`-pfoo`, `-cfoo`).
#[test]
fn req020_short_forbidden_flags_refused_with_attached_value() {
    let fx = Fx::new();
    for flag in FORBIDDEN_EXTRA_ARGS
        .iter()
        .filter(|f| f.len() == 2 && f.starts_with('-') && !f.starts_with("--"))
    {
        let attached = vec![format!("{flag}foo")];
        assert!(
            fx.build("hi", &attached, None, None).is_err(),
            "{flag}foo must be refused"
        );
        let bare = vec![flag.to_string()];
        assert!(
            fx.build("hi", &bare, None, None).is_err(),
            "bare {flag} must be refused"
        );
    }
}

// REQ-020: operator extra-args that try to set writer behaviour are refused (no invocation returned).
#[test]
fn req020_writer_flag_extra_args_refused() {
    let fx = Fx::new();
    for flag in [
        "--trust",
        "--worktree",
        "--continue",
        "--auto-approve",
        "--yolo",
    ] {
        let extra = vec![flag.to_string()];
        assert!(
            fx.build("hi", &extra, None, None).is_err(),
            "{flag} must be refused"
        );
    }
}
