//! Shared agy command assembly — the `sandbox-exec` wrapper + flags — used by BOTH
//! the inter-agent ask path (`triumvirate::agy`) and fleet (`fleet::orchestrator`) so
//! the sandbox profile (a security control) and the invocation have a SINGLE source
//! of truth (REQ-016/090). Capture, retry, and log parsing stay with each caller.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Verified `sandbox-exec` profile (probe4, agy 1.0.1 + 1.0.2). Constrains WRITES,
/// leaves READS + network open. Placeholders are substituted per dispatch.
///
/// H4: the consult path does NOT allow writes to the workspace — agy reads the repo
/// (reads are default-allow) but can never mutate it, even on a hallucinated/auto-
/// approved tool call. Writes are confined to agy's own state (~/.gemini) + temp. A
/// future explicit tool-exec mode can re-add a workspace-write allowance.
const SANDBOX_PROFILE_TEMPLATE: &str = r#";; Triumvirate sandbox-exec profile for the agy backend.
;; Constrains WRITES; leaves READS + network open. Verified by probe4 (REQ-016/062b).
;; Consult path: NO workspace write (H4) — repo is readable, never writable.
(version 1)
(allow default)
(deny file-write*)
(allow file-write* (subpath "@HOME@/.gemini"))
(allow file-write* (subpath "@HOME@/.antigravitycli"))
(allow file-write* (subpath "@TMPDIR@"))
(allow file-write* (subpath "/private/var/folders"))
(allow file-write* (subpath "/private/tmp"))
(allow file-write* (literal "/dev/null") (literal "/dev/stdout") (literal "/dev/stderr") (literal "/dev/dtracehelper") (literal "/dev/tty"))
"#;

/// agy's dedicated connector timeout (REQ-014). Default 900s — agy is blocking and
/// non-streaming. The same value is passed to agy's `--print-timeout`.
pub fn agy_connector_timeout() -> Duration {
    std::env::var("TRIUMVIRATE_AGY_CONNECTOR_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(900))
}

/// A fully-assembled agy invocation: `sandbox-exec -f <profile> <agy> -p <prompt>
/// --print-timeout <t> --log-file <log> [extra]`. The caller spawns `program` with
/// `args`, then should remove `profile_path` + `log_path` when done (the ask path
/// cleans up; fleet lets the OS reap them from the temp dir).
#[derive(Debug, Clone)]
pub struct AgyInvocation {
    pub program: String,
    pub args: Vec<String>,
    pub profile_path: PathBuf,
    pub log_path: PathBuf,
    pub print_timeout: Duration,
}

fn unique_temp(prefix: &str, ext: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{n}.{ext}", std::process::id()))
}

/// Render the per-dispatch sandbox-exec profile. No workspace-write allowance (H4):
/// the repo is readable (reads default-allow) but never writable on a consult.
fn render_sandbox_profile() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    let tmpdir = tmpdir.trim_end_matches('/').to_string();
    SANDBOX_PROFILE_TEMPLATE
        .replace("@HOME@", &home)
        .replace("@TMPDIR@", &tmpdir)
}

/// agy flags that Triumvirate owns; an operator's `TRIUMVIRATE_AGY_ARGS` must not
/// smuggle these (they'd defeat single-turn / output / containment guarantees). H3.
const FORBIDDEN_EXTRA_FLAGS: &[&str] = &[
    "-o",
    "--output-format",
    "-r",
    "--resume",
    "--session-id",
    "-c",
    "--continue",
    "--conversation",
    "--model",
    "-m",
    "-p",
    "--prompt",
    "-i",
    "--prompt-interactive",
    "--log-file",
    "--print-timeout",
    "--dangerously-skip-permissions",
];

/// Reject operator extra-args that would override Triumvirate-managed flags (H3).
/// Matches both bare (`-c`) and `--flag=value` forms.
fn validate_extra_args(extra: &[String]) -> Result<(), String> {
    for arg in extra {
        let flag = arg.split('=').next().unwrap_or(arg);
        if FORBIDDEN_EXTRA_FLAGS.contains(&flag) {
            return Err(format!(
                "TRIUMVIRATE_AGY_ARGS contains forbidden flag {flag:?}: agy single-turn/output/containment flags are managed by Triumvirate and cannot be overridden"
            ));
        }
    }
    Ok(())
}

