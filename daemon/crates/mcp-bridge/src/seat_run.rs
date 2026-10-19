//! Seven Primes W-1: the command line ABE and fleet spawn for a new writer seat (kimi, mistral, qwen).
//!
//! ABE and fleet never assemble vendor argv for a new seat. They write the prompt into the worktree and spawn
//! `triumvirate seat-run`, which owns the containment (seat home, credential scrub, writer seatbelt, escape audit)
//! and commits on the seat's behalf.

use std::path::{Path, PathBuf};

/// True for the seats whose writer path is `seat-run`.
pub fn is_writer_seat(seat: &str) -> bool {
    matches!(seat, "kimi" | "mistral" | "qwen")
}

/// The triumvirate binary: `TRIUMVIRATE_SELF_BIN` (tests), else this process's own executable. ABE and both fleet
/// engines run inside the daemon (the Temporal fleet worker is a thread in it), so `current_exe` is the triumvirate
/// binary on every production path.
pub fn self_bin() -> Result<PathBuf, String> {
    if let Some(explicit) = std::env::var_os("TRIUMVIRATE_SELF_BIN").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(explicit));
    }
    std::env::current_exe().map_err(|e| format!("seat_run_self_bin_unavailable: {e}"))
}

/// seat-run's own `--timeout-secs` for a supervisor that kills it after `outer_secs`: strictly inside it, so seat-run
/// always kills the vendor's process group itself and is never killed mid-commit or left with an orphaned vendor.
/// `outer - 60`, or half the outer bound when that is 120 s or less (never below 1).
pub fn inner_timeout_secs(outer_secs: u64) -> u64 {
    if outer_secs <= 120 {
        (outer_secs / 2).max(1)
    } else {
        outer_secs - 60
    }
}

/// `<worktree>/.triumvirate/seat-prompt-<seat>.txt`.
pub fn prompt_file_path(worktree: &Path, seat: &str) -> PathBuf {
    worktree
        .join(".triumvirate")
        .join(format!("seat-prompt-{seat}.txt"))
}

/// Write the prompt file (create_new: an existing file is an error, never overwritten) and return the
/// `(program, argv)` that runs `triumvirate seat-run` for it. `surface` is `abe`, `fleet` or `fleet-temporal`.
pub fn seat_run_command(
    surface: &str,
    seat: &str,
    worktree: &Path,
    prompt: &str,
    commit_message: &str,
    timeout_secs: u64,
) -> Result<(String, Vec<String>), String> {
    use std::io::Write;
    if !is_writer_seat(seat) {
        return Err(format!("not_a_new_seat: {seat}"));
    }
    if !worktree.is_absolute() {
        return Err(format!(
            "seat_requires_worktree:{seat}: {} is not absolute",
            worktree.display()
        ));
    }
    let bin = self_bin()?;
    let path = prompt_file_path(worktree, seat);
    let dir = path.parent().unwrap_or(worktree);
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("seat_prompt_unwritable:{seat}: {}: {e}", dir.display()))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("seat_prompt_unwritable:{seat}: {}: {e}", path.display()))?;
    file.write_all(prompt.as_bytes())
        .map_err(|e| format!("seat_prompt_unwritable:{seat}: {}: {e}", path.display()))?;
    let args = vec![
        "seat-run".to_string(),
        "--seat".to_string(),
        seat.to_string(),
        "--surface".to_string(),
        surface.to_string(),
        "--worktree".to_string(),
        worktree.to_string_lossy().into_owned(),
        "--prompt-file".to_string(),
        path.to_string_lossy().into_owned(),
        "--commit-message".to_string(),
        commit_message.to_string(),
        "--timeout-secs".to_string(),
        timeout_secs.to_string(),
    ];
    Ok((bin.to_string_lossy().into_owned(), args))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RED IF: the inner deadline is not strictly inside the outer one, or the 60 s margin and the half rule move.
    #[test]
    fn inner_timeout_is_strictly_inside_the_outer() {
        assert_eq!(inner_timeout_secs(900), 840);
        assert_eq!(inner_timeout_secs(121), 61);
        assert_eq!(inner_timeout_secs(120), 60);
        assert_eq!(inner_timeout_secs(60), 30);
        for outer in 2..2000 {
            assert!(inner_timeout_secs(outer) < outer, "{outer}");
        }
    }

    /// RED IF: the prompt file is not create_new (an existing one is overwritten), lands anywhere but
    /// `.triumvirate/seat-prompt-<seat>.txt`, or the argv drifts from the seat-run flag contract.
    #[test]
    fn writes_the_prompt_once_and_returns_the_seat_run_argv() {
        let wt = tempfile::tempdir().unwrap();
        let (_, args) =
            seat_run_command("fleet", "qwen", wt.path(), "do it", "fleet(qwen): wt", 7).unwrap();
        let p = wt.path().join(".triumvirate/seat-prompt-qwen.txt");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "do it");
        assert_eq!(
            args,
            [
                "seat-run",
                "--seat",
                "qwen",
                "--surface",
                "fleet",
                "--worktree",
                &wt.path().to_string_lossy(),
                "--prompt-file",
                &p.to_string_lossy(),
                "--commit-message",
                "fleet(qwen): wt",
                "--timeout-secs",
                "7"
            ]
        );
        let err = seat_run_command("fleet", "qwen", wt.path(), "again", "m", 7).unwrap_err();
        assert!(err.starts_with("seat_prompt_unwritable:qwen:"), "{err}");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "do it");
        assert!(seat_run_command("fleet", "codex", wt.path(), "x", "m", 7).is_err());
    }
}
