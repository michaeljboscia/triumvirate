// GAUNTLET filter suite for T-102, written blind from the spec (no implementation read).
//
// Role: filter. Crate: mcp-bridge (Mistral invocation builder). The parser half lives in
// daemon/crates/agent-adapter/tests/gauntlet_t102_filter.rs.
//
// SPEC_AMBIGUITY: the exact error text for a forbidden extra arg is not stated (only "code first"), so those tests assert
//   Err(..) and a non-empty message, not a code.
// SPEC_AMBIGUITY: argv ORDER is only partly stated (BACKEND_STRUCTURE section 6 lists an order, but the builder contract
//   speaks of flags and values), so tests assert flag/value pairs and counts, not absolute positions.
// SPEC_AMBIGUITY: SeatInvocation.env_remove for the Mistral builder is not stated (T-201 owns the scrub list); not tested.
// SPEC_AMBIGUITY: whether a forbidden flag appearing as the VALUE of an allowed extra arg (for example
//   ["--max-price", "--trust"]) is rejected is implied by "refuses" but not stated; it is tested as rejected because the
//   spec says every listed token is refused.

use mcp_bridge::containment::SeatRequest;
use mcp_bridge::mistral::{
    ENABLED_TOOLS, FORBIDDEN_EXTRA_ARGS, MAX_PROMPT_BYTES, MISTRAL_PROGRAM, build_mistral_invocation,
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
        max_turns: 12,
        time_limit: Duration::from_secs(600),
    }
}

struct Dirs {
    cwd: tempfile::TempDir,
    home: tempfile::TempDir,
    out: tempfile::TempDir,
}

fn dirs() -> Dirs {
    Dirs { cwd: tempfile::tempdir().unwrap(), home: tempfile::tempdir().unwrap(), out: tempfile::tempdir().unwrap() }
}

fn build(d: &Dirs, prompt: &str, model: Option<&str>, resume: Option<&str>, extra: &[String]) -> Result<mcp_bridge::containment::SeatInvocation, String> {
    build_mistral_invocation(&req(prompt, d.cwd.path(), d.home.path(), d.out.path(), model, resume, extra))
}

fn ok(d: &Dirs) -> mcp_bridge::containment::SeatInvocation {
    build(d, "review it", None, None, &[]).expect("builder should succeed")
}

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

fn count(args: &[String], flag: &str) -> usize {
    args.iter().filter(|a| a.as_str() == flag).count()
}

// ---------- constants ----------

#[test]
fn test_should_name_the_program_vibe() {
    // REQ-022: binary is `vibe`
    assert_eq!(MISTRAL_PROGRAM, "vibe");
}

#[test]
fn test_should_cap_prompts_at_64_kib() {
    // REQ-024: limit is 65536 bytes
    assert_eq!(MAX_PROMPT_BYTES, 65536);
}

#[test]
fn test_should_allowlist_exactly_read_file_grep_todo() {
    // REQ-022: read-only tool allowlist
    assert_eq!(ENABLED_TOOLS, ["read_file", "grep", "todo"]);
}

#[test]
fn test_should_list_all_sixteen_forbidden_extra_args() {
    // BACKEND_STRUCTURE section 6, Mistral column
    let want = [
        "--continue", "-c", "--trust", "--worktree", "--auto-approve", "--yolo", "--agent", "--enabled-tools",
        "--disabled-tools", "--output", "--output-dir", "--workdir", "--resume", "-p", "--prompt", "--prompt-file",
    ];
    assert_eq!(FORBIDDEN_EXTRA_ARGS.len(), 16);
    for f in want {
        assert!(FORBIDDEN_EXTRA_ARGS.contains(&f), "missing {f}");
    }
}

// ---------- argv contract (REQ-022) ----------

#[test]
fn test_should_use_vibe_as_program() {
    // REQ-022: program
    let d = dirs();
    assert_eq!(ok(&d).program, "vibe");
}

#[test]
fn test_should_set_vibe_cli_python_in_env_add() {
    // REQ-022: VIBE_CLI=python always
    let d = dirs();
    let inv = ok(&d);
    let vals: Vec<&str> = inv.env_add.iter().filter(|(k, _)| k == "VIBE_CLI").map(|(_, v)| v.as_str()).collect();
    assert_eq!(vals, vec!["python"]);
}

