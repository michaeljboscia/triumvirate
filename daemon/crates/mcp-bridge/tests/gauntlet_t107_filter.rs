// GAUNTLET filter suite for T-107, written blind from the spec (no implementation read).
//
// SPEC_AMBIGUITY: the public API lists `seat_gate_in` only as a doc comment ("seat_gate with the state directory already
// resolved") with no signature, so no test drives the filesystem-backed gate; only the pure `seat_gate_with` is tested.
// SPEC_AMBIGUITY: a seat string with no SeatProfile row ("not a seat at all") has no stated code or payload. The test only
// asserts it is refused (Err), not which code.
// SPEC_AMBIGUITY: for existing seats (codex, gemini, grok, claude, deepseek) the spec says "configured always, no
// verification" but `seat_gate_with` receives `state.verified` explicitly. Tests give existing seats a fully healthy state
// (configured, verified, not suspended) and do not assert what verified=false does to them.
// SPEC_AMBIGUITY: the `<surface>` text in payloads is taken to be the variant name (spec examples: "Prewarm",
// "BlindValidate"). Names for the other variants are assumed to be their identifiers.

use mcp_bridge::seat_gate::{
    Origin, Refusal, RefusalCode, Surface, installed_cli_version, register_installed_cli_version,
    seat_gate_with,
};
use mcp_bridge::seat_profile::all_seats;
use mcp_bridge::seat_state::SeatAvailability;

const NEW_SEATS: [&str; 3] = ["kimi", "mistral", "qwen"];
const ORIGINS: [Origin; 2] = [Origin::Interactive, Origin::Unattended];

fn surface_name(s: Surface) -> &'static str {
    match s {
        Surface::AskAgent => "AskAgent",
        Surface::Session => "Session",
        Surface::Jury => "Jury",
        Surface::BlindValidate => "BlindValidate",
        Surface::DoctorLiveProbe => "DoctorLiveProbe",
        Surface::PeerReviewRotation => "PeerReviewRotation",
        Surface::FleetSpawn => "FleetSpawn",
        Surface::FleetTemporal => "FleetTemporal",
        Surface::AbeDispatch => "AbeDispatch",
        Surface::Prewarm => "Prewarm",
    }
}

fn all_surfaces() -> Vec<Surface> {
    Surface::ALL.iter().map(|s| *s).collect()
}

fn healthy() -> SeatAvailability {
    SeatAvailability { configured: true, verified: true, suspended: None }
}

fn is_launch_surface(s: Surface) -> bool {
    matches!(
        s,
        Surface::AskAgent | Surface::Session | Surface::Jury | Surface::DoctorLiveProbe
    )
}

fn refuse(seat: &str, s: Surface, o: Origin, tty: bool, st: &SeatAvailability) -> Refusal {
    match seat_gate_with(seat, s, o, tty, st) {
        Ok(()) => panic!("expected refusal for {seat} on {}", surface_name(s)),
        Err(r) => r,
    }
}

fn expect_refusal(seat: &str, s: Surface, st: &SeatAvailability, code: &str, message: &str) {
    for o in ORIGINS {
        for tty in [true, false] {
            let r = refuse(seat, s, o, tty, st);
            assert_eq!(r.code.as_str(), code, "{seat} {}", surface_name(s));
            assert_eq!(r.message, message, "{seat} {}", surface_name(s));
        }
    }
}

// ---- Tier 1: properties over the full table ----

#[test]
fn test_should_cover_exactly_ten_surfaces_in_all() {
    // BACKEND_STRUCTURE section 3: Surface has ten variants.
    assert_eq!(Surface::ALL.len(), 10);
}

