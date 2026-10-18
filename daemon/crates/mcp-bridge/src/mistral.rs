//! Mistral invocation builder (Seven Primes REQ-022). BACKEND_STRUCTURE.md section 6 holds the argv contract.
//!
//! research/02-mistral-vibe-contract.md is the source for every flag here:
//! - `--output streaming` emits one completed history entry per line; `--output-dir` makes the CLI write
//!   `export.json` whichever way the run ends, the authoritative record of session id, usage and outcome.
//! - `--agent plan` is a label, not a control (vendor issue #1267), so read-only is enforced by the
//!   `--enabled-tools` allowlist (programmatic mode disables every unlisted tool): `read_file`, `grep`, `todo`.
//! - `VIBE_CLI=python` in the child env, because the Rust CLI refuses `--resume` with `-p` (vendor issue #1239).
//! - Never `--trust` (a reviewed repo could run its own `.vibe/` hooks), `--worktree` (Triumvirate owns worktrees),
//!   `--continue` (most recent session, not ours), or `--auto-approve` / `--yolo` (writer flags).
//!
//! The model is not on the argv: the contract names no model flag, and the seat home's daemon-written config.toml
//! (`active_model`, T-104) selects it.

use crate::containment::{SeatInvocation, SeatRequest};

/// The CLI binary (research/02: PyPI package `mistral-vibe`, binary `vibe`).
pub const MISTRAL_PROGRAM: &str = "vibe";

/// The largest prompt, in bytes, passed on argv (REQ-024).
pub const MAX_PROMPT_BYTES: usize = 65536;

/// The read-only tool allowlist (research/02, round 1 additions: built-in tool modules).
pub const ENABLED_TOOLS: [&str; 3] = ["read_file", "grep", "todo"];

/// Operator extra args the builder refuses (BACKEND_STRUCTURE section 6, Mistral column). Each also matches its
/// `--flag=value` form, and a single-dash short flag also matches with its value attached (`-pfoo`).
pub const FORBIDDEN_EXTRA_ARGS: [&str; 16] = [
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
];

fn forbidden(arg: &str) -> bool {
    FORBIDDEN_EXTRA_ARGS.iter().any(|f| {
        arg == *f
            || arg
                .strip_prefix(f)
                .is_some_and(|rest| rest.starts_with('=') || !f.starts_with("--"))
    })
}

