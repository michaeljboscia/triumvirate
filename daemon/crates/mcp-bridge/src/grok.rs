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

/// Runaway guard, and the main latency control.
///
/// Turns are the unit of both spend and WALL TIME here: each one is a fresh 5 to 12 second model
/// round trip that re-ships the whole system prompt and every tool schema. The old default of 20
/// permitted a twenty-round-trip explore, which is exactly where "grok takes 3 to 5 minutes" came
/// from. Grok's own recommendation was 1 for a consult and 4 for a file-reading review.
///
/// 6 is the compromise: enough to read a few files and answer, far short of an open-ended crawl.
/// Raise it deliberately with TRIUMVIRATE_GROK_MAX_TURNS for genuine review work.
pub fn grok_max_turns() -> u32 {
    grok_max_turns_for(grok_depth())
}

/// As `grok_max_turns`, for a caller that has already decided the depth for THIS invocation
/// rather than reading the daemon's process-wide setting. FIND-GROK-04.
pub fn grok_max_turns_for(depth: GrokDepth) -> u32 {
    std::env::var("TRIUMVIRATE_GROK_MAX_TURNS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(match depth {
            GrokDepth::Fast => 6,
            // Enough rope to actually explore a subsystem. The 900s client timeout is what stops
            // a runaway now, not the turn cap.
            GrokDepth::Deep => 30,
        })
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


/// How hard a peer is asked to work. REQ-GROK-020.
///
/// Two coherent profiles rather than a scatter of independent knobs, because the settings only
/// make sense together: capping turns while leaving effort on `high` still pays the reasoning tax,
/// and raising effort while forbidding subagents still cannot explore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrokDepth {
    /// A question to answer. Low effort, few turns, no self-directed exploration.
    ///
    /// This is the default because a peer consult is usually "what do you think of X", and the
    /// old behaviour (effort `high` by omission, 20 turns, subagents and web search enabled) is
    /// what made a simple question take minutes.
    Fast,
    /// Let it off the leash. High effort, many turns, subagents and web search enabled.
    ///
    /// For "go read this subsystem and tell me what is wrong with it", where the exploring IS the
    /// value and the wall time is the price of it. Deliberately opt-in.
    Deep,
}

// FIND-GROK-04 note: an earlier attempt here was a thread_local `with_forced_fast` helper,
// intended to stop a daemon started with TRIUMVIRATE_GROK_DEPTH=deep from making every panel
// review a multi-minute turn. Codex and Antigravity independently called it unsound: tokio's
// work-stealing scheduler moves tasks across OS threads at every await point, and a panic
// inside the closure would leave the flag set on that worker with no Drop guard. It was
// removed rather than patched.
//
// The finding is NOT closed. Grok established the deeper reason it cannot be:
// `enforce_mandatory_peer_review` writes a review row and immediately auto-approves it, so no
// reviewer is ever spawned and there is no panel dispatch to isolate. Fixing the depth of a
// dispatch that does not happen would be theater.

/// The ONE lock over this file's process-global env, sitting next to the state it guards.
///
/// It is here rather than inside a test module because `TRIUMVIRATE_GROK_DEPTH` is read by
/// `grok_depth`, `grok_effort` and `grok_max_turns`, and TWO test modules mutate it. A
/// per-module lock serialises a module against itself and against nothing else, which is not a
/// lock, it is a comment. That exact mistake produced three separate intermittent failures
/// earlier in this work, each of which surfaced inside an unrelated test and read like a real
/// defect in whatever happened to run next.
///
/// The rule it encodes: a lock must live wherever the STATE lives, not wherever the test lives.
#[cfg(test)]
pub(crate) static GROK_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `TRIUMVIRATE_GROK_DEPTH=deep` (aliases: riff, wild, max) unleashes it. Anything else is Fast.
pub fn grok_depth() -> GrokDepth {
    match std::env::var("TRIUMVIRATE_GROK_DEPTH")
        .ok()
        .map(|v| v.trim().to_lowercase())
        .as_deref()
    {
        Some("deep") | Some("riff") | Some("wild") | Some("max") => GrokDepth::Deep,
        _ => GrokDepth::Fast,
    }
}

