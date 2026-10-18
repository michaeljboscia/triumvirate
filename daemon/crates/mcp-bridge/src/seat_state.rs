//! Seat availability (Seven Primes FEAT-006, REQ-091): available = configured AND verified AND NOT suspended.
//! `$TRIUMVIRATE_HOME/seats/<seat>/verified.json` and `suspended.json` are read per call, never cached
//! (BACKEND_STRUCTURE.md section 4).
//!
//! T-003 scaffold: every public function returns `Err("not implemented: T-106")` until T-106 replaces it. The public
//! signatures and the shared types are final; Wave 1 tasks change bodies, not shapes.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// verified.json, written atomically by a passing `doctor --live`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedRecord {
    pub cli_version: String,
    pub seat_home: String,
    pub probed_at: String,
    pub served_model: Option<String>,
}

/// One seat's state at the moment of a dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatAvailability {
    pub configured: bool,
    pub verified: bool,
    /// The suspension reason, when suspended.json exists.
    pub suspended: Option<String>,
}

impl SeatAvailability {
    pub fn available(&self) -> bool {
        self.configured && self.verified && self.suspended.is_none()
    }
}

/// `TRIUMVIRATE_ENABLE_<SEAT>=1`, read at daemon start.
pub fn is_configured(seat: &str) -> Result<bool, String> {
    let _ = seat;
    Err("not implemented: T-106".to_string())
}

/// Read verified.json and suspended.json now. A verified.json whose `cli_version` differs from `installed_cli_version`
/// counts as absent.
pub fn availability(
    triumvirate_home: &Path,
    seat: &str,
    configured: bool,
    installed_cli_version: Option<&str>,
) -> Result<SeatAvailability, String> {
    let _ = (triumvirate_home, seat, configured, installed_cli_version);
    Err("not implemented: T-106".to_string())
}

pub fn write_verified(
    triumvirate_home: &Path,
    seat: &str,
    record: &VerifiedRecord,
) -> Result<(), String> {
    let _ = (triumvirate_home, seat, record);
    Err("not implemented: T-106".to_string())
}

/// Called when a probe spawned and then failed (credential, billing, served model, containment).
pub fn delete_verified(triumvirate_home: &Path, seat: &str) -> Result<(), String> {
    let _ = (triumvirate_home, seat);
    Err("not implemented: T-106".to_string())
}

pub fn write_suspended(triumvirate_home: &Path, seat: &str, reason: &str) -> Result<(), String> {
    let _ = (triumvirate_home, seat, reason);
    Err("not implemented: T-106".to_string())
}

/// `triumvirate seat clear <seat>`: the only way suspended.json is removed.
pub fn clear(triumvirate_home: &Path, seat: &str) -> Result<(), String> {
    let _ = (triumvirate_home, seat);
    Err("not implemented: T-106".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RED IF: a stub stops being wired to its public signature, or is mistaken for an implementation.
    #[test]
    fn t003_stubs_are_wired_and_refuse() {
        let p = Path::new("/nonexistent");
        let rec = VerifiedRecord {
            cli_version: "0".into(),
            seat_home: "/x".into(),
            probed_at: "t".into(),
            served_model: None,
        };
        let errs = [
            is_configured("kimi").unwrap_err(),
            availability(p, "kimi", true, None).unwrap_err(),
            write_verified(p, "kimi", &rec).unwrap_err(),
            delete_verified(p, "kimi").unwrap_err(),
            write_suspended(p, "kimi", "r").unwrap_err(),
            clear(p, "kimi").unwrap_err(),
        ];
        for e in errs {
            assert_eq!(e, "not implemented: T-106");
        }
        // The availability rule itself is final, not a stub.
        let a = |c, v, s: Option<&str>| SeatAvailability {
            configured: c,
            verified: v,
            suspended: s.map(String::from),
        };
        assert!(a(true, true, None).available());
        assert!(
            !a(false, true, None).available()
                && !a(true, false, None).available()
                && !a(true, true, Some("r")).available()
        );
    }
}