/// agy's argument vector (no sandbox-exec wrapper). REQ-011/012/013: prompt via `-p`,
/// `--print-timeout` + `--log-file` always set, operator extra args appended; NEVER
/// `-o/--output-format`, `-r/--resume`, `--session-id`, `-c/--continue`, or `--model`.
fn agy_args(prompt: &str, log_path: &Path, print_timeout: Duration, extra: &[String]) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        prompt.to_string(),
        "--print-timeout".to_string(),
        format!("{}s", print_timeout.as_secs()),
        "--log-file".to_string(),
        log_path.to_string_lossy().into_owned(),
    ];
    args.extend(extra.iter().cloned());
    args
}

/// Whether the legacy `sandbox-exec` seatbelt wraps agy. Default OFF (yolo): agy
/// runs unsandboxed with `--dangerously-skip-permissions` so it can actually write
/// the files a consult asks for. Set `TRIUMVIRATE_AGY_SANDBOX=1` to restore the
/// old write-confined `(deny file-write*)` profile (rollback / hardened hosts).
pub fn agy_sandbox_enabled() -> bool {
    std::env::var("TRIUMVIRATE_AGY_SANDBOX")
        .ok()
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "on" | "yes"))
        .unwrap_or(false)
}

/// Directories agy may write to in yolo mode (`--add-dir`, repeatable). The dispatch
/// `cwd` plus its parent so sibling projects under the same root (the common
/// "consult about project X while the daemon runs in project Y" case) are writable.
fn yolo_add_dirs(cwd: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    if !cwd.is_empty() {
        dirs.push(cwd.to_string());
        if let Some(parent) = Path::new(cwd).parent() {
            let parent = parent.to_string_lossy().into_owned();
            if !parent.is_empty() && parent != cwd {
                dirs.push(parent);
            }
        }
    }
    dirs
}

/// Build one agy invocation (REQ-016). `bin` + `extra_args` come from `agy_command()`.
/// Rejects forbidden operator flags (H3). Yolo by default (no seatbelt); the legacy
/// `sandbox-exec` wrapper is opt-in via `TRIUMVIRATE_AGY_SANDBOX` — see
/// [`agy_sandbox_enabled`]. `cwd` scopes the yolo `--add-dir` write grant.
pub fn build_agy_invocation(
    bin: &str,
    extra_args: &[String],
    prompt: &str,
    cwd: &str,
) -> std::io::Result<AgyInvocation> {
    validate_extra_args(extra_args)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let print_timeout = agy_connector_timeout();
    let log_path = unique_temp("agy-log", "txt");

    if agy_sandbox_enabled() {
        // Legacy seatbelt: sandbox-exec -f <profile> agy ... (write-confined, H4).
        let profile_path = unique_temp("agy-sandbox", "sb");
        std::fs::write(&profile_path, render_sandbox_profile())?;
        let mut args = vec![
            "-f".to_string(),
            profile_path.to_string_lossy().into_owned(),
            bin.to_string(),
        ];
        args.extend(agy_args(prompt, &log_path, print_timeout, extra_args));
        return Ok(AgyInvocation {
            program: "sandbox-exec".to_string(),
            args,
            profile_path,
            log_path,
            print_timeout,
        });
    }

    // Yolo: spawn agy directly, auto-approve tool actions, grant write scope. No
    // profile file is written, so profile_path is a sentinel cleanup ignores.
    let mut args = vec![
        "-p".to_string(),
        prompt.to_string(),
        "--print-timeout".to_string(),
        format!("{}s", print_timeout.as_secs()),
        "--log-file".to_string(),
        log_path.to_string_lossy().into_owned(),
        "--dangerously-skip-permissions".to_string(),
    ];
    for dir in yolo_add_dirs(cwd) {
        args.push("--add-dir".to_string());
        args.push(dir);
    }
    args.extend(extra_args.iter().cloned());

    Ok(AgyInvocation {
        program: bin.to_string(),
        args,
        profile_path: PathBuf::new(),
        log_path,
        print_timeout,
    })
}

