//! Slice F: grok integration, offline. REQ-GROK-001..019.
//!
//! Every test here runs against `tests/fixtures/mock_grok.sh`, so `cargo test` needs no network,
//! no `XAI_API_KEY`, and no subscription. Live tests live in the `#[ignore]` block at the bottom
//! behind `TRIUMVIRATE_LIVE_GROK=1`.
//!
//! These assert on things the unit tests structurally cannot: that the argv the builder produces
//! is the argv the CLI actually receives, and that the runner's policy decisions hold end to end.

use std::path::PathBuf;
use std::process::Command;

fn mock_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_grok.sh")
}

/// Run the mock the way the daemon would, capturing the argv it received.
fn run_mock(extra_env: &[(&str, &str)], args: &[&str]) -> (String, String, Vec<String>, bool) {
    // Unique per CALL, not per process: cargo runs these tests in parallel threads inside one
    // binary, so keying on the pid alone made them clobber each other's captured argv.
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let uniq = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let argv_out = std::env::temp_dir()
        .join(format!("grok-argv-{}-{uniq}.txt", std::process::id()));
    let _ = std::fs::remove_file(&argv_out);
    let mut cmd = Command::new(mock_bin());
    cmd.args(args)
        .env("MOCK_GROK_ARGS_OUT", &argv_out);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("mock must run");
    let argv = std::fs::read_to_string(&argv_out)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_file(&argv_out);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        argv,
        out.status.success(),
    )
}

fn parse_stream(stdout: &str) -> agent_adapter::grok::GrokParsed {
    let mut p = agent_adapter::GrokStreamParser::new();
    for line in stdout.lines() {
        let _ = p.parse_line(line);
    }
    p.finish_full()
}

// ─────────────────────────────────────────────────────────────────────────────
// I-01..03: the identity contract
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn i_grok_01_status_list_contains_grok_and_equals_the_allowlist() {
    let advertised = mcp_bridge::supported_agent_names();
    assert!(advertised.contains(&"grok"));
    // The advertised list and the dispatch gate must BE the same list, not two lists that
    // happen to agree. Four surfaces had drifted into three different answers before this.
    for a in advertised {
        assert!(
            mcp_bridge::is_supported_agent_name(a),
            "{a} is advertised on /status but would be refused by dispatch"
        );
    }
}

#[test]
fn i_grok_03_supergrok_alias_is_accepted_and_normalized_at_the_boundary() {
    assert!(mcp_bridge::is_supported_agent_name("supergrok"));
    assert_eq!(mcp_bridge::normalize_agent_name("SuperGrok"), "grok");
    // A raw alias must never reach dispatch: the arms match on the canonical key only.
    assert_eq!(mcp_bridge::display_agent_name("supergrok"), "Grok");
}

// ─────────────────────────────────────────────────────────────────────────────
// The argv the builder produces is the argv the process receives
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn i_grok_04_builder_argv_survives_the_process_boundary() {
    let inv = mcp_bridge::grok::build_grok_invocation(
        mock_bin().to_str().unwrap(),
        &[],
        "the prompt",
        "/tmp",
        Some("11111111-2222-3333-4444-555555555555"),
        false,
    )
    .expect("builder must succeed");

    let args: Vec<&str> = inv.args.iter().map(String::as_str).collect();
    let (stdout, _, argv, ok) = run_mock(&[], &args);
    assert!(ok);

    // Exactly what the builder promised, in order, with nothing lost or reordered.
    assert_eq!(argv, inv.args, "the CLI received a different argv than the builder produced");
    assert!(argv.contains(&"--output-format".to_string()));
    assert!(argv.contains(&"streaming-json".to_string()));
    assert!(argv.contains(&"--sandbox".to_string()), "consults must be write-contained");
    assert!(!argv.contains(&"--continue".to_string()));
    assert_eq!(argv[argv.len() - 2], "-p", "-p must stay adjacent to its value");
    assert_eq!(argv[argv.len() - 1], "the prompt");

    let full = parse_stream(&stdout);
    assert!(full.parsed.response_text.contains("pong"));
}