/// The read-only argv for one dispatch. Errors carry a code first, for example `prompt_too_large_for_mistral`.
pub fn build_mistral_invocation(req: &SeatRequest<'_>) -> Result<SeatInvocation, String> {
    let bytes = req.prompt.len();
    if bytes > MAX_PROMPT_BYTES {
        return Err(format!(
            "prompt_too_large_for_mistral: {bytes} bytes > {MAX_PROMPT_BYTES}"
        ));
    }
    if let Some(arg) = req.extra_args.iter().find(|a| forbidden(a)) {
        return Err(format!("forbidden_extra_arg_for_mistral: {arg}"));
    }

    let time_limit_secs = req.time_limit.as_secs().max(1);
    let mut args: Vec<String> = vec![
        "-p".into(),
        req.prompt.to_string(),
        "--output".into(),
        "streaming".into(),
        "--workdir".into(),
        req.cwd.to_string_lossy().into_owned(),
        "--output-dir".into(),
        req.output_dir.to_string_lossy().into_owned(),
        "--max-turns".into(),
        req.max_turns.to_string(),
        "--time-limit".into(),
        time_limit_secs.to_string(),
        "--agent".into(),
        "plan".into(),
    ];
    for tool in ENABLED_TOOLS {
        args.push("--enabled-tools".into());
        args.push(tool.into());
    }
    if let Some(id) = req.resume_session_id {
        args.push("--resume".into());
        args.push(id.to_string());
    }
    args.extend(req.extra_args.iter().cloned());

    Ok(SeatInvocation {
        program: MISTRAL_PROGRAM.to_string(),
        args,
        env_add: vec![("VIBE_CLI".to_string(), "python".to_string())],
        env_remove: Vec::new(),
        cwd: req.cwd.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::Duration;

    fn req<'a>(prompt: &'a str, extra: &'a [String], resume: Option<&'a str>) -> SeatRequest<'a> {
        SeatRequest {
            prompt,
            cwd: Path::new("/repo"),
            seat_home: Path::new("/th/homes/mistral"),
            output_dir: Path::new("/th/out/call-1"),
            model: Some("mistral-medium-3.5"),
            resume_session_id: resume,
            extra_args: extra,
            max_turns: 12,
            time_limit: Duration::from_secs(600),
        }
    }

    fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    }

    /// RED IF: the argv drifts from the BACKEND_STRUCTURE section 6 read-only contract, VIBE_CLI=python leaves the
    /// env (vendor issue #1239), the allowlist loses or gains a tool, or a writer flag appears.
    #[test]
    fn read_only_argv_matches_contract() {
        let inv = build_mistral_invocation(&req("review this", &[], None)).expect("builds");
        assert_eq!(inv.program, "vibe");
        assert_eq!(inv.cwd, Path::new("/repo"));
        assert_eq!(
            inv.env_add,
            vec![("VIBE_CLI".to_string(), "python".to_string())]
        );
        let a = &inv.args;
        assert_eq!(flag_value(a, "-p"), Some("review this"));
        assert_eq!(flag_value(a, "--output"), Some("streaming"));
        assert_eq!(flag_value(a, "--workdir"), Some("/repo"));
        assert_eq!(flag_value(a, "--output-dir"), Some("/th/out/call-1"));
        assert_eq!(flag_value(a, "--max-turns"), Some("12"));
        assert_eq!(flag_value(a, "--time-limit"), Some("600"));
        assert_eq!(flag_value(a, "--agent"), Some("plan"));
        let enabled: Vec<&str> = a
            .iter()
            .enumerate()
            .filter(|(_, x)| *x == "--enabled-tools")
            .map(|(i, _)| a[i + 1].as_str())
            .collect();
        assert_eq!(enabled, vec!["read_file", "grep", "todo"]);
        for writer in [
            "--trust",
            "--worktree",
            "--continue",
            "-c",
            "--auto-approve",
            "--yolo",
            "--resume",
            "--disabled-tools",
            "auto-approve",
            "write_file",
            "edit",
            "bash",
        ] {
            assert!(!a.iter().any(|x| x == writer), "{writer} in argv {a:?}");
        }
    }

    /// RED IF: a resume id is dropped, or resume switches to `--continue` (most recent session, not ours).
    #[test]
    fn resume_passes_the_session_id() {
        let inv = build_mistral_invocation(&req("p", &[], Some("sess-42"))).expect("builds");
        assert_eq!(flag_value(&inv.args, "--resume"), Some("sess-42"));
        assert!(!inv.args.iter().any(|x| x == "--continue"));
    }

    /// RED IF: the 64 KiB prompt cap moves, or an oversized prompt reaches argv instead of failing with its code.
    #[test]
    fn prompt_cap_is_64_kib() {
        let ok = "a".repeat(MAX_PROMPT_BYTES);
        assert!(build_mistral_invocation(&req(&ok, &[], None)).is_ok());
        let big = "a".repeat(MAX_PROMPT_BYTES + 1);
        assert_eq!(
            build_mistral_invocation(&req(&big, &[], None)).unwrap_err(),
            "prompt_too_large_for_mistral: 65537 bytes > 65536"
        );
    }

    /// RED IF: any forbidden extra arg (bare, `=value`, or short flag with attached value) gets through, or an
    /// allowed one is refused or dropped.
    #[test]
    fn forbidden_extra_args_are_refused() {
        for bad in [
            "--trust",
            "--worktree",
            "--continue",
            "-c",
            "--auto-approve",
            "--yolo",
            "--agent=auto-approve",
            "--enabled-tools=bash",
            "--disabled-tools",
            "--output=text",
            "--output-dir=/tmp",
            "--workdir",
            "--resume=x",
            "-p",
            "-pinjected",
            "--prompt",
            "--prompt-file=/etc/passwd",
        ] {
            let extra = vec![bad.to_string()];
            let err = build_mistral_invocation(&req("p", &extra, None)).unwrap_err();
            assert_eq!(err, format!("forbidden_extra_arg_for_mistral: {bad}"));
        }
        let extra = vec!["--max-price".to_string(), "0.50".to_string()];
        let inv = build_mistral_invocation(&req("p", &extra, None)).expect("allowed");
        assert_eq!(flag_value(&inv.args, "--max-price"), Some("0.50"));
        let extra = vec!["--output-format".to_string()];
        assert!(build_mistral_invocation(&req("p", &extra, None)).is_ok());
    }
}
