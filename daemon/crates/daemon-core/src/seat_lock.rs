//! One process per seat, machine-wide (Seven Primes FEAT-010, REQ-053): an advisory flock on
//! `$TRIUMVIRATE_HOME/locks/<seat>`, released when the guard drops or the holding process dies. The fd pattern is
//! pid.rs's (lines 62-127).
//!
//! T-003 scaffold: every public function returns `Err("not implemented: T-105")` until T-105 replaces it. The public
//! signatures and the shared types are final; Wave 1 tasks change bodies, not shapes.

use std::path::Path;

/// Holds the seat's lock for as long as it lives.
#[derive(Debug)]
pub struct SeatLockGuard {
    _file: std::fs::File,
}

/// Block until this process holds the seat's lock.
pub fn acquire_seat_lock(triumvirate_home: &Path, seat: &str) -> Result<SeatLockGuard, String> {
    let _ = (triumvirate_home, seat);
    Err("not implemented: T-105".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RED IF: a stub stops being wired to its public signature, or is mistaken for an implementation.
    #[test]
    fn t003_stubs_are_wired_and_refuse() {
        assert_eq!(
            acquire_seat_lock(Path::new("/nonexistent"), "kimi").unwrap_err(),
            "not implemented: T-105"
        );
    }
}