#[test]
fn test_should_resolve_every_new_seat_triple_to_the_amendment_a2_table() {
    // A-2 table: launch surfaces allowed for any origin and tty; BlindValidate cannot-write; the rest deferred.
    for seat in NEW_SEATS {
        for s in all_surfaces() {
            for o in ORIGINS {
                for tty in [true, false] {
                    let got = seat_gate_with(seat, s, o, tty, &healthy());
                    if is_launch_surface(s) {
                        assert!(got.is_ok(), "{seat} {} should be allowed", surface_name(s));
                    } else if matches!(s, Surface::BlindValidate) {
                        let r = got.expect_err("BlindValidate must be refused");
                        assert_eq!(r.code.as_str(), "seat_cannot_write_here");
                        assert_eq!(
                            r.message,
                            format!(
                                "seat_cannot_write_here: {seat}: blind validators must write test files and {seat} is read-only"
                            )
                        );
                    } else {
                        let r = got.expect_err("deferred surface must be refused");
                        assert_eq!(r.code.as_str(), "seat_surface_deferred");
                        assert_eq!(
                            r.message,
                            format!(
                                "seat_surface_deferred: {seat} on {} ships in v1.1",
                                surface_name(s)
                            )
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn test_should_never_produce_seat_surface_refused_for_any_triple_or_state() {
    // A-2: zero (seat, surface, origin, tty) triples map to seat_surface_refused.
    let states = [
        healthy(),
        SeatAvailability { configured: false, verified: false, suspended: None },
        SeatAvailability { configured: true, verified: false, suspended: None },
        SeatAvailability { configured: true, verified: true, suspended: Some("r".into()) },
        SeatAvailability { configured: true, verified: false, suspended: Some("r".into()) },
    ];
    for seat in NEW_SEATS {
        for s in all_surfaces() {
            for o in ORIGINS {
                for tty in [true, false] {
                    for st in &states {
                        if let Err(r) = seat_gate_with(seat, s, o, tty, st) {
                            assert_ne!(r.code.as_str(), "seat_surface_refused");
                            assert!(!r.message.starts_with("seat_surface_refused"));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn test_should_prefix_every_message_with_its_code_and_colon() {
    // BACKEND_STRUCTURE section 3: the code starts every payload string.
    let states = [
        healthy(),
        SeatAvailability { configured: false, verified: false, suspended: None },
        SeatAvailability { configured: true, verified: false, suspended: None },
        SeatAvailability { configured: true, verified: true, suspended: Some("boom".into()) },
    ];
    for seat in NEW_SEATS {
        for s in all_surfaces() {
            for st in &states {
                if let Err(r) = seat_gate_with(seat, s, Origin::Interactive, true, st) {
                    let prefix = format!("{}: ", r.code.as_str());
                    assert!(r.message.starts_with(&prefix), "{}", r.message);
                }
            }
        }
    }
}

#[test]
fn test_should_be_deterministic_for_repeated_calls() {
    // done_when: every combination resolves to exactly one code with the exact payload (pure decision).
    let a = refuse("qwen", Surface::Prewarm, Origin::Unattended, false, &healthy());
    let b = refuse("qwen", Surface::Prewarm, Origin::Unattended, false, &healthy());
    assert_eq!(a.code.as_str(), b.code.as_str());
    assert_eq!(a.message, b.message);
}

#[test]
fn test_should_not_depend_on_origin_or_tty_for_new_seats() {
    // A-2: origin and tty do not change any decision at launch.
    for seat in NEW_SEATS {
        for s in all_surfaces() {
            let base = seat_gate_with(seat, s, Origin::Interactive, true, &healthy());
            for o in ORIGINS {
                for tty in [true, false] {
                    let got = seat_gate_with(seat, s, o, tty, &healthy());
                    assert_eq!(base.is_ok(), got.is_ok());
                    if let (Err(x), Err(y)) = (&base, &got) {
                        assert_eq!(x.message, y.message);
                    }
                }
            }
        }
    }
}

// ---- Tier 2: spec happy paths and named examples ----

#[test]
fn test_should_allow_qwen_ask_agent_when_unattended() {
    // A-2 task impact: (qwen, AskAgent, Unattended) == allowed.
    assert!(seat_gate_with("qwen", Surface::AskAgent, Origin::Unattended, false, &healthy()).is_ok());
}

#[test]
fn test_should_allow_kimi_doctor_live_probe_without_tty() {
    // A-2: (kimi, DoctorLiveProbe, tty=false) == allowed.
    assert!(
        seat_gate_with("kimi", Surface::DoctorLiveProbe, Origin::Interactive, false, &healthy())
            .is_ok()
    );
}

#[test]
fn test_should_defer_qwen_prewarm_with_exact_payload() {
    // A-2: (qwen, Prewarm) == seat_surface_deferred payload.
    let r = refuse("qwen", Surface::Prewarm, Origin::Unattended, true, &healthy());
    assert_eq!(r.code.as_str(), "seat_surface_deferred");
    assert_eq!(r.message, "seat_surface_deferred: qwen on Prewarm ships in v1.1");
}

#[test]
fn test_should_defer_mistral_prewarm_with_exact_payload() {
    // reality_test: (mistral, Prewarm) == seat_surface_deferred payload.
    let r = refuse("mistral", Surface::Prewarm, Origin::Interactive, true, &healthy());
    assert_eq!(r.message, "seat_surface_deferred: mistral on Prewarm ships in v1.1");
}

#[test]
fn test_should_refuse_qwen_blind_validate_as_cannot_write_not_surface_refused() {
    // A-2: (qwen, BlindValidate, Unattended) is cannot-write, replacing the old surface_refused string.
    let r = refuse("qwen", Surface::BlindValidate, Origin::Unattended, true, &healthy());
    assert_eq!(r.code.as_str(), "seat_cannot_write_here");
    assert_eq!(
        r.message,
        "seat_cannot_write_here: qwen: blind validators must write test files and qwen is read-only"
    );
}

#[test]
fn test_should_defer_each_non_launch_surface_for_kimi() {
    // A-2 table rows PeerReviewRotation, FleetSpawn, FleetTemporal, AbeDispatch, Prewarm.
    for (s, name) in [
        (Surface::PeerReviewRotation, "PeerReviewRotation"),
        (Surface::FleetSpawn, "FleetSpawn"),
        (Surface::FleetTemporal, "FleetTemporal"),
        (Surface::AbeDispatch, "AbeDispatch"),
        (Surface::Prewarm, "Prewarm"),
    ] {
        expect_refusal(
            "kimi",
            s,
            &healthy(),
            "seat_surface_deferred",
            &format!("seat_surface_deferred: kimi on {name} ships in v1.1"),
        );
    }
}

#[test]
fn test_should_allow_every_surface_for_existing_seats() {
    // seat_gate.rs doc: existing seats are allowed everywhere (today's behavior).
    for p in all_seats() {
        if NEW_SEATS.contains(&p.key) {
            continue;
        }
        for s in all_surfaces() {
            for o in ORIGINS {
                for tty in [true, false] {
                    assert!(
                        seat_gate_with(p.key, s, o, tty, &healthy()).is_ok(),
                        "{} on {} should be allowed",
                        p.key,
                        surface_name(s)
                    );
                }
            }
        }
    }
}

#[test]
fn test_should_include_codex_and_grok_among_existing_seats() {
    // Guard so the existing-seat loop above is not vacuous.
    let keys: Vec<&str> = all_seats().iter().map(|p| p.key).collect();
    for k in ["codex", "gemini", "grok", "claude", "kimi", "mistral", "qwen"] {
        assert!(keys.contains(&k), "missing seat row {k}");
    }
}

// ---- Tier 3: precedence and tie-break ----

#[test]
fn test_should_report_not_configured_with_exact_payload() {
    // REQ-091 payload: seat_not_configured.
    let st = SeatAvailability { configured: false, verified: false, suspended: None };
    expect_refusal(
        "kimi",
        Surface::AskAgent,
        &st,
        "seat_not_configured",
        "seat_not_configured: kimi: set TRIUMVIRATE_ENABLE_KIMI=1 and restart the daemon",
    );
}

#[test]
fn test_should_uppercase_seat_in_enable_variable() {
    // REQ-091 payload: TRIUMVIRATE_ENABLE_<SEAT> is upper case.
    let st = SeatAvailability { configured: false, verified: true, suspended: None };
    let r = refuse("mistral", Surface::Session, Origin::Interactive, true, &st);
    assert_eq!(
        r.message,
        "seat_not_configured: mistral: set TRIUMVIRATE_ENABLE_MISTRAL=1 and restart the daemon"
    );
}

#[test]
fn test_should_report_suspended_with_reason_and_clear_hint() {
    // REQ-091 payload: seat_suspended.
    let st = SeatAvailability {
        configured: true,
        verified: true,
        suspended: Some("account suspended by vendor".to_string()),
    };
    expect_refusal(
        "qwen",
        Surface::Jury,
        &st,
        "seat_suspended",
        "seat_suspended: qwen: account suspended by vendor (run: triumvirate seat clear qwen)",
    );
}

#[test]
fn test_should_embed_unicode_suspension_reason_verbatim() {
    // REQ-091 payload: reason is inserted as given.
    let st = SeatAvailability {
        configured: true,
        verified: true,
        suspended: Some("暂停 \u{1F6AB} 403".to_string()),
    };
    let r = refuse("kimi", Surface::AskAgent, Origin::Interactive, true, &st);
    assert_eq!(
        r.message,
        "seat_suspended: kimi: 暂停 \u{1F6AB} 403 (run: triumvirate seat clear kimi)"
    );
}

#[test]
fn test_should_report_unverified_with_exact_payload() {
    // REQ-091 payload: seat_unverified.
    let st = SeatAvailability { configured: true, verified: false, suspended: None };
    expect_refusal(
        "mistral",
        Surface::AskAgent,
        &st,
        "seat_unverified",
        "seat_unverified: mistral: run triumvirate doctor --live in a terminal",
    );
}

#[test]
fn test_should_rank_not_configured_above_suspended_and_unverified() {
    // REQ-091 precedence: not configured first.
    let st = SeatAvailability { configured: false, verified: false, suspended: Some("x".into()) };
    let r = refuse("kimi", Surface::AskAgent, Origin::Interactive, true, &st);
    assert_eq!(r.code.as_str(), "seat_not_configured");
}

#[test]
fn test_should_rank_suspended_above_unverified() {
    // REQ-091 precedence: suspended outranks unverified.
    let st = SeatAvailability { configured: true, verified: false, suspended: Some("x".into()) };
    let r = refuse("qwen", Surface::AskAgent, Origin::Interactive, true, &st);
    assert_eq!(r.code.as_str(), "seat_suspended");
}

#[test]
fn test_should_rank_unverified_above_deferred_and_cannot_write() {
    // REQ-091 precedence: unverified outranks the surface rule.
    let st = SeatAvailability { configured: true, verified: false, suspended: None };
    for s in [Surface::Prewarm, Surface::BlindValidate, Surface::FleetSpawn] {
        let r = refuse("kimi", s, Origin::Interactive, true, &st);
        assert_eq!(r.code.as_str(), "seat_unverified", "{}", surface_name(s));
    }
}

#[test]
fn test_should_rank_not_configured_above_surface_rule() {
    // REQ-091 precedence: not configured outranks deferred.
    let st = SeatAvailability { configured: false, verified: true, suspended: None };
    let r = refuse("qwen", Surface::Prewarm, Origin::Interactive, true, &st);
    assert_eq!(r.code.as_str(), "seat_not_configured");
}

#[test]
fn test_should_rank_suspended_above_surface_rule() {
    // REQ-091 precedence: suspended outranks cannot-write.
    let st = SeatAvailability { configured: true, verified: true, suspended: Some("x".into()) };
    let r = refuse("mistral", Surface::BlindValidate, Origin::Interactive, true, &st);
    assert_eq!(r.code.as_str(), "seat_suspended");
}

#[test]
fn test_should_skip_unverified_check_for_doctor_live_probe() {
    // REQ-091: unverified is not checked for DoctorLiveProbe (how a seat gets verified).
    let st = SeatAvailability { configured: true, verified: false, suspended: None };
    for seat in NEW_SEATS {
        for o in ORIGINS {
            for tty in [true, false] {
                assert!(
                    seat_gate_with(seat, Surface::DoctorLiveProbe, o, tty, &st).is_ok(),
                    "{seat} probe must be allowed while unverified"
                );
            }
        }
    }
}

#[test]
fn test_should_still_check_suspended_and_configured_for_doctor_live_probe() {
    // REQ-091: only the unverified level is skipped for the probe.
    let suspended = SeatAvailability { configured: true, verified: false, suspended: Some("s".into()) };
    let r = refuse("kimi", Surface::DoctorLiveProbe, Origin::Interactive, true, &suspended);
    assert_eq!(r.code.as_str(), "seat_suspended");
    let off = SeatAvailability { configured: false, verified: false, suspended: None };
    let r = refuse("kimi", Surface::DoctorLiveProbe, Origin::Interactive, true, &off);
    assert_eq!(r.code.as_str(), "seat_not_configured");
}

#[test]
fn test_should_apply_unverified_check_to_non_probe_surfaces_only() {
    // REQ-091: every other launch surface is blocked by unverified.
    let st = SeatAvailability { configured: true, verified: false, suspended: None };
    for s in [Surface::AskAgent, Surface::Session, Surface::Jury] {
        let r = refuse("qwen", s, Origin::Interactive, true, &st);
        assert_eq!(r.code.as_str(), "seat_unverified");
    }
}

// ---- Tier 4: code strings and error path ----

#[test]
fn test_should_expose_exact_code_strings() {
    // BACKEND_STRUCTURE section 3: the six code strings.
    assert_eq!(RefusalCode::SeatNotConfigured.as_str(), "seat_not_configured");
    assert_eq!(RefusalCode::SeatSuspended.as_str(), "seat_suspended");
    assert_eq!(RefusalCode::SeatUnverified.as_str(), "seat_unverified");
    assert_eq!(RefusalCode::SeatSurfaceRefused.as_str(), "seat_surface_refused");
    assert_eq!(RefusalCode::SeatSurfaceDeferred.as_str(), "seat_surface_deferred");
    assert_eq!(RefusalCode::SeatCannotWriteHere.as_str(), "seat_cannot_write_here");
}

#[test]
fn test_should_refuse_a_seat_with_no_profile_row() {
    // seat_gate_with doc: a seat with no SeatProfile row is not a seat at all (code not specified, see SPEC_AMBIGUITY).
    let got = seat_gate_with(
        "no_such_seat_t107",
        Surface::AskAgent,
        Origin::Interactive,
        true,
        &healthy(),
    );
    assert!(got.is_err());
}

#[test]
fn test_should_refuse_empty_seat_name() {
    // seat_gate_with doc: no profile row means not a seat.
    assert!(seat_gate_with("", Surface::AskAgent, Origin::Interactive, true, &healthy()).is_err());
}

// ---- Tier 5: installed CLI version registry ----

#[test]
fn test_should_return_none_for_unregistered_cli_version() {
    // installed_cli_version doc: a seat with no registered version reads as unknown.
    assert_eq!(installed_cli_version("t107_filter_never_registered"), None);
}

#[test]
fn test_should_round_trip_registered_cli_version() {
    // register/installed_cli_version doc: read back what was registered.
    register_installed_cli_version("t107_filter_seat_a", "2.2.0");
    assert_eq!(installed_cli_version("t107_filter_seat_a"), Some("2.2.0".to_string()));
}

#[test]
fn test_should_overwrite_cli_version_after_upgrade() {
    // register doc: re-registered after a CLI upgrade is detected.
    register_installed_cli_version("t107_filter_seat_b", "1.0.0");
    register_installed_cli_version("t107_filter_seat_b", "1.1.0");
    assert_eq!(installed_cli_version("t107_filter_seat_b"), Some("1.1.0".to_string()));
}

#[test]
fn test_should_keep_cli_versions_isolated_per_seat() {
    // registry is keyed by seat.
    register_installed_cli_version("t107_filter_seat_c", "3.0.0");
    register_installed_cli_version("t107_filter_seat_d", "4.0.0");
    assert_eq!(installed_cli_version("t107_filter_seat_c"), Some("3.0.0".to_string()));
    assert_eq!(installed_cli_version("t107_filter_seat_d"), Some("4.0.0".to_string()));
    assert_eq!(installed_cli_version("t107_filter_seat_e_unset"), None);
}