#[test]
fn test_should_pass_prompt_after_dash_p() {
    // REQ-022: `-p <prompt>`
    let d = dirs();
    let inv = build(&d, "please review src", None, None, &[]).unwrap();
    assert_eq!(value_after(&inv.args, "-p"), Some("please review src"));
    assert_eq!(count(&inv.args, "-p"), 1);
}

#[test]
fn test_should_not_reject_a_prompt_that_looks_like_a_flag() {
    // edge: the prompt is a value, never scanned as an extra arg
    let d = dirs();
    let inv = build(&d, "--trust everything", None, None, &[]).unwrap();
    assert_eq!(value_after(&inv.args, "-p"), Some("--trust everything"));
    assert_eq!(count(&inv.args, "--trust"), 0);
}

#[test]
fn test_should_request_streaming_output() {
    // REQ-022: --output streaming
    let d = dirs();
    assert_eq!(value_after(&ok(&d).args, "--output"), Some("streaming"));
}

#[test]
fn test_should_pass_workdir_and_per_call_output_dir() {
    // REQ-022: --workdir <cwd> and --output-dir <per-call dir>
    let d = dirs();
    let inv = ok(&d);
    assert_eq!(value_after(&inv.args, "--workdir"), Some(d.cwd.path().to_str().unwrap()));
    assert_eq!(value_after(&inv.args, "--output-dir"), Some(d.out.path().to_str().unwrap()));
}

#[test]
fn test_should_set_cwd_to_request_cwd() {
    // SeatInvocation.cwd is the repo the seat reads
    let d = dirs();
    assert_eq!(ok(&d).cwd, d.cwd.path().to_path_buf());
}

#[test]
fn test_should_pass_max_turns_and_time_limit_in_seconds() {
    // REQ-022: --max-turns N --time-limit S
    let d = dirs();
    let inv = ok(&d);
    assert_eq!(value_after(&inv.args, "--max-turns"), Some("12"));
    assert_eq!(value_after(&inv.args, "--time-limit"), Some("600"));
}

#[test]
fn test_should_follow_changed_limits() {
    // property: limits come from the request, not constants
    let d = dirs();
    let mut r = req("p", d.cwd.path(), d.home.path(), d.out.path(), None, None, &[]);
    r.max_turns = 3;
    r.time_limit = Duration::from_secs(45);
    let inv = build_mistral_invocation(&r).unwrap();
    assert_eq!(value_after(&inv.args, "--max-turns"), Some("3"));
    assert_eq!(value_after(&inv.args, "--time-limit"), Some("45"));
}

#[test]
fn test_should_label_agent_plan() {
    // REQ-022: --agent plan (a label, never auto-approve)
    let d = dirs();
    let inv = ok(&d);
    assert_eq!(value_after(&inv.args, "--agent"), Some("plan"));
    assert_eq!(count(&inv.args, "--agent"), 1);
}

#[test]
fn test_should_pass_exactly_three_enabled_tools_in_order() {
    // REQ-022: three --enabled-tools: read_file, grep, todo
    let d = dirs();
    let inv = ok(&d);
    assert_eq!(count(&inv.args, "--enabled-tools"), 3);
    let vals: Vec<&str> = inv
        .args
        .iter()
        .enumerate()
        .filter(|(_, a)| a.as_str() == "--enabled-tools")
        .map(|(i, _)| inv.args[i + 1].as_str())
        .collect();
    assert_eq!(vals, vec!["read_file", "grep", "todo"]);
}

#[test]
fn test_should_emit_no_writer_or_unsafe_flags() {
    // REQ-022 / scope_out: never --trust, --worktree, --continue, --auto-approve, --yolo
    let d = dirs();
    let inv = ok(&d);
    for bad in ["--trust", "--worktree", "--continue", "-c", "--auto-approve", "--yolo", "--disabled-tools", "--prompt-file", "--prompt"] {
        assert_eq!(count(&inv.args, bad), 0, "argv contains {bad}");
    }
}

#[test]
fn test_should_not_put_any_model_on_argv() {
    // spec: the model is not on the argv; the seat config selects it
    let d = dirs();
    let inv = build(&d, "p", Some("mistral-large-secret-model"), None, &[]).unwrap();
    assert!(!inv.args.iter().any(|a| a.contains("mistral-large-secret-model")));
    assert_eq!(count(&inv.args, "-m"), 0);
    assert_eq!(count(&inv.args, "--model"), 0);
}

