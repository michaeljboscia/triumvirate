//! Seat availability (Seven Primes FEAT-006, REQ-091): available = configured AND verified AND NOT suspended.
//! `$TRIUMVIRATE_HOME/seats/<seat>/verified.json` and `suspended.json` are read per call, never cached
//! (BACKEND_STRUCTURE.md section 4), so a `doctor --live` pass or a `seat clear` takes effect on the next dispatch
//! without restarting the daemon.
//!
//! Every read fails CLOSED: an unreadable verified.json is "not verified", an unreadable suspended.json is still
//! "suspended", and an unknown installed CLI version cannot match a verified record. A seat is never made available
//! by a file the daemon could not understand.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// verified.json, written atomically by a passing `doctor --live`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedRecord {
    pub cli_version: String,
    pub seat_home: String,
    pub probed_at: String,
    pub served_model: Option<String>,
}

/// suspended.json, written when a provider response is classified as an account suspension (REQ-050).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SuspendedRecord {
    reason: String,
    at: String,
}

/// One seat's state at the moment of a dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatAvailability {
    pub configured: bool,
    pub verified: bool,
    /// The suspension reason, when suspended.json exists.
    pub suspended: Option<String>,
}

/// The first reason a seat is not available, in REQ-091 precedence: not configured, then suspended, then unverified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    NotConfigured,
    Suspended(String),
    Unverified,
}

impl SeatAvailability {
    pub fn available(&self) -> bool {
        self.configured && self.verified && self.suspended.is_none()
    }

    /// `None` when available. A suspension outranks a missing verification, so a suspended seat says "suspended" even
    /// after its verified.json was deleted by the failing probe.
    pub fn first_blocker(&self) -> Option<Blocker> {
        if !self.configured {
            Some(Blocker::NotConfigured)
        } else if let Some(reason) = &self.suspended {
            Some(Blocker::Suspended(reason.clone()))
        } else if !self.verified {
            Some(Blocker::Unverified)
        } else {
            None
        }
    }
}

/// `$TRIUMVIRATE_HOME`, else `~/.triumvirate` (the same rule as daemon-core's `triumvirate_home_dir`, which this crate
/// does not depend on).
pub fn triumvirate_home() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("TRIUMVIRATE_HOME") {
        return Ok(PathBuf::from(dir));
    }
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".triumvirate"))
        .map_err(|_| "seat_state: neither TRIUMVIRATE_HOME nor HOME is set".to_string())
}

/// A seat key becomes a directory name, so it must not be able to address anything outside seats/.
fn seat_dir(triumvirate_home: &Path, seat: &str) -> Result<PathBuf, String> {
    let ok = !seat.is_empty()
        && seat
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        return Err(format!(
            "invalid_seat_name: {seat:?} (lowercase ascii, digits and '-' only)"
        ));
    }
    Ok(triumvirate_home.join("seats").join(seat))
}

/// Write `bytes` to `path` atomically: a sibling temp file, then rename, so a reader never sees half a record.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("seat_state: {} has no parent", path.display()))?;
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("seat_state: cannot create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("record"),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes)
        .map_err(|e| format!("seat_state: cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("seat_state: cannot rename onto {}: {e}", path.display()))
}

fn remove_if_present(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("seat_state: cannot remove {}: {e}", path.display())),
    }
}

/// `TRIUMVIRATE_ENABLE_<SEAT>=1`, read at daemon start. Only "1" enables, so a typo cannot turn a seat on.
pub fn is_configured(seat: &str) -> Result<bool, String> {
    seat_dir(Path::new("/"), seat)?;
    let var = format!(
        "TRIUMVIRATE_ENABLE_{}",
        seat.to_ascii_uppercase().replace('-', "_")
    );
    Ok(std::env::var(var).is_ok_and(|v| v.trim() == "1"))
}