#[test]
fn i_grok_07_resume_passes_the_parsed_session_id_not_the_requested_one() {
    // Turn 1: new session.
    let inv1 = mcp_bridge::grok::build_grok_invocation(
        mock_bin().to_str().unwrap(), &[], "turn one", "/tmp",
        Some("aaaaaaaa-1111-2222-3333-444444444444"), false,
    ).unwrap();
    let a1: Vec<&str> = inv1.args.iter().map(String::as_str).collect();
    let (out1, _, argv1, _) = run_mock(&[], &a1);
    assert!(argv1.contains(&"--session-id".to_string()));
    assert!(!argv1.contains(&"--resume".to_string()), "turn 1 must not resume");

    let parsed1 = parse_stream(&out1).parsed;
    let sid = parsed1.session_id.expect("turn 1 must yield a session id");

    // Turn 2: resume with the PARSED id, which is the contract REQ-GROK-007 sets.
    let inv2 = mcp_bridge::grok::build_grok_invocation(
        mock_bin().to_str().unwrap(), &[], "turn two", "/tmp", Some(&sid), true,
    ).unwrap();
    let a2: Vec<&str> = inv2.args.iter().map(String::as_str).collect();
    let (_, _, argv2, _) = run_mock(&[], &a2);

    let i = argv2.iter().position(|a| a == "--resume").expect("turn 2 must resume");
    assert_eq!(argv2[i + 1], sid, "resume must carry the PARSED id");
    assert!(!argv2.contains(&"--session-id".to_string()), "the CLI rejects -s with -r");
}

