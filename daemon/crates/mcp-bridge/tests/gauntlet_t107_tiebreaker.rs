// GAUNTLET tiebreaker suite for T-107, written blind from the spec (no implementation read).
// SPEC_AMBIGUITY: The public API cannot inject overlapping surface predicates, so the surface-terminal tie-break cannot be isolated beyond the fixed launch table.

use mcp_bridge::seat_gate::{Refusal, RefusalCode, seat_gate_with};
use mcp_bridge::seat_state::SeatAvailability;
// Orchestrator compile fix (GAUNTLET triage): Surface is re-exported by seat_gate, not the crate root.
use mcp_bridge::seat_gate::Surface;
use shared_types::Origin;

#[derive(Clone, Copy)]
enum SurfaceOutcome {
    Allowed,
    CannotWrite,
    Deferred,
}

fn surface_case(index: usize) -> (Surface, &'static str, SurfaceOutcome, bool) {
    match index {
        0 => (Surface::AskAgent, "AskAgent", SurfaceOutcome::Allowed, false),
        1 => (Surface::Session, "Session", SurfaceOutcome::Allowed, false),
        2 => (Surface::Jury, "Jury", SurfaceOutcome::Allowed, false),
        3 => (
            Surface::BlindValidate,
            "BlindValidate",
            SurfaceOutcome::CannotWrite,
            false,
        ),
        4 => (
            Surface::DoctorLiveProbe,
            "DoctorLiveProbe",
            SurfaceOutcome::Allowed,
            true,
        ),
        5 => (
            Surface::PeerReviewRotation,
            "PeerReviewRotation",
            SurfaceOutcome::Deferred,
            false,
        ),
        6 => (
            Surface::FleetSpawn,
            "FleetSpawn",
            SurfaceOutcome::Deferred,
            false,
        ),
        7 => (
            Surface::FleetTemporal,
            "FleetTemporal",
            SurfaceOutcome::Deferred,
            false,
        ),
        8 => (
            Surface::AbeDispatch,
            "AbeDispatch",
            SurfaceOutcome::Deferred,
            false,
        ),
        9 => (
            Surface::Prewarm,
            "Prewarm",
            SurfaceOutcome::Deferred,
            false,
        ),
        _ => panic!("surface case index out of range: {index}"),
    }
}

fn origin_case(unattended: bool) -> Origin {
    if unattended {
        Origin::Unattended
    } else {
        Origin::Interactive
    }
}

fn assert_allowed(result: Result<(), Refusal>, context: &str) {
    match result {
        Ok(()) => {}
        Err(refusal) => panic!(
            "{context}: expected allowed, got {} with message {:?}",
            refusal.code.as_str(),
            refusal.message
        ),
    }
}

fn assert_refusal(
    result: Result<(), Refusal>,
    expected_code: &'static str,
    expected_message: String,
    context: &str,
) {
    match result {
        Ok(()) => panic!(
            "{context}: expected refusal {expected_code} with message {:?}, got allowed",
            expected_message
        ),
        Err(refusal) => {
            assert_eq!(
                refusal.code.as_str(),
                expected_code,
                "{context}: wrong refusal code"
            );
            assert_eq!(
                refusal.message, expected_message,
                "{context}: wrong refusal payload"
            );
        }
    }
}

fn expected_surface_result(
    seat: &str,
    surface_name: &str,
    outcome: SurfaceOutcome,
) -> Result<(), (&'static str, String)> {
    match outcome {
        SurfaceOutcome::Allowed => Ok(()),
        SurfaceOutcome::CannotWrite => Err((
            "seat_cannot_write_here",
            format!(
                "seat_cannot_write_here: {seat}: blind validators must write test files and {seat} is read-only"
            ),
        )),
        SurfaceOutcome::Deferred => Err((
            "seat_surface_deferred",
            format!("seat_surface_deferred: {seat} on {surface_name} ships in v1.1"),
        )),
    }
}

fn expected_gate_result(
    seat: &str,
    surface_name: &str,
    outcome: SurfaceOutcome,
    doctor_live_probe: bool,
    configured: bool,
    verified: bool,
    suspended: bool,
) -> Result<(), (&'static str, String)> {
    if !configured {
        return Err((
            "seat_not_configured",
            format!(
                "seat_not_configured: {seat}: set TRIUMVIRATE_ENABLE_{}=1 and restart the daemon",
                seat.to_ascii_uppercase()
            ),
        ));
    }

    if suspended {
        return Err((
            "seat_suspended",
            format!(
                "seat_suspended: {seat}: vendor disabled account (run: triumvirate seat clear {seat})"
            ),
        ));
    }

    if !verified && !doctor_live_probe {
        return Err((
            "seat_unverified",
            format!(
                "seat_unverified: {seat}: run triumvirate doctor --live in a terminal"
            ),
        ));
    }

    expected_surface_result(seat, surface_name, outcome)
}

// REQ-091: Every refusal variant exposes its exact code prefix.
#[test]
fn refusal_code_strings_are_exact() {
    let cases = [
        (RefusalCode::SeatNotConfigured, "seat_not_configured"),
        (RefusalCode::SeatSuspended, "seat_suspended"),
        (RefusalCode::SeatUnverified, "seat_unverified"),
        (RefusalCode::SeatSurfaceRefused, "seat_surface_refused"),
        (RefusalCode::SeatSurfaceDeferred, "seat_surface_deferred"),
        (RefusalCode::SeatCannotWriteHere, "seat_cannot_write_here"),
    ];

    for (code, expected) in cases {
        assert_eq!(code.as_str(), expected);
    }
}

