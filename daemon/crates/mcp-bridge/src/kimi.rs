//! Kimi invocation builder (Seven Primes REQ-021, REQ-024). BACKEND_STRUCTURE.md section 6 holds the argv contract;
//! the CLI behavior it encodes is from research/01-kimi-code-contract.md (Kimi Code CLI 2.2.0).
//!
//! The builder only shapes argv. It never spawns (T-205), never writes the seat home (T-104 writes ro-agent.md), and
//! leaves the env scrub to `containment::child_env`.

use crate::containment::{SeatInvocation, SeatRequest};

/// The binary name. research/01: "Binary name is still `kimi`."
pub const KIMI_PROGRAM: &str = "kimi";

/// The read-only agent file inside the seat home, written by T-104 (BACKEND_STRUCTURE section 5).
pub const RO_AGENT_FILE: &str = "ro-agent.md";

/// The prompt travels in argv (`-p <prompt>`), capped at 64 KiB of prompt text (REQ-024, `PromptTransport::ArgvMax64KiB`).
pub const KIMI_PROMPT_LIMIT_BYTES: usize = 65_536;

/// Operator extra args that would break containment or the stream contract (BACKEND_STRUCTURE section 6). research/01:
/// `--continue` picks the most recent session in cwd (cross-talk); `-p` cannot combine with `--yolo`/`--auto`/`--plan`;
/// `--agent`/`--agent-file` bind only at creation; `-r`/`--resume` are hidden aliases of `--session`.
pub const KIMI_FORBIDDEN_EXTRA_ARGS: &[&str] = &[
    "--continue",
    "-c",
    "--yolo",
    "-y",
    "--auto",
    "--plan",
    "--session",
    "-S",
    "-r",
    "--resume",
    "--output-format",
    "-p",
    "--prompt",
    "--agent",
    "--agent-file",
    "-m",
    "--model",
];

