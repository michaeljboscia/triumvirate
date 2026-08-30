//! Grok Build CLI invocation assembly. REQ-GROK-004/005/006/008/009/010/014.
//!
//! Grok is a SPAWNED CLI, like codex exec and agy, not HTTP like DeepSeek. This module owns
//! the argument vector so that session, output format, cwd and approval policy have a single
//! source of truth, and an operator's `TRIUMVIRATE_GROK_ARGS` cannot override them.
//!
//! Session semantics were verified against `grok 1.0.13`, not taken from documentation, because
//! the upstream docs contradict each other on `-s` versus `-r`. The binary is unambiguous:
//!
//!   -s, --session-id <ID>  "for a NEW conversation (must not already exist) ... Does not resume
//!                           existing sessions, use --resume / --continue instead"
//!   -r, --resume [<ID>]    note the BRACKETS: the argument is OPTIONAL
//!
//! Two consequences drive this module:
//!
//! 1. A bare `-r` means "most recent session in this cwd", which is exactly the cross-session
//!    cross-talk that makes `--continue` forbidden. So an empty resume id is an ERROR here, never
//!    a fallback. Emitting `-r` with nothing after it would silently attach to whatever ran last.
//! 2. `-s` and `-r` are mutually exclusive unless `--fork-session` is passed, and the CLI enforces
//!    it. The builder never emits both.
//!
//! `--resume` also accepts a session TITLE, not just a uuid, so passing a Triumvirate session name
//! where an id belongs resolves silently to the wrong conversation instead of erroring. Callers
//! must pass the parsed `end.sessionId`.

use std::time::Duration;