#[test]
fn test_should_omit_resume_for_fresh_session() {
    // REQ-022: --resume only when resuming
    let d = dirs();
    assert_eq!(count(&ok(&d).args, "--resume"), 0);
}

#[test]
fn test_should_pass_resume_with_session_id() {
    // REQ-022: [--resume <id>]
    let d = dirs();
    let inv = build(&d, "p", None, Some("vibe-sess-7f3a"), &[]).unwrap();
    assert_eq!(value_after(&inv.args, "--resume"), Some("vibe-sess-7f3a"));
    assert_eq!(count(&inv.args, "--resume"), 1);
}

#[test]
fn test_should_keep_read_only_flags_when_resuming() {
    // REQ-022: resume does not drop the allowlist or flip the label
    let d = dirs();
    let inv = build(&d, "p", None, Some("sid-1"), &[]).unwrap();
    assert_eq!(count(&inv.args, "--enabled-tools"), 3);
    assert_eq!(value_after(&inv.args, "--agent"), Some("plan"));
    assert_eq!(inv.env_add.iter().filter(|(k, v)| k == "VIBE_CLI" && v == "python").count(), 1);
}

#[test]
fn test_should_be_deterministic() {
    // property: same request, same invocation
    let d = dirs();
    let a = ok(&d);
    let b = ok(&d);
    assert_eq!(a.program, b.program);
    assert_eq!(a.args, b.args);
    assert_eq!(a.env_add, b.env_add);
}

#[test]
fn test_should_follow_prompt_changes() {
    // property: different prompts give different argv
    let d = dirs();
    let a = build(&d, "alpha", None, None, &[]).unwrap();
    let b = build(&d, "beta", None, None, &[]).unwrap();
    assert_ne!(a.args, b.args);
    assert_eq!(value_after(&a.args, "-p"), Some("alpha"));
    assert_eq!(value_after(&b.args, "-p"), Some("beta"));
}

// ---------- prompt size (REQ-024) ----------

#[test]
fn test_should_accept_prompt_of_exactly_max_bytes() {
    // REQ-024 boundary: 65536 bytes allowed
    let d = dirs();
    let p = "a".repeat(MAX_PROMPT_BYTES);
    let inv = build(&d, &p, None, None, &[]).expect("exactly 64 KiB is allowed");
    assert_eq!(value_after(&inv.args, "-p").map(|s| s.len()), Some(MAX_PROMPT_BYTES));
}

#[test]
fn test_should_reject_prompt_one_byte_over_max() {
    // REQ-024 boundary: 65537 bytes rejected with the code first
    let d = dirs();
    let p = "a".repeat(MAX_PROMPT_BYTES + 1);
    let err = build(&d, &p, None, None, &[]).expect_err("must be rejected");
    assert!(err.starts_with("prompt_too_large_for_mistral"), "was {err:?}");
}

#[test]
fn test_should_reject_65_kib_prompt() {
    // REQ-024 reality test: 65 KiB prompt
    let d = dirs();
    let p = "x".repeat(65 * 1024);
    let err = build(&d, &p, None, None, &[]).expect_err("must be rejected");
    assert!(err.starts_with("prompt_too_large_for_mistral"), "was {err:?}");
}

#[test]
fn test_should_measure_prompt_size_in_bytes_not_chars() {
    // REQ-024: the limit is bytes. 32768 two-byte chars = 65536 bytes (ok); one more char = over.
    let d = dirs();
    let at = "\u{e9}".repeat(32768);
    assert_eq!(at.len(), 65536);
    assert!(build(&d, &at, None, None, &[]).is_ok());
    let over = "\u{e9}".repeat(32769);
    let err = build(&d, &over, None, None, &[]).expect_err("must be rejected");
    assert!(err.starts_with("prompt_too_large_for_mistral"), "was {err:?}");
}

#[test]
fn test_should_never_truncate_prompt() {
    // REQ-024: never truncate; an accepted prompt is passed whole
    let d = dirs();
    let p = format!("{}END", "b".repeat(60000));
    let inv = build(&d, &p, None, None, &[]).unwrap();
    assert_eq!(value_after(&inv.args, "-p"), Some(p.as_str()));
}

// ---------- forbidden extra args ----------