/// Reasoning effort. Defaults to `low`, NOT to grok's own default.
///
/// Grok, asked about its own latency, identified this as the single biggest lever: when `--effort`
/// is omitted, grok-4.6 uses **high**, and that reasoning time is the entire 5 to 12 second gap
/// between tools being ready (~0.6s) and the first token. Local startup was never the cost.
///
/// `low` is the right default for a peer consult, which is a question to answer rather than a
/// problem to grind on. Set TRIUMVIRATE_GROK_EFFORT=medium or high deliberately when a task
/// actually warrants it. Valid values for grok-4.6: low, medium, high, xhigh.
pub fn grok_effort() -> Option<String> {
    grok_effort_for(grok_depth())
}

/// As `grok_effort`, for a caller that has already decided the depth for THIS invocation.
///
/// An explicit `TRIUMVIRATE_GROK_EFFORT` still wins. A panel child is forced Fast to stop a
/// daemon started in Deep from making every review a multi-minute turn; it is not there to
/// override an operator who deliberately set an effort.
pub fn grok_effort_for(depth: GrokDepth) -> Option<String> {
    match std::env::var("TRIUMVIRATE_GROK_EFFORT").ok().map(|v| v.trim().to_string()) {
        Some(v) if v.eq_ignore_ascii_case("default") => None,
        Some(v) if !v.is_empty() => Some(v),
        // An explicit TRIUMVIRATE_GROK_EFFORT always wins; otherwise the depth profile decides.
        _ => Some(match depth {
            GrokDepth::Fast => "low",
            GrokDepth::Deep => "high",
        }
        .to_string()),
    }
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
    // ---- Hidden aliases and undocumented flags, named by Grok reviewing its own adapter
    // against the 1.0.13 binary. None appear in `--help`. Extras are appended AFTER the managed
    // flags and grok takes the LAST occurrence, so any one of these silently OVERRIDES
    // Triumvirate rather than being rejected. ----
    "--load",                  // hidden alias of --resume: last-wins resume of another session
    "--print",                 // hidden alias of -p/--single
    "--append-system-prompt",  // hidden alias of --rules: rewrites what the agent IS
    "--trust",                 // folder-trust for project MCP, LSP and hooks
    "--fs-write",              // client-side file writes
    "--fs-read",               // client-side file reads
    "--allow-cwd",             // cwd access in non-interactive mode
    "--leader",                // shared leader process
    "--no-leader",
    "--oauth",                 // auth path, and therefore which account is billed
    "--compaction-mode",       // changes what is persisted and the token spend
    "--storage-mode",          // where the session lives; invalid values are silently IGNORED
    "--no-memory",
    "--experimental-memory",
    "--memory-flush",          // runs a flush LLM: spend
    "--client-identifier",
    "--no-ask-user",
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
    build_grok_invocation_with_profile(
        bin,
        extra_args,
        prompt,
        cwd,
        session_id,
        resume,
        sandbox_override,
        None,
    )
}

