//! Seat containment (Seven Primes FEAT-005, REQ-030, REQ-031, REQ-033, REQ-035): seat homes and their daemon-written
//! files, the child environment (scrub, then add), the macOS seatbelt profile, and the post-run repo audit.
//! BACKEND_STRUCTURE.md section 5 is the contract.
//!
//! T-003 scaffold: every public function returns `Err("not implemented: T-104")` until T-104 replaces it. The public
//! signatures and the shared types are final; Wave 1 tasks change bodies, not shapes.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::seat_profile::SeatProfile;

/// One read-only dispatch to a new seat, as the per-seat builders (kimi.rs, mistral.rs, qwen.rs) receive it.
#[derive(Debug, Clone)]
pub struct SeatRequest<'a> {
    pub prompt: &'a str,
    /// The repo the seat reads (never written: REQ-035).
    pub cwd: &'a Path,
    /// `$TRIUMVIRATE_HOME/homes/<seat>`, also the child's HOME (REQ-030).
    pub seat_home: &'a Path,
    /// Per-call directory the CLI may write its own output into (Mistral export.json).
    pub output_dir: &'a Path,
    pub model: Option<&'a str>,
    /// The vendor session id to resume, or `None` for a fresh session.
    pub resume_session_id: Option<&'a str>,
    /// Operator extra args; each builder rejects its forbidden list (BACKEND_STRUCTURE section 6).
    pub extra_args: &'a [String],
    pub max_turns: u32,
    pub time_limit: Duration,
}

/// What a builder returns: the program, its argv, and the env changes the runner applies on top of the scrubbed env.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatInvocation {
    pub program: String,
    pub args: Vec<String>,
    /// Applied after `env_remove`, so a value here always wins (Mistral `VIBE_CLI=python`).
    pub env_add: Vec<(String, String)>,
    /// Exact names or `PREFIX*` patterns removed from the inherited env.
    pub env_remove: Vec<String>,
    pub cwd: PathBuf,
}

/// The tracked and untracked state of a repo before or after a run (REQ-035).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoSnapshot {
    /// Path -> content hash (or a marker for a deletion), from `git status --porcelain=v1 -z --untracked-files=all`.
    pub entries: std::collections::BTreeMap<String, String>,
}

/// `$TRIUMVIRATE_HOME/homes/<seat>`.
pub fn seat_home(triumvirate_home: &Path, seat: &str) -> Result<PathBuf, String> {
    let _ = (triumvirate_home, seat);
    Err("not implemented: T-104".to_string())
}

/// Write the daemon-owned files into a seat home: Kimi ro-agent.md (0444), Qwen settings.json, Mistral config.toml.
pub fn write_seat_files(seat_home: &Path, profile: &SeatProfile) -> Result<(), String> {
    let _ = (seat_home, profile);
    Err("not implemented: T-104".to_string())
}

/// The child env: `base` minus the profile's `env_remove`, with HOME and the vendor home var set to the seat home, then
/// `env_add` applied last.
pub fn child_env(
    base: &[(String, String)],
    profile: &SeatProfile,
    seat_home: &Path,
    env_add: &[(String, String)],
) -> Result<Vec<(String, String)>, String> {
    let _ = (base, profile, seat_home, env_add);
    Err("not implemented: T-104".to_string())
}

/// The seatbelt profile for one dispatch (deny file-write*, then allow the seat home, output dir, tmp and dev paths).
pub fn render_seatbelt_profile(
    seat_home: &Path,
    output_dir: &Path,
    tmpdir: &Path,
) -> Result<String, String> {
    let _ = (seat_home, output_dir, tmpdir);
    Err("not implemented: T-104".to_string())
}

pub fn snapshot_repo(cwd: &Path) -> Result<RepoSnapshot, String> {
    let _ = cwd;
    Err("not implemented: T-104".to_string())
}

/// Paths that are NEW or CHANGED in `after` compared with `before`. A non-empty result fails the call as
/// `read_only_violation: <paths>`; nothing is reverted.
pub fn audit_repo(before: &RepoSnapshot, after: &RepoSnapshot) -> Result<Vec<String>, String> {
    let _ = (before, after);
    Err("not implemented: T-104".to_string())
}

/// `homes/<seat>/credentials.env` must exist with mode 0600.
pub fn check_credentials_file(path: &Path) -> Result<(), String> {
    let _ = path;
    Err("not implemented: T-104".to_string())
}

/// Names of metered-API keys present in an env that a seat must never see.
pub fn scan_stray_keys(env: &[(String, String)]) -> Result<Vec<String>, String> {
    let _ = env;
    Err("not implemented: T-104".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RED IF: a stub stops being wired to its public signature, or is mistaken for an implementation.
    #[test]
    fn t003_stubs_are_wired_and_refuse() {
        let p = Path::new("/nonexistent");
        let gemini = crate::seat_profile::seat_profile("gemini").expect("row");
        let errs = [
            seat_home(p, "kimi").unwrap_err(),
            write_seat_files(p, gemini).unwrap_err(),
            child_env(&[], gemini, p, &[]).unwrap_err(),
            render_seatbelt_profile(p, p, p).unwrap_err(),
            snapshot_repo(p).unwrap_err(),
            audit_repo(&RepoSnapshot::default(), &RepoSnapshot::default()).unwrap_err(),
            check_credentials_file(p).unwrap_err(),
            scan_stray_keys(&[]).unwrap_err(),
        ];
        for e in errs {
            assert_eq!(e, "not implemented: T-104");
        }
    }
}