/// A bare `--resume` attaches to the most recent session in the cwd, which is cross-session
/// contamination. It must be impossible to produce, not merely untested.
#[test]
fn i_grok_09_a_bare_resume_can_never_be_emitted() {
    for empty in [None, Some(""), Some("  ")] {
        assert!(
            mcp_bridge::grok::build_grok_invocation(
                mock_bin().to_str().unwrap(), &[], "p", "/tmp", empty, true,
            ).is_err(),
            "resume with an empty id must fail rather than emit a bare --resume"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Runner policy, end to end through the mock
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn i_grok_11_chain_of_thought_never_reaches_the_answer() {
    let inv = mcp_bridge::grok::build_grok_invocation(
        mock_bin().to_str().unwrap(), &[], "hi", "/tmp", None, false,
    ).unwrap();
    let args: Vec<&str> = inv.args.iter().map(String::as_str).collect();
    let (stdout, _, _, _) = run_mock(&[], &args);
    assert!(stdout.contains("CHAIN OF THOUGHT MUST NOT LEAK"), "the mock must emit a thought");
    let full = parse_stream(&stdout);
    assert!(
        !full.parsed.response_text.contains("CHAIN OF THOUGHT"),
        "thoughts must not reach the operator-facing answer"
    );
}

#[test]
fn i_grok_12_max_turns_is_classified_so_the_runner_can_withhold() {
    let (stdout, _, _, _) = run_mock(&[("MOCK_GROK_MODE", "max_turns")], &["-p", "hi"]);
    let full = parse_stream(&stdout);
    assert_eq!(full.termination, agent_adapter::GrokTermination::MaxTurnsReached);
    // The partial text is preserved and handed up. The parser neither discards nor blesses it.
    assert!(full.parsed.response_text.contains("pong"));
}

#[test]
fn i_grok_13_a_missing_end_event_is_incomplete_not_success() {
    let (stdout, _, _, _) = run_mock(&[("MOCK_GROK_MODE", "no_end")], &["-p", "hi"]);
    let full = parse_stream(&stdout);
    assert_eq!(full.termination, agent_adapter::GrokTermination::Incomplete);
}

#[test]
fn i_grok_14_error_events_carry_their_detail() {
    let (stdout, _, _, ok) = run_mock(&[("MOCK_GROK_MODE", "error")], &["-p", "hi"]);
    assert!(!ok, "error mode must exit nonzero");
    let full = parse_stream(&stdout);
    assert_eq!(full.termination, agent_adapter::GrokTermination::Errored);
    assert_eq!(full.error_detail.as_deref(), Some("mock grok failure"));
}

/// The silent-failure case the live probe found: an unknown sandbox profile does NOT fail grok,
/// it warns and runs uncontained. The runner must be able to see that.
#[test]
fn i_grok_15_an_unapplied_sandbox_is_visible_on_stderr() {
    let (_, stderr, _, ok) = run_mock(&[("MOCK_GROK_MODE", "sandbox_fail")], &["-p", "hi"]);
    assert!(ok, "grok exits 0 even when the sandbox did not apply, which is why this matters");
    assert!(
        stderr.contains("sandbox could not be applied"),
        "the runner keys on this exact string to refuse an uncontained run"
    );
}

#[test]
fn i_grok_16_auth_failure_is_distinguishable_from_other_failures() {
    let (_, stderr, _, ok) = run_mock(&[("MOCK_GROK_MODE", "auth_fail")], &["-p", "hi"]);
    assert!(!ok);
    let h = stderr.to_lowercase();
    assert!(h.contains("unauthorized") || h.contains("401") || h.contains("login"));
}

#[test]
fn i_grok_17_usage_and_self_reported_cost_are_captured() {
    let (stdout, _, _, _) = run_mock(&[], &["-p", "hi"]);
    let full = parse_stream(&stdout);
    let u = full.parsed.token_usage.expect("usage must be captured");
    assert_eq!(u.input, Some(14386));
    assert_eq!(u.output, Some(31));
    assert_eq!(u.tool_calls, Some(1));
    // Grok is the only sibling that is subscription-billed AND self-reporting. On a flat plan
    // this is a usage signal, not a bill, and quota burn is invisible without it.
    assert_eq!(full.total_cost_usd, Some(0.00493));
}

#[test]
fn i_grok_18_tool_calls_are_recorded_with_their_kind_and_outcome() {
    let (stdout, _, _, _) = run_mock(&[], &["-p", "hi"]);
    let r = parse_stream(&stdout).parsed;
    assert_eq!(r.tool_calls.len(), 1);
    assert_eq!(r.tool_calls[0].tool, "read_file");
    assert_eq!(r.tool_calls[0].kind, agent_adapter::types::ToolKind::ReadFile);
    assert_eq!(r.tool_calls[0].success, Some(true));
}

#[test]
fn i_grok_19_operator_extras_cannot_smuggle_a_managed_flag() {
    for hostile in ["--always-approve", "--sandbox=off", "-rp", "--system-prompt-override"] {
        assert!(
            mcp_bridge::grok::build_grok_invocation(
                mock_bin().to_str().unwrap(), &[hostile.to_string()], "p", "/tmp", None, false,
            ).is_err(),
            "{hostile} must be refused before spawn"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Slice L: live tests. Opt-in, and they spend real subscription quota.
// ─────────────────────────────────────────────────────────────────────────────

fn live_enabled() -> bool {
    std::env::var("TRIUMVIRATE_LIVE_GROK").map(|v| v == "1").unwrap_or(false)
}

#[test]
#[ignore = "live: set TRIUMVIRATE_LIVE_GROK=1; spends subscription quota"]
fn e_grok_01_live_consult_answers_and_returns_a_session_id() {
    if !live_enabled() {
        return;
    }
    let (bin, args) = mcp_bridge::grok_command();
    let inv = mcp_bridge::grok::build_grok_invocation(
        &bin, &args, "reply with the single word pong", "/tmp", None, false,
    ).unwrap();
    let out = Command::new(&inv.program).args(&inv.args).output().expect("grok must run");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let full = parse_stream(&String::from_utf8_lossy(&out.stdout));
    assert!(full.parsed.response_text.to_lowercase().contains("pong"));
    assert!(full.parsed.session_id.is_some(), "REQ-GROK-007");
    assert_eq!(full.termination, agent_adapter::GrokTermination::EndTurn);
}

#[test]
#[ignore = "live: set TRIUMVIRATE_LIVE_GROK=1; proves subscription auth needs no API key"]
fn e_grok_03_headless_works_on_cached_login_without_an_api_key() {
    if !live_enabled() {
        return;
    }
    let (bin, args) = mcp_bridge::grok_command();
    let inv = mcp_bridge::grok::build_grok_invocation(
        &bin, &args, "reply with the single word pong", "/tmp", None, false,
    ).unwrap();
    let out = Command::new(&inv.program)
        .args(&inv.args)
        .env_remove("XAI_API_KEY")
        .output()
        .expect("grok must run");
    assert!(out.status.success(), "subscription auth must be sufficient for headless");
}