// REQ-090 and REQ-091: Every new-seat surface, origin, TTY, and availability combination resolves to the exact launch result.
#[test]
fn complete_new_seat_gate_matrix_has_exact_codes_and_payloads() {
    let seats = ["kimi", "mistral", "qwen"];
    let mut checked = 0usize;

    for seat in seats {
        for surface_index in 0..10 {
            for unattended in [false, true] {
                for tty in [false, true] {
                    for configured in [false, true] {
                        for verified in [false, true] {
                            for suspended in [false, true] {
                                let (surface, surface_name, outcome, doctor_live_probe) =
                                    surface_case(surface_index);
                                let state = SeatAvailability {
                                    configured,
                                    verified,
                                    suspended: suspended
                                        .then(|| "vendor disabled account".to_owned()),
                                };
                                let context = format!(
                                    "seat={seat}, surface={surface_name}, unattended={unattended}, tty={tty}, configured={configured}, verified={verified}, suspended={suspended}"
                                );

                                let actual = seat_gate_with(
                                    seat,
                                    surface,
                                    origin_case(unattended),
                                    tty,
                                    &state,
                                );
                                let expected = expected_gate_result(
                                    seat,
                                    surface_name,
                                    outcome,
                                    doctor_live_probe,
                                    configured,
                                    verified,
                                    suspended,
                                );

                                match expected {
                                    Ok(()) => assert_allowed(actual, &context),
                                    Err((code, message)) => {
                                        assert_refusal(actual, code, message, &context)
                                    }
                                }

                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    assert_eq!(checked, 960);
}

// Amendment A-2 and REQ-090: No launch triple for a new seat produces seat_surface_refused.
#[test]
fn no_new_seat_launch_triple_is_surface_refused() {
    let available = SeatAvailability {
        configured: true,
        verified: true,
        suspended: None,
    };
    let mut checked = 0usize;

    for seat in ["kimi", "mistral", "qwen"] {
        for surface_index in 0..10 {
            for unattended in [false, true] {
                for tty in [false, true] {
                    let (surface, _, _, _) = surface_case(surface_index);
                    let result =
                        seat_gate_with(seat, surface, origin_case(unattended), tty, &available);

                    if let Err(refusal) = result {
                        assert_ne!(
                            refusal.code.as_str(),
                            "seat_surface_refused",
                            "seat={seat}, surface_index={surface_index}, unattended={unattended}, tty={tty}"
                        );
                    }

                    checked += 1;
                }
            }
        }
    }

    assert_eq!(checked, 120);
}

// Amendment A-2 and REQ-091: DoctorLiveProbe skips verification and has no origin or TTY restriction.
#[test]
fn doctor_live_probe_skips_unverified_and_has_no_tty_rule() {
    let unverified = SeatAvailability {
        configured: true,
        verified: false,
        suspended: None,
    };
    let mut checked = 0usize;

    for seat in ["kimi", "mistral", "qwen"] {
        for unattended in [false, true] {
            for tty in [false, true] {
                let result = seat_gate_with(
                    seat,
                    Surface::DoctorLiveProbe,
                    origin_case(unattended),
                    tty,
                    &unverified,
                );
                let context =
                    format!("seat={seat}, DoctorLiveProbe, unattended={unattended}, tty={tty}");
                assert_allowed(result, &context);
                checked += 1;
            }
        }
    }

    assert_eq!(checked, 12);
}

// REQ-091: Suspension outranks missing verification, including on DoctorLiveProbe.
#[test]
fn suspension_outranks_unverified_and_doctor_probe_exception() {
    let suspended = SeatAvailability {
        configured: true,
        verified: false,
        suspended: Some("billing access revoked".to_owned()),
    };

    for seat in ["kimi", "mistral", "qwen"] {
        for surface_index in 0..10 {
            let (surface, surface_name, _, _) = surface_case(surface_index);
            let result =
                seat_gate_with(seat, surface, Origin::Interactive, true, &suspended);
            assert_refusal(
                result,
                "seat_suspended",
                format!(
                    "seat_suspended: {seat}: billing access revoked (run: triumvirate seat clear {seat})"
                ),
                &format!("seat={seat}, surface={surface_name}"),
            );
        }
    }
}

// REQ-091: Not configured outranks both suspension and missing verification.
#[test]
fn not_configured_has_highest_precedence() {
    let blocked = SeatAvailability {
        configured: false,
        verified: false,
        suspended: Some("billing access revoked".to_owned()),
    };

    for seat in ["kimi", "mistral", "qwen"] {
        for surface_index in 0..10 {
            let (surface, surface_name, _, _) = surface_case(surface_index);
            let result =
                seat_gate_with(seat, surface, Origin::Unattended, false, &blocked);
            assert_refusal(
                result,
                "seat_not_configured",
                format!(
                    "seat_not_configured: {seat}: set TRIUMVIRATE_ENABLE_{}=1 and restart the daemon",
                    seat.to_ascii_uppercase()
                ),
                &format!("seat={seat}, surface={surface_name}"),
            );
        }
    }
}

// REQ-090: Available existing seats are allowed on every surface regardless of origin or TTY.
#[test]
fn available_existing_seats_are_allowed_everywhere() {
    let available = SeatAvailability {
        configured: true,
        verified: true,
        suspended: None,
    };
    let mut checked = 0usize;

    for seat in ["codex", "gemini", "grok", "claude", "deepseek"] {
        for surface_index in 0..10 {
            for unattended in [false, true] {
                for tty in [false, true] {
                    let (surface, surface_name, _, _) = surface_case(surface_index);
                    let result =
                        seat_gate_with(seat, surface, origin_case(unattended), tty, &available);
                    let context = format!(
                        "seat={seat}, surface={surface_name}, unattended={unattended}, tty={tty}"
                    );
                    assert_allowed(result, &context);
                    checked += 1;
                }
            }
        }
    }

    assert_eq!(checked, 200);
}