/// As `build_grok_invocation_with_sandbox`, with an explicit depth for THIS invocation.
///
/// FIND-GROK-04, and the reason it stayed open. The first attempt was a thread_local
/// `with_forced_fast`; Codex and Antigravity independently called it unsound, because tokio's
/// work-stealing scheduler moves tasks across OS threads at every await point and a panic inside
/// the closure would leave the flag set on that worker with no Drop guard. It was deleted rather
/// than patched, and it must not come back.
///
/// Setting the variable on the child `Command` would not work either: the argv is built HERE, in
/// the parent, so a child env would arrive too late to change a single flag. The depth has to be
/// a parameter, which is the same shape `sandbox_override` already uses for the same reason.
///
/// `depth_override: None` means "use the daemon's configured depth".
#[allow(clippy::too_many_arguments)]
pub fn build_grok_invocation_with_profile(
    bin: &str,
    extra_args: &[String],
    prompt: &str,
    cwd: &str,
    session_id: Option<&str>,
    resume: bool,
    sandbox_override: Option<&str>,
    depth_override: Option<GrokDepth>,
) -> Result<GrokInvocation, String> {
    let depth = depth_override.unwrap_or_else(grok_depth);
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
    if let Some(effort) = grok_effort_for(depth) {
        args.push("--effort".to_string());
        args.push(effort);
    }

    args.push("--max-turns".to_string());
    args.push(grok_max_turns_for(depth).to_string());

    // Turn-burners, named by Grok when asked why reviews take minutes: it spawns subagents, writes
    // todo lists and searches the web, and each of those is another 5 to 12 second round trip that
    // produces no answer.
    //
    // In Deep mode they are exactly what you want: the exploring IS the value. So the profile
    // decides, and the two settings stay coherent instead of half-throttling it.
    if depth == GrokDepth::Fast {
        args.push("--no-subagents".to_string());
        args.push("--disable-web-search".to_string());
        args.push("--disallowed-tools".to_string());
        args.push("Agent,task,todo_write".to_string());
    }

    // Containment before approval, so the ordering reads as the policy it is: contain first,
    // then decide what may be approved inside that containment.
    // An override is a DEFAULT for this call site, not a policy that outranks the operator.
    //
    // Antigravity caught this: ABE passes Some("workspace") because a worker must write its
    // worktree. But an operator who deliberately sets TRIUMVIRATE_GROK_SANDBOX=off (say, inside
    // a container where sandbox-exec cannot run) had that intent silently overridden, and the
    // dispatch would then fail on a containment they explicitly disabled.
    //
    // So: an EXPLICIT operator setting always wins. The override only fills the gap when the
    // operator expressed no preference.
    let operator_set = std::env::var("TRIUMVIRATE_GROK_SANDBOX").is_ok();
    let profile = if operator_set {
        grok_sandbox_profile()
    } else {
        match sandbox_override {
            Some("off") | Some("") => None,
            Some(p) => Some(p.to_string()),
            None => grok_sandbox_profile(),
        }
    };
    if let Some(profile) = profile {
        args.push("--sandbox".to_string());
        args.push(profile);
    }

    // Approval policy is Triumvirate's, and it must be stated EXPLICITLY on every invocation.
    //
    // `~/.grok/config.toml` carries a `permission_mode`, and the operator's is currently
    // `always-approve`. Relying on the absence of `--always-approve` to mean "ask first" is
    // therefore wrong: the config already said yes. Grok found this reviewing its own adapter.
    // Passing `--permission-mode` unconditionally makes the flag, not the config file, decide.
    if grok_yolo_enabled() {
        args.push("--always-approve".to_string());
    } else {
        // `default` is grok's ask-before-acting mode. Combined with `--sandbox`, a consult can
        // neither auto-approve from config nor write outside its containment.
        args.push("--permission-mode".to_string());
        args.push("default".to_string());
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
    use std::sync::Mutex;

    /// Shared with `panel_depth_tests`, which mutates the SAME variables. See GROK_ENV_LOCK.
    fn env_lock() -> &'static Mutex<()> {
        &super::GROK_ENV_LOCK
    }

    /// Fast and Deep must differ in the ARGS that actually cost time, not just in an enum.
    ///
    /// Turns are the unit of wall clock: each one is a fresh round trip that re-ships the
    /// system prompt and every tool schema.
    ///
    /// This is what survives of FIND-GROK-04. The `with_forced_fast` helper it originally
    /// tested was removed as unsound (tokio moves tasks across threads at await points), and
    /// the finding is NOT closed: there is no panel dispatch to isolate, because mandatory peer
    /// review auto-approves without spawning a reviewer.
    ///
    /// Serialised with `env_lock()` like the rest of this module. The first version of these
    /// tests set the depth env var without it, which Grok flagged as a flake against the
    /// neighbouring depth tests, and which is the same parallel-env race this repo has now
    /// produced three times.
    ///
    /// RED IF: Fast stops capping turns below Deep, or effort stops following depth.
    #[test]
    fn fast_and_deep_differ_in_the_arguments_that_cost_time() {
        let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: serialised by env_lock; both values restored below.
        unsafe { std::env::set_var("TRIUMVIRATE_GROK_DEPTH", "deep") };
        let (deep_turns, deep_effort) = (grok_max_turns(), grok_effort());
        unsafe { std::env::set_var("TRIUMVIRATE_GROK_DEPTH", "fast") };
        let (fast_turns, fast_effort) = (grok_max_turns(), grok_effort());
        unsafe { std::env::remove_var("TRIUMVIRATE_GROK_DEPTH") };

        assert!(
            fast_turns < deep_turns,
            "Fast must cap turns below Deep ({fast_turns} vs {deep_turns})"
        );
        assert_ne!(fast_effort, deep_effort, "effort must follow depth");
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
            "TRIUMVIRATE_GROK_DEPTH",
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
        // Effort IS defaulted, deliberately. Omitting it means grok-4.6 uses `high`, and Grok
        // identified that reasoning time as the entire 5-12s gap before the first token.
        assert_eq!(value_after(&inv.args, "--effort").as_deref(), Some("low"),
            "omitting --effort silently selects grok's `high` default");
        assert_eq!(value_after(&inv.args, "--max-turns").as_deref(), Some("6"),
            "20 permitted a twenty-round-trip explore, which is where the minutes came from");

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

    /// An ABE worker asks for a writable sandbox, but an operator who explicitly disabled
    /// containment must not have that silently overridden. Found by Antigravity.
    #[test]
    fn u_b_21_an_explicit_operator_sandbox_setting_outranks_a_call_site_override() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        // No operator preference: the call site's default applies.
        let inv = build_grok_invocation_with_sandbox(
            "grok", &[], "p", "/tmp", None, false, Some("workspace"),
        ).unwrap();
        assert_eq!(value_after(&inv.args, "--sandbox").as_deref(), Some("workspace"));

        // Operator explicitly turned containment OFF. The override must not resurrect it.
        // SAFETY: guarded by env_lock.
        unsafe { std::env::set_var("TRIUMVIRATE_GROK_SANDBOX", "off") };
        let inv = build_grok_invocation_with_sandbox(
            "grok", &[], "p", "/tmp", None, false, Some("workspace"),
        ).unwrap();
        assert!(!inv.args.contains(&"--sandbox".to_string()),
            "an explicit operator `off` must outrank a call-site default");

        // Operator explicitly chose a DIFFERENT profile. Theirs wins too.
        // SAFETY: guarded by env_lock.
        unsafe { std::env::set_var("TRIUMVIRATE_GROK_SANDBOX", "strict") };
        let inv = build_grok_invocation_with_sandbox(
            "grok", &[], "p", "/tmp", None, false, Some("workspace"),
        ).unwrap();
        assert_eq!(value_after(&inv.args, "--sandbox").as_deref(), Some("strict"));
        clear_env();
    }

    /// Hidden aliases are the dangerous case: extras are appended AFTER the managed flags and
    /// grok takes the LAST occurrence, so an unlisted alias does not conflict, it WINS.
    /// Named by Grok reviewing its own adapter against the 1.0.13 binary; none are in `--help`.
    #[test]
    fn u_b_22_hidden_flag_aliases_cannot_override_managed_flags() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        // --load is a hidden alias of --resume; --append-system-prompt of --rules.
        for hostile in [
            "--load", "--print", "--append-system-prompt", "--trust", "--fs-write", "--fs-read",
            "--allow-cwd", "--leader", "--no-leader", "--oauth", "--compaction-mode",
            "--storage-mode", "--no-memory", "--experimental-memory", "--memory-flush",
            "--client-identifier", "--no-ask-user",
        ] {
            assert!(
                build_grok_invocation("grok", &[hostile.to_string(), "v".into()], "p", "/tmp", None, false).is_err(),
                "{hostile} is a hidden alias that would override a Triumvirate-managed flag"
            );
        }
    }

    /// grok's own config carries a `permission_mode`, and the operator's is `always-approve`.
    /// Relying on the ABSENCE of `--always-approve` to mean "ask first" was therefore wrong: the
    /// config had already said yes. Approval policy must be stated explicitly every time.
    #[test]
    fn u_b_23_approval_policy_is_explicit_not_inherited_from_config() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let inv = build(None, false).unwrap();
        assert_eq!(
            value_after(&inv.args, "--permission-mode").as_deref(),
            Some("default"),
            "a consult must pass its approval mode explicitly so config cannot widen it"
        );
        assert!(!inv.args.contains(&"--always-approve".to_string()));

        // Yolo is the deliberate opt-in, and then the explicit mode is redundant.
        // SAFETY: guarded by env_lock.
        unsafe { std::env::set_var("TRIUMVIRATE_GROK_YOLO", "1") };
        let inv = build(None, false).unwrap();
        assert!(inv.args.contains(&"--always-approve".to_string()));
        assert!(!inv.args.contains(&"--permission-mode".to_string()));
        clear_env();
    }

    /// Grok canonicalizes cwd, Triumvirate does not, and Grok itself flagged the consequence:
    /// `/tmp` and `/private/tmp` are different worker keys but the SAME on-disk session folder.
    /// This asserts the builder passes cwd through verbatim so the mismatch stays visible at one
    /// layer rather than being half-normalized in two.
    #[test]
    fn u_b_24_cwd_is_passed_through_verbatim_not_silently_normalized() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        for cwd in ["/tmp", "/private/tmp", "/tmp/", "."] {
            let inv = build_grok_invocation("grok", &[], "p", cwd, None, false).unwrap();
            assert_eq!(
                value_after(&inv.args, "--cwd").as_deref(),
                Some(cwd),
                "cwd must reach grok exactly as given; normalizing here would hide the mismatch"
            );
        }
    }

    /// Fast and Deep must be COHERENT profiles, not a scatter of knobs. Capping turns while
    /// leaving effort high still pays the reasoning tax; raising effort while forbidding subagents
    /// still cannot explore. Each setting is asserted against its profile so they cannot drift
    /// into a half-throttled state that is slow AND shallow.
    #[test]
    fn u_b_25_depth_profiles_are_coherent() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_env();

        // FAST: a question to answer.
        let inv = build(None, false).unwrap();
        assert_eq!(value_after(&inv.args, "--effort").as_deref(), Some("low"));
        assert_eq!(value_after(&inv.args, "--max-turns").as_deref(), Some("6"));
        assert!(inv.args.contains(&"--no-subagents".to_string()));
        assert!(inv.args.contains(&"--disable-web-search".to_string()));

        // DEEP: let it off the leash.
        // SAFETY: guarded by env_lock.
        unsafe { std::env::set_var("TRIUMVIRATE_GROK_DEPTH", "deep") };
        let inv = build(None, false).unwrap();
        assert_eq!(value_after(&inv.args, "--effort").as_deref(), Some("high"),
            "deep mode must not still be throttled to low effort");
        assert_eq!(value_after(&inv.args, "--max-turns").as_deref(), Some("30"));
        assert!(!inv.args.contains(&"--no-subagents".to_string()),
            "deep mode must be allowed to spawn subagents; exploring is the point");
        assert!(!inv.args.contains(&"--disable-web-search".to_string()));
        assert!(!inv.args.contains(&"--disallowed-tools".to_string()));

        // Aliases, because nobody remembers the exact word.
        for alias in ["riff", "wild", "max", "DEEP"] {
            // SAFETY: guarded by env_lock.
            unsafe { std::env::set_var("TRIUMVIRATE_GROK_DEPTH", alias) };
            assert_eq!(grok_depth(), GrokDepth::Deep, "alias {alias}");
        }

        // An explicit effort ALWAYS wins over the profile, in either direction.
        // SAFETY: guarded by env_lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GROK_DEPTH", "deep");
            std::env::set_var("TRIUMVIRATE_GROK_EFFORT", "low");
        }
        assert_eq!(value_after(&build(None, false).unwrap().args, "--effort").as_deref(), Some("low"),
            "an explicit operator setting outranks the profile");
        clear_env();
    }

}



