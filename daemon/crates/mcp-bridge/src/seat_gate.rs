//! The surface gate (Seven Primes FEAT-007, REQ-090, REQ-091): one named refusal code per refused call, decided before
//! anything spawns. BACKEND_STRUCTURE.md section 3 holds precedence and payloads.
//!
//! Amendment A-2 (Mike overrode D-02): Kimi and Qwen get exactly Mistral's launch surfaces, so the gate no longer
//! distinguishes interactive from unattended calls. `origin` and `tty` stay in the signature (callers already pass
//! them, and a future vendor-terms rule would need them), but at launch no (seat, surface, origin, tty) triple produces
//! `seat_surface_refused`.
//!
//! The decision is table-driven: a surface is allowed when it is in the seat's `SeatProfile.surfaces_allowed`.
//! Outside it, BlindValidate on a seat that cannot write a test file is `seat_cannot_write_here`, and anything else is
//! `seat_surface_deferred` (writer surfaces, peer-review rotation and prewarm ship in v1.1, F-4). Existing seats
//! (configured always, no verification) are allowed everywhere, which is today's behavior.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub use crate::seat_profile::Surface;
pub use shared_types::Origin;

use crate::seat_profile::{Configured, SeatProfile, SuspensionPolicy, VerifyPolicy};
use crate::seat_state::{Blocker, SeatAvailability};

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

fn refuse(code: RefusalCode, message: String) -> Result<(), Refusal> {
    Err(Refusal { code, message })
}

/// The decision for one seat profile and its state at this moment. Precedence (REQ-091): not configured, suspended,
/// unverified (not checked for DoctorLiveProbe, which is how a seat GETS verified), then the surface rule.
fn decide(
    profile: &SeatProfile,
    surface: Surface,
    state: &SeatAvailability,
) -> Result<(), Refusal> {
    let seat = profile.key;
    // Existing seats are not gated on files they never write (no doctor probe, no suspension classifier).
    let state = SeatAvailability {
        configured: state.configured,
        verified: profile.verify_policy == VerifyPolicy::NotRequired || state.verified,
        suspended: match profile.suspension_policy {
            SuspensionPolicy::Classified => state.suspended.clone(),
            SuspensionPolicy::NotClassified => None,
        },
    };
    match state.first_blocker() {
        Some(Blocker::NotConfigured) => {
            return refuse(
                RefusalCode::SeatNotConfigured,
                format!(
                    "seat_not_configured: {seat}: set TRIUMVIRATE_ENABLE_{}=1 and restart the daemon",
                    seat.to_ascii_uppercase()
                ),
            );
        }
        Some(Blocker::Suspended(reason)) => {
            return refuse(
                RefusalCode::SeatSuspended,
                format!("seat_suspended: {seat}: {reason} (run: triumvirate seat clear {seat})"),
            );
        }
        Some(Blocker::Unverified) if surface != Surface::DoctorLiveProbe => {
            return refuse(
                RefusalCode::SeatUnverified,
                format!("seat_unverified: {seat}: run triumvirate doctor --live in a terminal"),
            );
        }
        Some(Blocker::Unverified) | None => {}
    }
    if profile.surfaces_allowed.contains(&surface) {
        return Ok(());
    }
    if surface == Surface::BlindValidate && !profile.can_write_test_file {
        return refuse(
            RefusalCode::SeatCannotWriteHere,
            format!(
                "seat_cannot_write_here: {seat}: blind validators must write test files and {seat} is read-only"
            ),
        );
    }
    refuse(
        RefusalCode::SeatSurfaceDeferred,
        format!("seat_surface_deferred: {seat} on {surface:?} ships in v1.1"),
    )
}

/// The pure decision, given the seat's state at this moment. A seat with no SeatProfile row is not a seat at all.
pub fn seat_gate_with(
    seat: &str,
    surface: Surface,
    origin: Origin,
    tty: bool,
    state: &SeatAvailability,
) -> Result<(), Refusal> {
    // A-2: no surface rule depends on origin or tty at launch.
    let _ = (origin, tty);
    match crate::seat_profile::seat_profile(seat) {
        Some(profile) => decide(profile, surface, state),
        None => refuse(
            RefusalCode::SeatNotConfigured,
            format!("seat_not_configured: {seat}: no such seat"),
        ),
    }
}