/// Read verified.json and suspended.json now. A verified.json whose `cli_version` differs from `installed_cli_version`
/// counts as absent, and so does any verified.json when the installed version is unknown.
pub fn availability(
    triumvirate_home: &Path,
    seat: &str,
    configured: bool,
    installed_cli_version: Option<&str>,
) -> Result<SeatAvailability, String> {
    let dir = seat_dir(triumvirate_home, seat)?;

    let verified = match std::fs::read(dir.join("verified.json")) {
        Err(_) => false,
        Ok(bytes) => match serde_json::from_slice::<VerifiedRecord>(&bytes) {
            Ok(rec) => installed_cli_version.is_some_and(|v| v == rec.cli_version),
            Err(e) => {
                tracing::warn!(seat, error = %e, "unreadable verified.json counts as not verified");
                false
            }
        },
    };

    let suspended = match std::fs::read(dir.join("suspended.json")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        // Present but unreadable, or unparseable: still suspended. Only `seat clear` lifts a suspension.
        Err(e) => Some(format!("suspended.json unreadable: {e}")),
        Ok(bytes) => Some(
            serde_json::from_slice::<SuspendedRecord>(&bytes)
                .map(|r| r.reason)
                .unwrap_or_else(|e| format!("suspended.json unparseable: {e}")),
        ),
    };

    Ok(SeatAvailability {
        configured,
        verified,
        suspended,
    })
}

pub fn write_verified(
    triumvirate_home: &Path,
    seat: &str,
    record: &VerifiedRecord,
) -> Result<(), String> {
    let path = seat_dir(triumvirate_home, seat)?.join("verified.json");
    let json = serde_json::to_vec_pretty(record).map_err(|e| format!("seat_state: {e}"))?;
    write_atomic(&path, &json)
}

/// Called when a probe spawned and then failed (credential, billing, served model, containment).
pub fn delete_verified(triumvirate_home: &Path, seat: &str) -> Result<(), String> {
    remove_if_present(&seat_dir(triumvirate_home, seat)?.join("verified.json"))
}

pub fn write_suspended(triumvirate_home: &Path, seat: &str, reason: &str) -> Result<(), String> {
    let path = seat_dir(triumvirate_home, seat)?.join("suspended.json");
    let rec = SuspendedRecord {
        reason: reason.to_string(),
        at: chrono::Utc::now().to_rfc3339(),
    };
    let json = serde_json::to_vec_pretty(&rec).map_err(|e| format!("seat_state: {e}"))?;
    write_atomic(&path, &json)
}