/// FIND-GROK-04: a panel seat is Fast even when the daemon is Deep.
///
/// The finding stayed open for two reasons, both worth keeping. First, the original fix was a
/// thread_local `with_forced_fast`, which Codex and Antigravity independently called unsound:
/// tokio's work-stealing scheduler moves tasks across OS threads at every await point, and a
/// panic inside the closure would leave the flag set on that worker with no Drop guard. Second,
/// and more fundamentally, Grok pointed out there was no panel dispatch to isolate at all,
/// because mandatory review auto-approved without spawning anything. Both are now fixed.
#[cfg(test)]
mod panel_depth_tests {
    use super::*;

    /// The SAME lock `mod tests` uses. A private one here would serialise these four tests
    /// against each other and not against the eight in `mod tests` that set the same variable.
    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        super::GROK_ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn flag_value(args: &[String], flag: &str) -> Option<String> {
        args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
    }

    /// The headline. A daemon running Deep must still build a Fast panel child.
    /// RED IF: the depth override is dropped, or the builder goes back to reading the env.
    #[test]
    fn u_gd_01_a_panel_child_is_fast_on_a_deep_daemon() {
        let _g = env_guard();
        // SAFETY: held under the module lock, removed at the end.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GROK_DEPTH", "deep");
            std::env::remove_var("TRIUMVIRATE_GROK_EFFORT");
            std::env::remove_var("TRIUMVIRATE_GROK_MAX_TURNS");
        }
        assert_eq!(grok_depth(), GrokDepth::Deep, "the fixture must actually be Deep");