/// Installed CLI versions, registered by the daemon at startup (T-203/T-205 fill it from `<cli> --version`). A seat
/// with no registered version cannot match a verified.json, so it reads as unverified: the gate fails closed.
fn installed_versions() -> &'static Mutex<HashMap<String, String>> {
    static VERSIONS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    VERSIONS.get_or_init(Mutex::default)
}

/// Record the installed CLI version for a seat (called at daemon start and after a CLI upgrade is detected).
pub fn register_installed_cli_version(seat: &str, version: &str) {
    installed_versions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(seat.to_string(), version.to_string());
}

/// The gate at a call site: reads the seat's state now (configured from its profile, availability files from
/// `$TRIUMVIRATE_HOME`), then decides.
pub fn seat_gate(seat: &str, surface: Surface, origin: Origin, tty: bool) -> Result<(), Refusal> {
    let Some(profile) = crate::seat_profile::seat_profile(seat) else {
        return seat_gate_with(
            seat,
            surface,
            origin,
            tty,
            &SeatAvailability {
                configured: false,
                verified: false,
                suspended: None,
            },
        );
    };
    let configured = match profile.configured {
        Configured::Always => true,
        Configured::EnvFlag(var) => std::env::var(var).is_ok_and(|v| v.trim() == "1"),
    };
    let version = installed_versions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(seat)
        .cloned();
    let state = crate::seat_state::triumvirate_home()
        .and_then(|home| {
            crate::seat_state::availability(&home, seat, configured, version.as_deref())
        })
        // Unreadable state fails closed: treated as unverified, never as available.
        .unwrap_or(SeatAvailability {
            configured,
            verified: false,
            suspended: None,
        });
    seat_gate_with(seat, surface, origin, tty, &state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seat_profile::{SeatProfile, seat_profile};

    /// The launch surfaces A-2 gives every new seat.
    const LAUNCH: &[Surface] = &[
        Surface::AskAgent,
        Surface::Session,
        Surface::Jury,
        Surface::DoctorLiveProbe,
    ];

    /// The new-seat rows as T-201 will add them, built here because T-201 has not landed: only the fields the gate
    /// reads differ from an existing row.
    fn new_seat(key: &'static str, var: &'static str) -> SeatProfile {
        SeatProfile {
            key,
            surfaces_allowed: LAUNCH,
            can_write_test_file: false,
            configured: Configured::EnvFlag(var),
            verify_policy: VerifyPolicy::DoctorLive,
            suspension_policy: SuspensionPolicy::Classified,
            ..seat_profile("codex").unwrap().clone()
        }
    }

    fn st(configured: bool, verified: bool, suspended: Option<&str>) -> SeatAvailability {
        SeatAvailability {
            configured,
            verified,
            suspended: suspended.map(String::from),
        }
    }

    fn msg(p: &SeatProfile, s: Surface, a: &SeatAvailability) -> Result<(), String> {
        decide(p, s, a).map_err(|r| {
            assert!(
                r.message.starts_with(r.code.as_str()),
                "payload must start with its code: {}",
                r.message
            );
            r.message
        })
    }

    /// Every (new seat, surface, origin, tty, state) combination, with the exact payload. origin and tty are walked
    /// on purpose: under A-2 they must not change any answer.
    /// RED IF: any new seat's answer changes for any surface or state, or origin/tty start mattering.
    #[test]
    fn launch_table_for_every_new_seat_surface_origin_tty_and_state() {
        let seats = [
            new_seat("kimi", "TRIUMVIRATE_ENABLE_KIMI"),
            new_seat("mistral", "TRIUMVIRATE_ENABLE_MISTRAL"),
            new_seat("qwen", "TRIUMVIRATE_ENABLE_QWEN"),
        ];
        let mut refused_codes = 0;
        for p in &seats {
            let k = p.key;
            for &surface in Surface::ALL {
                for _origin in [Origin::Interactive, Origin::Unattended] {
                    for _tty in [true, false] {
                        assert_eq!(
                            msg(p, surface, &st(false, true, None)),
                            Err(format!(
                                "seat_not_configured: {k}: set TRIUMVIRATE_ENABLE_{}=1 and restart the daemon",
                                k.to_uppercase()
                            ))
                        );
                        assert_eq!(
                            msg(p, surface, &st(true, false, Some("403 suspended"))),
                            Err(format!(
                                "seat_suspended: {k}: 403 suspended (run: triumvirate seat clear {k})"
                            ))
                        );
                        let unverified = msg(p, surface, &st(true, false, None));
                        let available = msg(p, surface, &st(true, true, None));
                        let surface_rule = match surface {
                            Surface::AskAgent
                            | Surface::Session
                            | Surface::Jury
                            | Surface::DoctorLiveProbe => Ok(()),
                            Surface::BlindValidate => Err(format!(
                                "seat_cannot_write_here: {k}: blind validators must write test files and {k} is read-only"
                            )),
                            other => Err(format!(
                                "seat_surface_deferred: {k} on {other:?} ships in v1.1"
                            )),
                        };
                        assert_eq!(available, surface_rule, "{k} {surface:?} available");
                        if surface == Surface::DoctorLiveProbe {
                            assert_eq!(unverified, Ok(()), "doctor is how a seat gets verified");
                        } else {
                            assert_eq!(
                                unverified,
                                Err(format!(
                                    "seat_unverified: {k}: run triumvirate doctor --live in a terminal"
                                ))
                            );
                        }
                        for r in [&unverified, &available] {
                            if r.as_ref()
                                .is_err_and(|m| m.starts_with("seat_surface_refused"))
                            {
                                refused_codes += 1;
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(
            refused_codes, 0,
            "A-2: no launch triple yields seat_surface_refused"
        );
    }

    /// Spot checks named in the plan (rewritten for A-2) through the public entry point.
    #[test]
    fn named_examples_through_seat_gate_with() {
        let ok = st(true, true, None);
        // Existing seats run everywhere, whatever the state files say (they write none).
        for seat in ["codex", "gemini", "grok", "claude", "deepseek"] {
            for &s in Surface::ALL {
                assert_eq!(
                    seat_gate_with(
                        seat,
                        s,
                        Origin::Unattended,
                        false,
                        &st(true, false, Some("x"))
                    ),
                    Ok(())
                );
            }
        }
        let e =
            seat_gate_with("zzz", Surface::AskAgent, Origin::Interactive, true, &ok).unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            (
                RefusalCode::SeatNotConfigured,
                "seat_not_configured: zzz: no such seat"
            )
        );
        assert_eq!(
            RefusalCode::SeatSurfaceDeferred.as_str(),
            "seat_surface_deferred"
        );
    }

    /// The wrapper fails closed: a seat whose CLI version was never registered reads as unverified even with a valid
    /// verified.json on disk, and registering the matching version makes it available.
    /// RED IF: the wrapper ever treats an unknown installed version as verified.
    #[test]
    fn wrapper_reads_state_files_and_fails_closed_without_a_version() {
        let home = tempfile::tempdir().unwrap();
        let rec = crate::seat_state::VerifiedRecord {
            cli_version: "9.9.9".into(),
            seat_home: "/x".into(),
            probed_at: "t".into(),
            served_model: None,
        };
        crate::seat_state::write_verified(home.path(), "gatewrapperprobe", &rec).unwrap();
        let state =
            |v| crate::seat_state::availability(home.path(), "gatewrapperprobe", true, v).unwrap();
        let p = new_seat("gatewrapperprobe", "TRIUMVIRATE_ENABLE_GATEWRAPPERPROBE");
        assert!(
            msg(&p, Surface::AskAgent, &state(None))
                .unwrap_err()
                .starts_with("seat_unverified")
        );
        assert_eq!(msg(&p, Surface::AskAgent, &state(Some("9.9.9"))), Ok(()));
        register_installed_cli_version("gatewrapperprobe", "9.9.9");
        assert_eq!(
            installed_versions()
                .lock()
                .unwrap()
                .get("gatewrapperprobe")
                .map(String::as_str),
            Some("9.9.9")
        );
    }
}
