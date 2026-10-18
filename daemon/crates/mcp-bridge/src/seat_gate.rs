//! The surface gate (Seven Primes FEAT-007, REQ-090, REQ-091): one named refusal code per refused call, decided before
//! anything spawns. BACKEND_STRUCTURE.md section 3 holds precedence and payloads; amendment A-2 (Mike overrode D-02)
//! gives Kimi and Qwen exactly Mistral's launch surfaces.
//!
//! T-003 scaffold: every public function returns `Err("not implemented: T-107")` until T-107 replaces it. The public
//! signatures and the shared types are final; Wave 1 tasks change bodies, not shapes.

pub use crate::seat_profile::Surface;
pub use shared_types::Origin;

use crate::seat_state::SeatAvailability;

/// Refusal codes in precedence order: the first that applies is the one returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalCode {
    SeatNotConfigured,
    SeatSuspended,
    SeatUnverified,
    SeatSurfaceRefused,
    SeatSurfaceDeferred,
    SeatCannotWriteHere,
}

impl RefusalCode {
    /// The code as it starts every payload string.
    pub fn as_str(self) -> &'static str {
        match self {
            RefusalCode::SeatNotConfigured => "seat_not_configured",
            RefusalCode::SeatSuspended => "seat_suspended",
            RefusalCode::SeatUnverified => "seat_unverified",
            RefusalCode::SeatSurfaceRefused => "seat_surface_refused",
            RefusalCode::SeatSurfaceDeferred => "seat_surface_deferred",
            RefusalCode::SeatCannotWriteHere => "seat_cannot_write_here",
        }
    }
}

/// A refused call: the code and the exact payload string the caller sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: RefusalCode,
    pub message: String,
}

/// The pure decision, given the seat's state at this moment.
pub fn seat_gate_with(
    seat: &str,
    surface: Surface,
    origin: Origin,
    tty: bool,
    state: &SeatAvailability,
) -> Result<(), Refusal> {
    let _ = (seat, surface, origin, tty, state);
    Err(Refusal {
        code: RefusalCode::SeatNotConfigured,
        message: "not implemented: T-107".to_string(),
    })
}

/// The gate at a call site: reads the seat's state now, then decides.
pub fn seat_gate(seat: &str, surface: Surface, origin: Origin, tty: bool) -> Result<(), Refusal> {
    let _ = (seat, surface, origin, tty);
    Err(Refusal {
        code: RefusalCode::SeatNotConfigured,
        message: "not implemented: T-107".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RED IF: a stub stops being wired to its public signature, or is mistaken for an implementation.
    #[test]
    fn t003_stubs_are_wired_and_refuse() {
        let state = SeatAvailability {
            configured: true,
            verified: true,
            suspended: None,
        };
        let a = seat_gate_with("kimi", Surface::AskAgent, Origin::Interactive, true, &state)
            .unwrap_err();
        let b = seat_gate("kimi", Surface::AskAgent, Origin::Interactive, true).unwrap_err();
        assert_eq!(a.message, "not implemented: T-107");
        assert_eq!(b.message, "not implemented: T-107");
        assert_eq!(
            RefusalCode::SeatSurfaceDeferred.as_str(),
            "seat_surface_deferred"
        );
    }
}