#[test]
fn test_should_reject_every_forbidden_extra_arg_alone() {
    // REQ-022 / section 6: each of the 16 tokens refused
    let d = dirs();
    for f in FORBIDDEN_EXTRA_ARGS {
        let extra = vec![f.to_string()];
        let err = build(&d, "p", None, None, &extra).err().unwrap_or_else(|| panic!("{f} was accepted"));
        assert!(!err.is_empty(), "{f}: empty error");
    }
}

#[test]
fn test_should_reject_every_forbidden_extra_arg_in_flag_equals_value_form() {
    // spec: each also matches its --flag=value form
    let d = dirs();
    for f in FORBIDDEN_EXTRA_ARGS.iter().filter(|f| f.starts_with("--")) {
        let extra = vec![format!("{f}=whatever")];
        assert!(build(&d, "p", None, None, &extra).is_err(), "{f}=whatever was accepted");
    }
}

#[test]
fn test_should_reject_short_flag_with_value_attached() {
    // spec: a single-dash short flag also matches with its value attached (-pfoo)
    let d = dirs();
    for s in ["-pfoo", "-cfoo", "-p=foo"] {
        let extra = vec![s.to_string()];
        assert!(build(&d, "p", None, None, &extra).is_err(), "{s} was accepted");
    }
}

#[test]
fn test_should_reject_forbidden_flag_with_separate_value() {
    // spec: forbidden flag followed by its value operand
    let d = dirs();
    for pair in [["--resume", "abc"], ["--agent", "auto-approve"], ["--workdir", "/tmp"], ["--output-dir", "/tmp/x"]] {
        let extra: Vec<String> = pair.iter().map(|s| s.to_string()).collect();
        assert!(build(&d, "p", None, None, &extra).is_err(), "{pair:?} was accepted");
    }
}

#[test]
fn test_should_reject_when_forbidden_arg_follows_allowed_ones() {
    // edge: position in the list does not matter
    let d = dirs();
    let extra: Vec<String> = ["--max-price", "0.5", "--trust"].iter().map(|s| s.to_string()).collect();
    assert!(build(&d, "p", None, None, &extra).is_err());
}

#[test]
fn test_should_reject_forbidden_flag_that_follows_an_allowed_flag_directly() {
    // edge: ["--max-price", "--trust"] still contains --trust
    let d = dirs();
    let extra: Vec<String> = ["--max-price", "--trust"].iter().map(|s| s.to_string()).collect();
    assert!(build(&d, "p", None, None, &extra).is_err());
}

#[test]
fn test_should_reject_writer_flags_even_with_other_extras() {
    // scope_out: writer flags never reach argv
    let d = dirs();
    for w in ["--auto-approve", "--yolo", "--trust", "--worktree", "--continue"] {
        let extra = vec!["--max-price".to_string(), "1".to_string(), w.to_string()];
        assert!(build(&d, "p", None, None, &extra).is_err(), "{w} accepted");
    }
}

#[test]
fn test_should_pass_through_allowed_extra_args() {
    // edge: a non-forbidden extra arg is accepted and reaches argv with its value
    let d = dirs();
    let extra: Vec<String> = ["--max-price", "0.5"].iter().map(|s| s.to_string()).collect();
    let inv = build(&d, "p", None, None, &extra).expect("--max-price is not forbidden");
    assert_eq!(value_after(&inv.args, "--max-price"), Some("0.5"));
    assert_eq!(count(&inv.args, "--enabled-tools"), 3);
}

#[test]
fn test_should_accept_allowed_flag_equals_value_form() {
    // edge: only listed tokens are matched in --flag=value form
    let d = dirs();
    let extra = vec!["--max-price=0.5".to_string()];
    let inv = build(&d, "p", None, None, &extra).expect("--max-price=0.5 is not forbidden");
    assert!(inv.args.iter().any(|a| a == "--max-price=0.5"));
}

#[test]
fn test_should_accept_empty_extra_args() {
    // edge: no extras
    let d = dirs();
    assert!(build(&d, "p", None, None, &[]).is_ok());
}

#[test]
fn test_should_still_emit_exactly_one_of_each_controlled_flag_with_extras() {
    // property: extras cannot duplicate builder-owned flags
    let d = dirs();
    let extra: Vec<String> = ["--max-price", "2"].iter().map(|s| s.to_string()).collect();
    let inv = build(&d, "p", None, None, &extra).unwrap();
    for f in ["-p", "--output", "--workdir", "--output-dir", "--max-turns", "--time-limit", "--agent"] {
        assert_eq!(count(&inv.args, f), 1, "{f}");
    }
}
