//! Shared agy command assembly — the `sandbox-exec` wrapper + flags — used by BOTH
//! the inter-agent ask path (`triumvirate::agy`) and fleet (`fleet::orchestrator`) so
//! the sandbox profile (a security control) and the invocation have a SINGLE source
//! of truth (REQ-016/090). Capture, retry, and log parsing stay with each caller.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Verified `sandbox-exec` profile (probe4, agy 1.0.1 + 1.0.2). Constrains WRITES,
/// leaves READS + network open. Placeholders are substituted per dispatch.
const SANDBOX_PROFILE_TEMPLATE: &str = r#";; Triumvirate sandbox-exec profile for the agy backend.
;; Constrains WRITES; leaves READS + network open. Verified by probe4 (REQ-016/062b).
(version 1)
(allow default)
(deny file-write*)
(allow file-write* (subpath "@WORKSPACE@"))
(allow file-write* (subpath "@HOME@/.gemini"))
(allow file-write* (subpath "@HOME@/.antigravitycli"))
(allow file-write* (subpath "@TMPDIR@"))
(allow file-write* (subpath "/private/var/folders"))
(allow file-write* (subpath "/private/tmp"))
(allow file-write* (literal "/dev/null") (literal "/dev/stdout") (literal "/dev/stderr") (literal "/dev/dtracehelper") (literal "/dev/tty"))
@EXTRA_WRITABLE@
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

/// Render the per-dispatch sandbox-exec profile. The workspace is canonicalized so
/// subpath matching works under macOS symlinks (`/var` → `/private/var`).
fn render_sandbox_profile(cwd: &str) -> String {
    let workspace = std::fs::canonicalize(cwd)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| cwd.to_string());
    let home = std::env::var("HOME").unwrap_or_default();
    let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    let tmpdir = tmpdir.trim_end_matches('/').to_string();
    SANDBOX_PROFILE_TEMPLATE
        .replace("@WORKSPACE@", &workspace)
        .replace("@HOME@", &home)
        .replace("@TMPDIR@", &tmpdir)
        .replace("@EXTRA_WRITABLE@", "")
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

/// Build (and write the sandbox profile for) one agy invocation (REQ-016). `bin` +
/// `extra_args` come from `agy_command()`.
pub fn build_agy_invocation(
    bin: &str,
    extra_args: &[String],
    prompt: &str,
    cwd: &str,
) -> std::io::Result<AgyInvocation> {
    let print_timeout = agy_connector_timeout();
    let profile_path = unique_temp("agy-sandbox", "sb");
    std::fs::write(&profile_path, render_sandbox_profile(cwd))?;
    let log_path = unique_temp("agy-log", "txt");

    let mut args = vec![
        "-f".to_string(),
        profile_path.to_string_lossy().into_owned(),
        bin.to_string(),
    ];
    args.extend(agy_args(prompt, &log_path, print_timeout, extra_args));

    Ok(AgyInvocation {
        program: "sandbox-exec".to_string(),
        args,
        profile_path,
        log_path,
        print_timeout,
    })
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
    fn invocation_wraps_agy_in_sandbox_exec() {
        let inv = build_agy_invocation("agy", &[], "2+2?", "/tmp").expect("invocation");
        assert_eq!(inv.program, "sandbox-exec");
        // sandbox-exec -f <profile> agy -p 2+2? ...
        assert_eq!(inv.args[0], "-f");
        assert_eq!(inv.args[1], inv.profile_path.to_string_lossy());
        assert_eq!(inv.args[2], "agy");
        assert!(inv.args.iter().any(|a| a == "--log-file"));
        assert!(inv.profile_path.exists(), "profile written");
        let _ = std::fs::remove_file(&inv.profile_path);
    }
}