/// `triumvirate seat clear <seat>`: the only way suspended.json is removed.
pub fn clear(triumvirate_home: &Path, seat: &str) -> Result<(), String> {
    remove_if_present(&seat_dir(triumvirate_home, seat)?.join("suspended.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(version: &str) -> VerifiedRecord {
        VerifiedRecord {
            cli_version: version.into(),
            seat_home: "/th/homes/kimi".into(),
            probed_at: "2026-10-18T19:00:00Z".into(),
            served_model: None,
        }
    }

    /// State is read per call: a verified.json written by another process (here, plain fs::write, as doctor would)
    /// flips the very next read, with no reinit. Changing the installed version flips it back.
    /// RED IF: availability is cached, or the version check stops invalidating.
    #[test]
    fn verified_takes_effect_on_the_next_read_and_version_change_invalidates() {
        let home = tempfile::tempdir().unwrap();
        let read = |v| availability(home.path(), "kimi", true, v).unwrap();
        assert!(!read(Some("2.2.0")).available());

        let dir = home.path().join("seats/kimi");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("verified.json"),
            serde_json::to_vec(&rec("2.2.0")).unwrap(),
        )
        .unwrap();
        assert!(read(Some("2.2.0")).available());
        assert!(
            !read(Some("2.2.1")).verified,
            "a new CLI version voids the probe"
        );
        assert!(
            !read(None).verified,
            "an unknown installed version cannot match"
        );
    }

    /// Suspension outranks a missing verification, and only clear() lifts it.
    /// RED IF: precedence changes, or delete_verified/write_verified can lift a suspension.
    #[test]
    fn suspension_outranks_unverified_and_only_clear_removes_it() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        write_suspended(h, "qwen", "403 account suspended").unwrap();
        let a = availability(h, "qwen", true, Some("0.25.2")).unwrap();
        assert_eq!(
            a.first_blocker(),
            Some(Blocker::Suspended("403 account suspended".into()))
        );

        delete_verified(h, "qwen").unwrap();
        write_verified(h, "qwen", &rec("0.25.2")).unwrap();
        let a = availability(h, "qwen", true, Some("0.25.2")).unwrap();
        assert!(
            a.suspended.is_some() && !a.available(),
            "verification cannot lift a suspension"
        );

        clear(h, "qwen").unwrap();
        let a = availability(h, "qwen", true, Some("0.25.2")).unwrap();
        assert!(a.available() && a.first_blocker().is_none());
        clear(h, "qwen").unwrap();
    }

    /// RED IF: the precedence order of REQ-091 changes.
    #[test]
    fn blocker_precedence() {
        let a = |c, v, s: Option<&str>| SeatAvailability {
            configured: c,
            verified: v,
            suspended: s.map(String::from),
        };
        assert_eq!(
            a(false, false, Some("r")).first_blocker(),
            Some(Blocker::NotConfigured)
        );
        assert_eq!(
            a(true, false, Some("r")).first_blocker(),
            Some(Blocker::Suspended("r".into()))
        );
        assert_eq!(
            a(true, false, None).first_blocker(),
            Some(Blocker::Unverified)
        );
        assert_eq!(a(true, true, None).first_blocker(), None);
    }

    /// Unreadable state fails closed: garbage verified.json is not verified, garbage suspended.json is suspended.
    /// RED IF: a file the daemon cannot parse ever makes a seat available.
    #[test]
    fn unreadable_files_fail_closed() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("seats/mistral");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("verified.json"), b"{not json").unwrap();
        assert!(
            !availability(home.path(), "mistral", true, Some("2.26.1"))
                .unwrap()
                .verified
        );
        std::fs::write(
            dir.join("verified.json"),
            serde_json::to_vec(&rec("2.26.1")).unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join("suspended.json"), b"garbage").unwrap();
        let a = availability(home.path(), "mistral", true, Some("2.26.1")).unwrap();
        assert!(
            a.suspended
                .unwrap()
                .starts_with("suspended.json unparseable")
        );
    }

    /// Writes are atomic (no temp file is left behind) and the record round-trips.
    #[test]
    fn writes_are_atomic_and_round_trip() {
        let home = tempfile::tempdir().unwrap();
        write_verified(home.path(), "kimi", &rec("2.2.0")).unwrap();
        let dir = home.path().join("seats/kimi");
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["verified.json"]);
        let back: VerifiedRecord =
            serde_json::from_slice(&std::fs::read(dir.join("verified.json")).unwrap()).unwrap();
        assert_eq!(back, rec("2.2.0"));
    }

    /// RED IF: a seat name can address a path outside seats/, or a non-"1" value enables a seat.
    #[test]
    fn seat_names_are_validated_and_only_one_enables() {
        let home = tempfile::tempdir().unwrap();
        for bad in ["", "../x", "a/b", "Kimi"] {
            assert!(
                availability(home.path(), bad, true, None)
                    .unwrap_err()
                    .starts_with("invalid_seat_name:")
            );
        }
        // A seat name no other test uses, so this env key cannot race another test.
        let var = "TRIUMVIRATE_ENABLE_SEATSTATEPROBE";
        for (value, want) in [("1", true), ("true", false), ("0", false), (" 1 ", true)] {
            // SAFETY: this variable is read only by this test.
            unsafe { std::env::set_var(var, value) };
            assert_eq!(is_configured("seatstateprobe").unwrap(), want, "{value:?}");
        }
        unsafe { std::env::remove_var(var) };
        assert!(!is_configured("seatstateprobe").unwrap());
    }
}