        let panel = build_grok_invocation_with_profile(
            "grok", &[], "review this", "/tmp", None, false, None, Some(GrokDepth::Fast),
        )
        .expect("panel invocation");

        assert_eq!(flag_value(&panel.args, "--effort").as_deref(), Some("low"));
        assert_eq!(flag_value(&panel.args, "--max-turns").as_deref(), Some("6"));
        assert!(
            panel.args.iter().any(|a| a == "--no-subagents"),
            "the Fast turn-burner suppression must apply to the panel child too"
        );
        assert!(panel.args.iter().any(|a| a == "--disable-web-search"));

        unsafe { std::env::remove_var("TRIUMVIRATE_GROK_DEPTH") };
    }

    /// The control, in the SAME process state. Without it the test above would pass even if the
    /// daemon env were being ignored entirely, which would mean Deep no longer works at all.
    /// RED IF: the override leaks into ordinary consults.
    #[test]
    fn u_gd_02_an_ordinary_consult_on_the_same_daemon_is_still_deep() {
        let _g = env_guard();
        unsafe {
            std::env::set_var("TRIUMVIRATE_GROK_DEPTH", "deep");
            std::env::remove_var("TRIUMVIRATE_GROK_EFFORT");
            std::env::remove_var("TRIUMVIRATE_GROK_MAX_TURNS");
        }

        let consult =
            build_grok_invocation("grok", &[], "a question", "/tmp", None, false).expect("consult");

        assert_eq!(flag_value(&consult.args, "--effort").as_deref(), Some("high"));
        assert_eq!(flag_value(&consult.args, "--max-turns").as_deref(), Some("30"));
        assert!(
            !consult.args.iter().any(|a| a == "--no-subagents"),
            "Deep must keep the exploring that is the whole point of it"
        );

        unsafe { std::env::remove_var("TRIUMVIRATE_GROK_DEPTH") };
    }

    /// An operator who set an effort deliberately outranks the panel default. The override is
    /// there to stop a Deep daemon making every review multi-minute, not to overrule a person.
    /// RED IF: the forced depth starts ignoring TRIUMVIRATE_GROK_EFFORT.
    #[test]
    fn u_gd_03_an_explicit_operator_effort_still_wins() {
        let _g = env_guard();
        unsafe {
            std::env::set_var("TRIUMVIRATE_GROK_DEPTH", "deep");
            std::env::set_var("TRIUMVIRATE_GROK_EFFORT", "medium");
        }

        let panel = build_grok_invocation_with_profile(
            "grok", &[], "review this", "/tmp", None, false, None, Some(GrokDepth::Fast),
        )
        .expect("panel invocation");
        assert_eq!(flag_value(&panel.args, "--effort").as_deref(), Some("medium"));

        unsafe {
            std::env::remove_var("TRIUMVIRATE_GROK_EFFORT");
            std::env::remove_var("TRIUMVIRATE_GROK_DEPTH");
        }
    }

    /// `None` must mean "whatever the daemon is configured for", so every existing caller keeps
    /// its behaviour. A default of Fast here would silently throttle ABE workers and consults.
    /// RED IF: the None arm stops falling through to grok_depth().
    #[test]
    fn u_gd_04_no_override_means_the_daemon_setting() {
        let _g = env_guard();
        unsafe {
            std::env::set_var("TRIUMVIRATE_GROK_DEPTH", "deep");
            std::env::remove_var("TRIUMVIRATE_GROK_EFFORT");
            std::env::remove_var("TRIUMVIRATE_GROK_MAX_TURNS");
        }

        let inv = build_grok_invocation_with_profile(
            "grok", &[], "hi", "/tmp", None, false, None, None,
        )
        .expect("invocation");
        assert_eq!(flag_value(&inv.args, "--max-turns").as_deref(), Some("30"));

        unsafe { std::env::remove_var("TRIUMVIRATE_GROK_DEPTH") };
    }
}