/// Grok's connector timeout. REQ-GROK-014.
pub fn grok_connector_timeout() -> Duration {
    std::env::var("TRIUMVIRATE_GROK_CONNECTOR_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(900))
}

/// REQ-GROK-009: auto-approval is OPT-IN. A consult must not mutate the workspace because a
/// tool call was hallucinated or auto-approved.
pub fn grok_yolo_enabled() -> bool {
    std::env::var("TRIUMVIRATE_GROK_YOLO")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

/// REQ-GROK-005: streaming on by default. Mirrors `TRIUMVIRATE_GEMINI_STREAMING`.
pub fn grok_streaming_enabled() -> bool {
    std::env::var("TRIUMVIRATE_GROK_STREAMING")
        .map(|v| !matches!(v.to_lowercase().as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(true)
}

/// Runaway guard. Each turn re-ships the whole system prompt and tool schemas, so turns are the
/// unit of spend here, not tokens.
pub fn grok_max_turns() -> u32 {
    std::env::var("TRIUMVIRATE_GROK_MAX_TURNS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(20)
}

/// REQ-GROK-019: write containment for the consult path, the equivalent of agy's H4 rule.
/// Verified profiles on grok 1.0.13: `workspace`, `read-only`, `strict`, `off`.
///
/// **An unknown profile does NOT fail.** It prints "sandbox could not be applied" to stderr and
/// the run continues with NO containment. So the runner must treat that string as a hard error;
/// a silently-unsandboxed consult is worse than a refused one.
pub fn grok_sandbox_profile() -> Option<String> {
    match std::env::var("TRIUMVIRATE_GROK_SANDBOX").ok().as_deref() {
        Some("off") | Some("") => None,
        Some(p) => Some(p.to_string()),
        // Default: reads allowed, writes denied. A consult must not mutate the workspace
        // because a tool call was hallucinated or auto-approved.
        None => Some("read-only".to_string()),
    }
}

pub fn grok_model() -> Option<String> {
    std::env::var("TRIUMVIRATE_GROK_MODEL").ok().filter(|s| !s.trim().is_empty())
}

pub fn grok_effort() -> Option<String> {
    std::env::var("TRIUMVIRATE_GROK_EFFORT").ok().filter(|s| !s.trim().is_empty())
}

/// Flags Triumvirate owns. REQ-GROK-008, mirroring agy's H3 rule.
///
/// `--permission-mode`, `--json-schema`, `--tools` and `--disallowed-tools` are here for reasons
/// the vendor guide missed: the first two change approval policy and output shape, and the last
/// two change which tool schemas ship in the system prompt. All four are Triumvirate's to decide.
const FORBIDDEN_EXTRA_FLAGS: &[&str] = &[
    "-p",
    "--single",
    "--prompt",
    "--prompt-file",
    "--prompt-json",
    "-o",
    "--output-format",
    "-r",
    "--resume",
    "-s",
    "--session-id",
    "-c",
    "--continue",
    "--cwd",
    "--fork-session",
    "--always-approve",
    "--yolo",
    "--dangerously-skip-permissions",
    "--sandbox",
    "--permission-mode",
    "--json-schema",
    "--tools",
    "--disallowed-tools",
    "--max-turns",
    "-m",
    "--model",
    "--effort",
    "--reasoning-effort",
    "--no-auto-update",
    "--no-alt-screen",
    // Named by Codex from the live `grok --help`. Each one lets an operator change what the
    // agent IS, what it may touch, or where it runs, all of which Triumvirate owns.
    "--system-prompt",
    "--system-prompt-override",
    "--agent",
    "--agents",
    "--allow",
    "--allowedTools",
    "--allowed-tools",
    "--deny",
    "--disallowedTools",
    "--leader-socket",
    "--no-subagents",
    "--no-plan",
    "--restore-code",
    "--rules",
    "--verbatim",
    "--include-partial-messages",
    "--disable-web-search",
    "-w",
    "--worktree",
    "--worktree-ref",
    "--ref",
];

/// Managed flags in short form. Checked separately because clustered shorts (`-rp`) and
/// joined values (`-mfoo`) never match a whole-token comparison.
const FORBIDDEN_SHORT_FLAGS: &[char] = &['p', 'r', 's', 'c', 'o', 'm', 'w'];

/// A session id or cwd that begins with `-` is parsed by clap as the NEXT FLAG, not as the
/// value of the one before it. `--resume -c` becomes `--resume` with no argument followed by
/// `--continue`, which is precisely the bare-resume cross-talk this module exists to prevent.
/// Found by Codex in review of the first draft.
fn validate_argv_value(kind: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("grok {kind} must not be empty"));
    }
    if value.starts_with('-') {
        return Err(format!(
            "grok {kind} {value:?} begins with '-': clap would parse it as a flag rather than a \
             value, silently changing the invocation"
        ));
    }
    Ok(())
}

/// Reject operator extra-args that would override a Triumvirate-managed flag. Matches both the
/// spaced form (`--model x`) and the joined form (`--model=x`); checking only one is a hole.
fn validate_extra_args(extra: &[String]) -> Result<(), String> {
    for arg in extra {
        let flag = arg.split('=').next().unwrap_or(arg);
        // Clustered or joined shorts: `-rp`, `-mfoo`, `-sUUID`. A whole-token match misses all
        // of these, so inspect every character of a short-flag cluster.
        if flag.starts_with('-')
            && !flag.starts_with("--")
            && flag.len() > 1
            && let Some(c) = flag.chars().skip(1).find(|c| FORBIDDEN_SHORT_FLAGS.contains(c))
        {
            return Err(format!(
                "TRIUMVIRATE_GROK_ARGS contains forbidden short flag '-{c}' inside {flag:?}: grok \
                 session, output, approval and context flags are managed by Triumvirate"
            ));
        }
        if FORBIDDEN_EXTRA_FLAGS.contains(&flag) {
            return Err(format!(
                "TRIUMVIRATE_GROK_ARGS contains forbidden flag {flag:?}: grok session, output, \
                 approval and context flags are managed by Triumvirate and cannot be overridden"
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrokInvocation {
    pub program: String,
    pub args: Vec<String>,
    pub timeout: Duration,
}

/// Build grok's argument vector.
///
/// `session_id` is the uuid for a NEW session when `resume` is false, and the previously parsed
/// `end.sessionId` when `resume` is true. Passing `resume = true` without an id is an error, not
/// a fallback: see the module docs on bare `-r`.
pub fn build_grok_invocation(
    bin: &str,
    extra_args: &[String],
    prompt: &str,
    cwd: &str,
    session_id: Option<&str>,
    resume: bool,
) -> Result<GrokInvocation, String> {
    build_grok_invocation_with_sandbox(bin, extra_args, prompt, cwd, session_id, resume, None)
}

/// As `build_grok_invocation`, with an explicit containment profile.
///
/// Exists because an ABE worker needs a WRITABLE sandbox (it is there to produce code in its own
/// worktree) while a consult needs `read-only`. The first version passed that by temporarily
/// mutating `TRIUMVIRATE_GROK_SANDBOX`, which is a race in a threaded daemon and was caught by a
/// test asserting the operator's value survived. Pass the parameter instead of touching global
/// state.
///
/// `sandbox_override: None` means "use the operator's configuration".
#[allow(clippy::too_many_arguments)]
pub fn build_grok_invocation_with_sandbox(
    bin: &str,
    extra_args: &[String],
    prompt: &str,
    cwd: &str,
    session_id: Option<&str>,
    resume: bool,
    sandbox_override: Option<&str>,
) -> Result<GrokInvocation, String> {
    validate_extra_args(extra_args)?;

    // REQ-GROK-006. A bare `-r` resumes "most recent session in this cwd", which is the exact
    // cross-talk `--continue` is banned for. Fail loudly rather than attach to a stranger.
    let resume_id = if resume {
        let id = session_id.map(str::trim).unwrap_or("");
        if id.is_empty() {
            return Err(
                "grok resume requested without a session id: emitting a bare --resume would \
                 attach to the most recent session in this cwd (cross-session contamination). \
                 Pass the sessionId parsed from the previous turn's `end` event."
                    .to_string(),
            );
        }
        validate_argv_value("resume session id", id)?;
        Some(id.to_string())
    } else {
        if let Some(id) = session_id.filter(|s| !s.trim().is_empty()) {
            validate_argv_value("session id", id)?;
        }
        None
    };

    let mut args: Vec<String> = Vec::new();

    // REQ-GROK-010: never let a daemon-spawned process self-update or grab the alt screen.
    args.push("--no-auto-update".to_string());
    args.push("--no-alt-screen".to_string());

    // REQ-GROK-005. The CLI default is `plain`, which is unparseable prose, so this is never
    // optional: a dropped flag degrades silently rather than failing.
    args.push("--output-format".to_string());
    args.push(if grok_streaming_enabled() { "streaming-json" } else { "json" }.to_string());

    if !cwd.trim().is_empty() {
        validate_argv_value("cwd", cwd)?;
        args.push("--cwd".to_string());
        args.push(cwd.to_string());
    }

    // Mutually exclusive without --fork-session, and the CLI enforces it.
    match (&resume_id, session_id) {
        (Some(id), _) => {
            args.push("--resume".to_string());
            args.push(id.clone());
        }
        (None, Some(id)) if !id.trim().is_empty() => {
            args.push("--session-id".to_string());
            args.push(id.to_string());
        }
        _ => {}
    }

    if let Some(model) = grok_model() {
        args.push("-m".to_string());
        args.push(model);
    }
    if let Some(effort) = grok_effort() {
        args.push("--effort".to_string());
        args.push(effort);
    }

    args.push("--max-turns".to_string());
    args.push(grok_max_turns().to_string());

    // Containment before approval, so the ordering reads as the policy it is: contain first,
    // then decide what may be approved inside that containment.
    let profile = match sandbox_override {
        Some("off") | Some("") => None,
        Some(p) => Some(p.to_string()),
        None => grok_sandbox_profile(),
    };
    if let Some(profile) = profile {
        args.push("--sandbox".to_string());
        args.push(profile);
    }

    if grok_yolo_enabled() {
        args.push("--always-approve".to_string());
    }

    // Operator extras are already validated. They go BEFORE `-p` so nothing can land between
    // `-p` and its value.
    args.extend(extra_args.iter().cloned());

    args.push("-p".to_string());
    args.push(prompt.to_string());

    Ok(GrokInvocation {
        program: bin.to_string(),
        args,
        timeout: grok_connector_timeout(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Clear every knob so a test sees documented defaults regardless of the developer's shell.
    fn clear_env() {
        for k in [
            "TRIUMVIRATE_GROK_YOLO",
            "TRIUMVIRATE_GROK_STREAMING",
            "TRIUMVIRATE_GROK_MAX_TURNS",
            "TRIUMVIRATE_GROK_MODEL",
            "TRIUMVIRATE_GROK_EFFORT",
            "TRIUMVIRATE_GROK_CONNECTOR_TIMEOUT_SECS",
        ] {
            // SAFETY: test controls env var lifecycle in-process, guarded by env_lock.
            unsafe { std::env::remove_var(k) };
        }
    }

    fn build(session: Option<&str>, resume: bool) -> Result<GrokInvocation, String> {
        build_grok_invocation("grok", &[], "hello", "/tmp/ws", session, resume)
    }

    /// `--flag value` pairs: find the value that follows a flag.
    fn value_after(args: &[String], flag: &str) -> Option<String> {
        args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
    }

    // ---- U-B-01: default consult shape ----
    #[test]
    fn u_b_01_default_consult_is_streaming_json_with_prompt_last() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let inv = build(None, false).expect("default consult must build");
        assert_eq!(value_after(&inv.args, "--output-format").as_deref(), Some("streaming-json"));
        // `-p` must be immediately before its value, and last, so nothing can separate them.
        let n = inv.args.len();
        assert_eq!(inv.args[n - 2], "-p");
        assert_eq!(inv.args[n - 1], "hello");
        assert!(!inv.args.contains(&"--always-approve".to_string()), "yolo must be opt-in");
    }

    // ---- U-B-02: REQ-GROK-006, never --continue ----
    #[test]
    fn u_b_02_never_emits_continue() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        for (s, r) in [(None, false), (Some("abc"), false), (Some("abc"), true)] {
            let inv = build(s, r).expect("must build");
            assert!(!inv.args.iter().any(|a| a == "-c" || a == "--continue"),
                "--continue cross-talks between sessions and is never valid");
        }
    }

    // ---- U-B-03 / U-B-04: session flags are mutually exclusive ----
    #[test]
    fn u_b_03_resume_uses_resume_and_never_session_id() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let inv = build(Some("sess-123"), true).expect("resume must build");
        assert_eq!(value_after(&inv.args, "--resume").as_deref(), Some("sess-123"));
        assert!(!inv.args.contains(&"--session-id".to_string()),
            "the CLI rejects -s with -r unless --fork-session is passed");
    }

    #[test]
    fn u_b_04_new_session_uses_session_id_and_never_resume() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let inv = build(Some("uuid-abc"), false).expect("new session must build");
        assert_eq!(value_after(&inv.args, "--session-id").as_deref(), Some("uuid-abc"));
        assert!(!inv.args.contains(&"--resume".to_string()));
    }

    // ---- U-B-05: the hazard the vendor guide missed ----
    #[test]
    fn u_b_05_resume_without_an_id_is_an_error_never_a_bare_dash_r() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        for empty in [None, Some(""), Some("   ")] {
            let err = build(empty, true).expect_err("resume with no id must fail");
            assert!(err.contains("cross-session"), "error must explain WHY: {err}");
        }
        // And prove the dangerous form is genuinely unreachable, not merely untested.
        let inv = build(Some("real-id"), true).unwrap();
        let i = inv.args.iter().position(|a| a == "--resume").unwrap();
        assert!(inv.args.get(i + 1).is_some_and(|v| !v.starts_with('-')),
            "--resume must always be followed by a real id");
    }

    // ---- U-B-06 / U-B-07: forbidden flags, both spellings ----
    #[test]
    fn u_b_06_forbidden_extra_flags_rejected_in_both_forms() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        for bad in ["--model", "--output-format", "-p", "--resume", "--cwd", "--always-approve"] {
            for form in [vec![bad.to_string(), "x".into()], vec![format!("{bad}=x")]] {
                let err = build_grok_invocation("grok", &form, "hi", "/tmp", None, false)
                    .expect_err("forbidden flag must be rejected");
                assert!(err.contains(bad), "error must name the flag: {err}");
            }
        }
    }

    #[test]
    fn u_b_07_context_and_approval_flags_are_triumvirate_owned() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        // Not in the vendor guide's list. --permission-mode and --json-schema change approval
        // policy and output shape; --tools and --disallowed-tools change what ships in the prompt.
        for bad in ["--permission-mode", "--json-schema", "--tools", "--disallowed-tools"] {
            assert!(build_grok_invocation("grok", &[bad.to_string(), "x".into()], "hi", "/tmp", None, false).is_err(),
                "{bad} must be operator-forbidden");
        }
    }

    // ---- U-B-08: yolo ----
    #[test]
    fn u_b_08_yolo_is_opt_in() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        assert!(!build(None, false).unwrap().args.contains(&"--always-approve".to_string()));
        // SAFETY: guarded by env_lock.
        unsafe { std::env::set_var("TRIUMVIRATE_GROK_YOLO", "1") };
        assert!(build(None, false).unwrap().args.contains(&"--always-approve".to_string()));
        clear_env();
    }

    // ---- U-B-09 / U-B-12 ----
    #[test]
    fn u_b_09_update_and_altscreen_flags_always_present() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let inv = build(None, false).unwrap();
        assert!(inv.args.contains(&"--no-auto-update".to_string()));
        assert!(inv.args.contains(&"--no-alt-screen".to_string()));
    }

    #[test]
    fn u_b_12_cwd_present_when_given_and_absent_when_blank() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        assert_eq!(value_after(&build(None, false).unwrap().args, "--cwd").as_deref(), Some("/tmp/ws"));
        let inv = build_grok_invocation("grok", &[], "hi", "   ", None, false).unwrap();
        assert!(!inv.args.contains(&"--cwd".to_string()), "blank cwd must not emit an empty flag");
    }

    // ---- U-B-10: streaming toggle ----
    #[test]
    fn u_b_10_streaming_off_falls_back_to_batch_json() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        // SAFETY: guarded by env_lock.
        unsafe { std::env::set_var("TRIUMVIRATE_GROK_STREAMING", "0") };
        assert_eq!(value_after(&build(None, false).unwrap().args, "--output-format").as_deref(), Some("json"));
        clear_env();
        assert_eq!(value_after(&build(None, false).unwrap().args, "--output-format").as_deref(), Some("streaming-json"));
    }

    // ---- U-B-11: optional knobs ----
    #[test]
    fn u_b_11_model_effort_and_max_turns_follow_config_only() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let inv = build(None, false).unwrap();
        assert!(!inv.args.contains(&"-m".to_string()), "must not invent a model");
        assert!(!inv.args.contains(&"--effort".to_string()));
        assert_eq!(value_after(&inv.args, "--max-turns").as_deref(), Some("20"), "documented default");

        // SAFETY: guarded by env_lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GROK_MODEL", "grok-4.6-build");
            std::env::set_var("TRIUMVIRATE_GROK_EFFORT", "high");
            std::env::set_var("TRIUMVIRATE_GROK_MAX_TURNS", "3");
        }
        let inv = build(None, false).unwrap();
        assert_eq!(value_after(&inv.args, "-m").as_deref(), Some("grok-4.6-build"));
        assert_eq!(value_after(&inv.args, "--effort").as_deref(), Some("high"));
        assert_eq!(value_after(&inv.args, "--max-turns").as_deref(), Some("3"));
        clear_env();
    }

    // ---- U-B-13: ordering invariant ----
    #[test]
    fn u_b_13_operator_extras_can_never_separate_p_from_its_value() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let extras = vec!["--debug".to_string(), "--debug-file".to_string(), "/tmp/x.log".to_string()];
        let inv = build_grok_invocation("grok", &extras, "the prompt", "/tmp", None, false).unwrap();
        let n = inv.args.len();
        assert_eq!(inv.args[n - 2], "-p");
        assert_eq!(inv.args[n - 1], "the prompt");
        assert!(inv.args.contains(&"--debug".to_string()), "valid extras must survive");
    }

    #[test]
    fn u_b_14_timeout_default_is_900s_and_configurable() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        assert_eq!(build(None, false).unwrap().timeout, Duration::from_secs(900));
        // SAFETY: guarded by env_lock.
        unsafe { std::env::set_var("TRIUMVIRATE_GROK_CONNECTOR_TIMEOUT_SECS", "42") };
        assert_eq!(build(None, false).unwrap().timeout, Duration::from_secs(42));
        clear_env();
    }

    // ---- Defects found in peer review of the first draft ----

    /// Codex: a session id beginning with '-' is parsed by clap as the next FLAG, so
    /// `--resume -c` becomes a bare `--resume` followed by `--continue`. That is the exact
    /// cross-talk this module exists to prevent, reached through the flag it recommends.
    #[test]
    fn u_b_15_session_id_cannot_inject_a_flag() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        for hostile in ["-c", "--continue", "-p", "--always-approve", "-"] {
            assert!(build(Some(hostile), true).is_err(), "resume id {hostile:?} must be rejected");
            assert!(build(Some(hostile), false).is_err(), "session id {hostile:?} must be rejected");
        }
        // A legitimate uuid still works.
        assert!(build(Some("CD94C2BD-530A-48E7-8EA4-91D7853CE6B0"), false).is_ok());
    }

    #[test]
    fn u_b_16_cwd_cannot_inject_a_flag() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        assert!(build_grok_invocation("grok", &[], "hi", "--always-approve", None, false).is_err());
        assert!(build_grok_invocation("grok", &[], "hi", "/tmp/ok", None, false).is_ok());
    }

    /// Codex: `split('=')` catches `--model=x` but never `-rp` or `-mfoo`.
    #[test]
    fn u_b_17_clustered_and_joined_short_flags_are_caught() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        for hostile in ["-rp", "-mfoo", "-sUUID", "-pc", "-o"] {
            let err = build_grok_invocation("grok", &[hostile.to_string()], "hi", "/tmp", None, false)
                .expect_err("{hostile} must be rejected");
            assert!(err.contains("forbidden"), "{hostile}: {err}");
        }
        // A short flag grok owns but Triumvirate does not must still pass.
        assert!(build_grok_invocation("grok", &["-h".to_string()], "hi", "/tmp", None, false).is_ok());
    }

    /// Antigravity: agy denies workspace writes on the consult path (its H4 rule) and grok had
    /// no containment at all, while grok's own config sets permission_mode = "always-approve".
    #[test]
    fn u_b_18_consults_are_write_contained_by_default() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        assert_eq!(value_after(&build(None, false).unwrap().args, "--sandbox").as_deref(),
            Some("read-only"), "a consult must not be able to mutate the workspace");
        // SAFETY: guarded by env_lock.
        unsafe { std::env::set_var("TRIUMVIRATE_GROK_SANDBOX", "off") };
        assert!(!build(None, false).unwrap().args.contains(&"--sandbox".to_string()),
            "containment must be disableable, but only deliberately");
        // SAFETY: guarded by env_lock.
        unsafe { std::env::set_var("TRIUMVIRATE_GROK_SANDBOX", "workspace") };
        assert_eq!(value_after(&build(None, false).unwrap().args, "--sandbox").as_deref(), Some("workspace"));
        clear_env();
    }

    /// Codex: `value_after` returns the FIRST match, so a duplicated managed flag would go
    /// unnoticed. grok takes the last occurrence, so a duplicate silently wins.
    #[test]
    fn u_b_19_managed_flags_appear_exactly_once() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        // SAFETY: guarded by env_lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GROK_MODEL", "grok-4.6-build");
            std::env::set_var("TRIUMVIRATE_GROK_EFFORT", "high");
        }
        let inv = build(Some("sess-1"), true).unwrap();
        for flag in ["--output-format", "--cwd", "--resume", "--max-turns", "-m", "--effort", "-p", "--sandbox"] {
            let n = inv.args.iter().filter(|a| *a == flag).count();
            assert_eq!(n, 1, "{flag} appears {n} times; grok takes the LAST, so a duplicate silently wins");
        }
        assert_eq!(inv.args.iter().filter(|a| *a == "--session-id").count(), 0);
        clear_env();
    }

    /// Every entry in the forbidden list must actually be rejected. Sampling six of them, as the
    /// first draft did, leaves the rest asserted only by inspection.
    #[test]
    fn u_b_20_every_forbidden_flag_is_actually_rejected() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        for flag in FORBIDDEN_EXTRA_FLAGS {
            let spaced = build_grok_invocation("grok", &[flag.to_string(), "v".into()], "hi", "/tmp", None, false);
            assert!(spaced.is_err(), "{flag} (spaced) must be rejected");
            let joined = build_grok_invocation("grok", &[format!("{flag}=v")], "hi", "/tmp", None, false);
            assert!(joined.is_err(), "{flag}=v (joined) must be rejected");
        }
    }

}