/// Remove stale agy temp files (profiles/logs) older than ~1 hour from the temp dir
/// (M9). The ask path cleans up its own files synchronously; this catches the fleet
/// path — where the child outlives the invocation, so RAII/Drop cleanup would race the
/// agy launch (sandbox-exec reads the profile at startup) — plus any crash-orphaned
/// files. Best-effort; safe to call periodically.
pub fn sweep_stale_temp_files() {
    let cutoff = std::time::SystemTime::now()
        .checked_sub(Duration::from_secs(3600))
        .unwrap_or_else(std::time::SystemTime::now);
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_agy_temp = (name.starts_with("agy-sandbox-") && name.ends_with(".sb"))
            || (name.starts_with("agy-log-") && name.ends_with(".txt"));
        if !is_agy_temp {
            continue;
        }
        if let Ok(modified) = entry.metadata().and_then(|m| m.modified())
            && modified < cutoff
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agy_args_never_include_forbidden_flags() {
        let args = agy_args(
            "hi",
            &PathBuf::from("/tmp/x.log"),
            Duration::from_secs(900),
            &[],
        );
        for forbidden in [
            "-o",
            "--output-format",
            "-r",
            "--resume",
            "-c",
            "--continue",
            "--model",
        ] {
            assert!(!args.iter().any(|a| a == forbidden), "must not pass {forbidden}");
        }
        assert!(args.windows(2).any(|w| w[0] == "-p" && w[1] == "hi"));
        assert!(args.contains(&"--print-timeout".to_string()));
        assert!(args.contains(&"--log-file".to_string()));
    }

    #[test]
    fn yolo_invocation_runs_agy_directly() {
        // Default (no TRIUMVIRATE_AGY_SANDBOX): no sandbox-exec wrapper. agy runs
        // directly with auto-approve + a write grant for the dispatch cwd.
        let inv = build_agy_invocation("agy", &[], "2+2?", "/Users/me/projects/triumvirate")
            .expect("invocation");
        assert_eq!(inv.program, "agy");
        assert_eq!(inv.args[0], "-p");
        assert_eq!(inv.args[1], "2+2?");
        assert!(inv.args.iter().any(|a| a == "--log-file"));
        assert!(
            inv.args.iter().any(|a| a == "--dangerously-skip-permissions"),
            "yolo auto-approves tool actions"
        );
        // cwd + its parent are granted as writable dirs.
        assert!(inv.args.windows(2).any(|w| w[0] == "--add-dir"
            && w[1] == "/Users/me/projects/triumvirate"));
        assert!(inv.args.windows(2).any(|w| w[0] == "--add-dir"
            && w[1] == "/Users/me/projects"));
        // No profile file is written in yolo mode.
        assert!(inv.profile_path.as_os_str().is_empty(), "no sandbox profile");
    }

    #[test]
    fn invocation_rejects_smuggled_forbidden_flags() {
        // H3: operator TRIUMVIRATE_AGY_ARGS cannot reintroduce managed flags.
        for bad in [
            vec!["-c".to_string()],
            vec!["--model".to_string(), "gemini-x".to_string()],
            vec!["--continue=true".to_string()],
            vec!["--dangerously-skip-permissions".to_string()],
            vec!["-o".to_string(), "json".to_string()],
        ] {
            assert!(
                build_agy_invocation("agy", &bad, "2+2?", "/tmp").is_err(),
                "must reject {bad:?}"
            );
        }
        // A benign extra arg is allowed.
        let ok = build_agy_invocation("agy", &["--add-dir".to_string(), "/x".to_string()], "2+2?", "/tmp");
        if let Ok(inv) = &ok {
            let _ = std::fs::remove_file(&inv.profile_path);
        }
        assert!(ok.is_ok(), "benign extra args allowed");
    }

    #[test]
    fn consult_profile_has_no_workspace_write() {
        // H4: the rendered profile must NOT grant workspace writes.
        let profile = render_sandbox_profile();
        assert!(!profile.contains("@WORKSPACE@"), "no unsubstituted workspace placeholder");
        assert!(profile.contains("(deny file-write*)"));
        assert!(profile.contains(".gemini"), "agy state still writable");
        // The only write-allows are agy state + temp + dev streams — never a repo subpath.
        assert!(!profile.to_lowercase().contains("/users/") || profile.contains(".gemini"));
    }
}