/// The read-only argv for one dispatch. Errors carry a code first, for example `prompt_too_large_for_kimi`.
///
/// Creation: `kimi -p <p> --output-format stream-json --agent-file <seat_home>/ro-agent.md [-m <model>] [extra...]`.
/// Resume: `kimi -p <p> --output-format stream-json --session <id> [-m <model>] [extra...]`, with no `--agent-file`,
/// because research/01 says `--agent`/`--agent-file` "cannot combine with `--session`" and the resumed session restores
/// its bound agent. There is no `--work-dir` flag in 2.2.0, so the runner sets the cwd (research/01).
pub fn build_kimi_invocation(req: &SeatRequest<'_>) -> Result<SeatInvocation, String> {
    let prompt_bytes = req.prompt.len();
    if prompt_bytes > KIMI_PROMPT_LIMIT_BYTES {
        return Err(format!(
            "prompt_too_large_for_kimi: {prompt_bytes} bytes > {KIMI_PROMPT_LIMIT_BYTES}"
        ));
    }
    for arg in req.extra_args {
        if is_forbidden(arg) {
            return Err(format!("forbidden_extra_arg_for_kimi: {arg}"));
        }
    }

    let mut args = vec![
        "-p".to_string(),
        req.prompt.to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
    ];
    match req.resume_session_id {
        Some(id) => {
            args.push("--session".to_string());
            args.push(id.to_string());
        }
        None => {
            args.push("--agent-file".to_string());
            args.push(
                req.seat_home
                    .join(RO_AGENT_FILE)
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    if let Some(model) = req.model {
        args.push("-m".to_string());
        args.push(model.to_string());
    }
    args.extend(req.extra_args.iter().cloned());

    Ok(SeatInvocation {
        program: KIMI_PROGRAM.to_string(),
        args,
        // research/01: "`KIMI_DISABLE_TELEMETRY=1` turns off vendor telemetry." KIMI_CODE_HOME and HOME are set by
        // `containment::child_env` from the seat home.
        env_add: vec![("KIMI_DISABLE_TELEMETRY".to_string(), "1".to_string())],
        env_remove: Vec::new(),
        cwd: req.cwd.to_path_buf(),
    })
}

/// True for a forbidden flag, alone or in `--flag=value` form.
fn is_forbidden(arg: &str) -> bool {
    let flag = arg.split_once('=').map_or(arg, |(f, _)| f);
    KIMI_FORBIDDEN_EXTRA_ARGS.contains(&flag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::Duration;

    fn req<'a>(
        prompt: &'a str,
        resume: Option<&'a str>,
        extra: &'a [String],
        model: Option<&'a str>,
    ) -> SeatRequest<'a> {
        SeatRequest {
            prompt,
            cwd: Path::new("/repo"),
            seat_home: Path::new("/th/homes/kimi"),
            output_dir: Path::new("/th/out/1"),
            model,
            resume_session_id: resume,
            extra_args: extra,
            max_turns: 8,
            time_limit: Duration::from_secs(60),
        }
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|w| w[0] == flag && w[1] == value)
    }

    /// RED IF: creation stops passing the read-only agent file, or drops stream-json, the prompt, or the model.
    #[test]
    fn creation_argv_carries_agent_file() {
        let inv =
            build_kimi_invocation(&req("hello", None, &[], Some("kimi-code/kimi-for-coding")))
                .expect("builds");
        assert_eq!(inv.program, "kimi");
        assert!(has_pair(&inv.args, "-p", "hello"));
        assert!(has_pair(&inv.args, "--output-format", "stream-json"));
        assert!(has_pair(
            &inv.args,
            "--agent-file",
            "/th/homes/kimi/ro-agent.md"
        ));
        assert!(has_pair(&inv.args, "-m", "kimi-code/kimi-for-coding"));
        assert!(!inv.args.iter().any(|a| a == "--session"));
        assert_eq!(inv.cwd, Path::new("/repo"));
    }

    /// RED IF: resume stops passing `--session <id>`, or passes `--agent-file` with it (research/01: cannot combine).
    #[test]
    fn resume_argv_has_session_and_no_agent_file() {
        let inv = build_kimi_invocation(&req("again", Some("sess-42"), &[], None)).expect("builds");
        assert!(has_pair(&inv.args, "--session", "sess-42"));
        assert!(!inv.args.iter().any(|a| a == "--agent-file"));
        assert!(!inv.args.iter().any(|a| a == "-m"));
    }

    /// RED IF: `--continue` (cross-talk, research/01) or any forbidden flag, plain or `=value`, slips through.
    #[test]
    fn forbidden_extra_args_are_rejected() {
        for bad in [
            "--continue",
            "-c",
            "--yolo",
            "--session=abc",
            "--agent-file",
            "-r",
        ] {
            let extra = vec![bad.to_string()];
            let err = build_kimi_invocation(&req("p", None, &extra, None)).unwrap_err();
            assert_eq!(err, format!("forbidden_extra_arg_for_kimi: {bad}"));
        }
        let ok = vec!["--add-dir".to_string(), "/other".to_string()];
        let inv =
            build_kimi_invocation(&req("p", None, &ok, None)).expect("allowed extra args pass");
        assert!(has_pair(&inv.args, "--add-dir", "/other"));
    }

    /// RED IF: the 64 KiB argv limit (REQ-024) is not enforced, or is enforced below the limit.
    #[test]
    fn prompt_over_64_kib_is_rejected() {
        let big = "x".repeat(65 * 1024);
        let err = build_kimi_invocation(&req(&big, None, &[], None)).unwrap_err();
        assert!(
            err.starts_with("prompt_too_large_for_kimi: 66560 bytes > 65536"),
            "{err}"
        );
        let at_limit = "x".repeat(KIMI_PROMPT_LIMIT_BYTES);
        assert!(build_kimi_invocation(&req(&at_limit, None, &[], None)).is_ok());
    }
}
