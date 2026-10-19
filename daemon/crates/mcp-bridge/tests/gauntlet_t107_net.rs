// GAUNTLET net suite for T-107, written blind from the spec (no implementation read).
//
// SPEC_AMBIGUITY: the spec says "a seat with no SeatProfile row is not a seat at all" but gives no refusal code or payload
// for an unknown seat. No test asserts one.
// SPEC_AMBIGUITY: seat_surface_refused is unreachable at launch (AMENDMENTS A-2: no triple produces it). Its positive payload
// (Alibaba or Moonshot terms) cannot be reached through the listed API, so only the zero-triples rule is asserted.
// SPEC_AMBIGUITY: the <surface> token in payloads is not defined by the spec. This suite uses the Rust variant name
// (for example Prewarm, BlindValidate), the form the spec's own examples use (A-2: "qwen on Prewarm ships in v1.1").
// SPEC_AMBIGUITY: Origin variant names (Interactive, Unattended) come from BACKEND_STRUCTURE.md section 3, not from the
// listed public API.
// SPEC_AMBIGUITY: the seat-key directory guard, the atomic-write helper and the triumvirate_home rule are described in doc
// comments without listed signatures, and triumvirate_home and is_configured read process environment. Not tested here.

use mcp_bridge::seat_gate::{Origin, Refusal, seat_gate_with};
use mcp_bridge::seat_profile::{
    AttemptPolicy, ConcurrencyLimit, Configured, Containment, NEW_SEAT_LAUNCH_SURFACES,
    PrewarmPolicy, Provider, QuotaPolicy, StallWindow, Surface, SuspensionPolicy, VerifyPolicy,
    all_seats, seat_profile,
};
use mcp_bridge::seat_state::{
    Blocker, SeatAvailability, VerifiedRecord, availability, clear as clear_suspended,
    delete_verified, write_suspended, write_verified,
};

// ---------- helpers ----------

fn surfaces() -> Vec<Surface> {
    vec![
        Surface::AskAgent,
        Surface::Session,
        Surface::Jury,
        Surface::BlindValidate,
        Surface::DoctorLiveProbe,
        Surface::PeerReviewRotation,
        Surface::FleetSpawn,
        Surface::FleetTemporal,
        Surface::AbeDispatch,
        Surface::Prewarm,
    ]
}

fn surface_name(s: &Surface) -> &'static str {
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

/// Collapse a gate result to (code, payload). None means the call is allowed.
fn describe(r: Result<(), Refusal>) -> Option<(String, String)> {
    match r {
        Ok(()) => None,
        Err(e) => Some((e.code.as_str().to_string(), e.message)),
    }
}

fn exp(code: &str, message: String) -> Option<(String, String)> {
    Some((code.to_string(), message))
}

fn av(configured: bool, verified: bool, suspended: Option<&str>) -> SeatAvailability {
    SeatAvailability {
        configured,
        verified,
        suspended: suspended.map(|s| s.to_string()),
    }
}

/// The spec's decision for a NEW seat (kimi, mistral, qwen) in a given state, per SPEC_v2 REQ-090/091 and A-2.
/// Origin and tty never change the answer at launch.
fn expected_new_seat(
    seat: &str,
    surface: &Surface,
    st: &SeatAvailability,
) -> Option<(String, String)> {
    if !st.configured {
        return exp(
            "seat_not_configured",
            format!(
                "seat_not_configured: {seat}: set TRIUMVIRATE_ENABLE_{}=1 and restart the daemon",
                seat.to_uppercase()
            ),
        );
    }
    if let Some(reason) = &st.suspended {
        return exp(
            "seat_suspended",
            format!("seat_suspended: {seat}: {reason} (run: triumvirate seat clear {seat})"),
        );
    }
    let is_probe = matches!(surface, Surface::DoctorLiveProbe);
    if !st.verified && !is_probe {
        return exp(
            "seat_unverified",
            format!("seat_unverified: {seat}: run triumvirate doctor --live in a terminal"),
        );
    }
    match surface {
        Surface::AskAgent
        | Surface::Session
        | Surface::Jury
        | Surface::DoctorLiveProbe
        // W-1 (A-16): the writer surfaces, through `triumvirate seat-run`.
        | Surface::AbeDispatch
        | Surface::FleetSpawn
        | Surface::FleetTemporal => None,
        Surface::BlindValidate => exp(
            "seat_cannot_write_here",
            format!(
                "seat_cannot_write_here: {seat}: blind validators must write test files and {seat} is read-only"
            ),
        ),
        other => exp(
            "seat_surface_deferred",
            format!(
                "seat_surface_deferred: {seat} on {} ships in v1.1",
                surface_name(other)
            ),
        ),
    }
}

fn blocker_desc(a: &SeatAvailability) -> String {
    match a.first_blocker() {
        None => "none".to_string(),
        Some(Blocker::NotConfigured) => "not_configured".to_string(),
        Some(Blocker::Suspended(reason)) => format!("suspended:{reason}"),
        Some(Blocker::Unverified) => "unverified".to_string(),
    }
}

fn rec(cli: &str) -> VerifiedRecord {
    VerifiedRecord {
        cli_version: cli.to_string(),
        seat_home: "fixture-home/kimi".to_string(),
        probed_at: "2026-10-18T12:00:00-04:00".to_string(),
        served_model: Some("fixture-model".to_string()),
    }
}

// ---------- REQ-090 / REQ-091: seat_gate_with decisions ----------

#[test]
// REQ-090, REQ-091, A-2: every (new seat, surface, origin, tty, state) cell resolves to exactly one code and exact payload.
fn new_seats_full_matrix_exact_payloads() {
    let states = [
        av(false, true, None),                             // not configured
        av(true, true, Some("account access terminated")), // suspended
        av(true, false, None),                             // configured, unverified
        av(true, true, None),                              // available
    ];
    let mut cells = 0usize;
    let mut refused = 0usize;
    for seat in ["kimi", "mistral", "qwen"] {
        for st in &states {
            for i in 0..10 {
                let name = surface_name(&surface_at(i));
                for origin in [Origin::Interactive, Origin::Unattended] {
                    for tty in [true, false] {
                        let surface = surface_at(i);
                        let expected = expected_new_seat(seat, &surface, st);
                        let got = describe(seat_gate_with(seat, surface, origin, tty, st));
                        if let Some((code, _)) = &got
                            && code == "seat_surface_refused"
                        {
                            refused += 1;
                        }
                        assert_eq!(got, expected, "seat {seat} surface {name} tty {tty}");
                        cells += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cells, 3 * 4 * 10 * 2 * 2, "matrix size");
    assert_eq!(refused, 0, "A-2: zero triples yield seat_surface_refused");
}

/// A fresh Surface at roster index `i` (Surface is not Clone in the listed API, so each call builds a new one).
fn surface_at(i: usize) -> Surface {
    surfaces().remove(i)
}

#[test]
// REQ-090, A-2: Kimi, Mistral and Qwen run read-only AskAgent, Session and Jury for both origins and both TTY values.
fn launch_surfaces_allowed_for_any_origin_and_tty() {
    let st = av(true, true, None);
    for seat in ["kimi", "mistral", "qwen"] {
        for i in [0usize, 1, 2] {
            for origin in [Origin::Interactive, Origin::Unattended] {
                for tty in [true, false] {
                    let surface = surface_at(i);
                    let name = surface_name(&surface);
                    assert!(
                        describe(seat_gate_with(seat, surface, origin, tty, &st)).is_none(),
                        "{seat} on {name} must be allowed"
                    );
                }
            }
        }
    }
}

#[test]
// A-2 examples: (qwen, AskAgent, Unattended) is allowed; (kimi, DoctorLiveProbe, tty=false) is allowed.
fn a2_worked_examples_allowed() {
    let st = av(true, true, None);
    assert!(
        describe(seat_gate_with(
            "qwen",
            Surface::AskAgent,
            Origin::Unattended,
            false,
            &st
        ))
        .is_none()
    );
    let probe = describe(seat_gate_with(
        "kimi",
        Surface::DoctorLiveProbe,
        Origin::Interactive,
        false,
        &st,
    ));
    assert!(probe.is_none());
}

#[test]
// A-2 examples: (qwen, Prewarm) and (mistral, Prewarm) return the exact v1.1 deferral payload.
fn prewarm_is_deferred_with_exact_payload() {
    let st = av(true, true, None);
    assert_eq!(
        describe(seat_gate_with(
            "qwen",
            Surface::Prewarm,
            Origin::Unattended,
            false,
            &st
        )),
        exp(
            "seat_surface_deferred",
            "seat_surface_deferred: qwen on Prewarm ships in v1.1".to_string()
        )
    );
    assert_eq!(
        describe(seat_gate_with(
            "mistral",
            Surface::Prewarm,
            Origin::Unattended,
            false,
            &st
        )),
        exp(
            "seat_surface_deferred",
            "seat_surface_deferred: mistral on Prewarm ships in v1.1".to_string()
        )
    );
}

#[test]
// A-2 (replaces the reality example): BlindValidate on a read-only new seat is seat_cannot_write_here, even unattended.
fn blind_validate_is_cannot_write_here_not_refused() {
    let st = av(true, true, None);
    assert_eq!(
        describe(seat_gate_with("qwen", Surface::BlindValidate, Origin::Unattended, false, &st)),
        exp(
            "seat_cannot_write_here",
            "seat_cannot_write_here: qwen: blind validators must write test files and qwen is read-only".to_string()
        )
    );
}

#[test]
// REQ-091 precedence: not configured outranks suspended and unverified.
fn not_configured_outranks_suspended_and_unverified() {
    let st = av(false, false, Some("account access terminated"));
    assert_eq!(
        describe(seat_gate_with(
            "kimi",
            Surface::AskAgent,
            Origin::Interactive,
            true,
            &st
        )),
        exp(
            "seat_not_configured",
            "seat_not_configured: kimi: set TRIUMVIRATE_ENABLE_KIMI=1 and restart the daemon"
                .to_string()
        )
    );
}

#[test]
// REQ-091 precedence: suspended outranks unverified, and suspended outranks the surface rule.
fn suspended_outranks_unverified_and_surface_rule() {
    let st = av(true, false, Some("provider terminated access"));
    assert_eq!(
        describe(seat_gate_with("mistral", Surface::AskAgent, Origin::Interactive, true, &st)),
        exp(
            "seat_suspended",
            "seat_suspended: mistral: provider terminated access (run: triumvirate seat clear mistral)".to_string()
        )
    );
    let st2 = av(true, true, Some("provider terminated access"));
    assert_eq!(
        describe(seat_gate_with("mistral", Surface::Prewarm, Origin::Unattended, false, &st2)),
        exp(
            "seat_suspended",
            "seat_suspended: mistral: provider terminated access (run: triumvirate seat clear mistral)".to_string()
        )
    );
}

#[test]
// REQ-091 precedence: unverified outranks the surface rule (BlindValidate on an unverified seat is seat_unverified).
fn unverified_outranks_surface_rule() {
    let st = av(true, false, None);
    assert_eq!(
        describe(seat_gate_with(
            "kimi",
            Surface::BlindValidate,
            Origin::Interactive,
            true,
            &st
        )),
        exp(
            "seat_unverified",
            "seat_unverified: kimi: run triumvirate doctor --live in a terminal".to_string()
        )
    );
}

#[test]
// REQ-091: the live probe is how a seat gets verified, so DoctorLiveProbe skips the unverified check, with or without a TTY.
fn doctor_probe_skips_unverified_check() {
    let st = av(true, false, None);
    assert!(
        describe(seat_gate_with(
            "qwen",
            Surface::DoctorLiveProbe,
            Origin::Interactive,
            true,
            &st
        ))
        .is_none()
    );
    assert!(
        describe(seat_gate_with(
            "qwen",
            Surface::DoctorLiveProbe,
            Origin::Interactive,
            false,
            &st
        ))
        .is_none()
    );
}

#[test]
// REQ-091: existing seats are configured always and need no verification, so they are allowed on every surface.
fn existing_seats_allowed_everywhere_without_verification() {
    let st = av(true, false, None);
    for seat in ["grok", "codex"] {
        for i in 0..10 {
            for origin in [Origin::Interactive, Origin::Unattended] {
                for tty in [true, false] {
                    let surface = surface_at(i);
                    let name = surface_name(&surface);
                    assert!(
                        describe(seat_gate_with(seat, surface, origin, tty, &st)).is_none(),
                        "{seat} on {name} must be allowed"
                    );
                }
            }
        }
    }
}

// ---------- Surface enum ----------

#[test]
// REQ-090: Surface::ALL lists the ten dispatch surfaces in spec order.
fn surface_all_lists_ten_surfaces_in_order() {
    let names: Vec<&str> = Surface::ALL.iter().map(surface_name).collect();
    assert_eq!(
        names,
        vec![
            "AskAgent",
            "Session",
            "Jury",
            "BlindValidate",
            "DoctorLiveProbe",
            "PeerReviewRotation",
            "FleetSpawn",
            "FleetTemporal",
            "AbeDispatch",
            "Prewarm",
        ]
    );
}

// ---------- REQ-090 / A-2: seat rows ----------

#[test]
// A-2: every new seat's surfaces_allowed is exactly Mistral's launch set; the shared constant matches.
fn new_seat_rows_allow_exactly_the_launch_surfaces() {
    let want_const: Vec<&str> = NEW_SEAT_LAUNCH_SURFACES.iter().map(surface_name).collect();
    assert_eq!(
        want_const,
        vec![
            "AskAgent",
            "Session",
            "Jury",
            "DoctorLiveProbe",
            "AbeDispatch",
            "FleetSpawn",
            "FleetTemporal"
        ]
    );
    for key in ["kimi", "mistral", "qwen"] {
        let p = seat_profile(key).expect("row exists");
        let mut got: Vec<&str> = p.surfaces_allowed.iter().map(surface_name).collect();
        got.sort();
        let mut want = want_const.clone();
        want.sort();
        assert_eq!(got, want, "{key} surfaces_allowed");
    }
}

#[test]
// REQ-091: each new seat is configured by its own TRIUMVIRATE_ENABLE_<SEAT> flag, default off.
fn new_seats_are_env_gated_with_their_own_flag() {
    for key in ["kimi", "mistral", "qwen"] {
        let p = seat_profile(key).expect("row exists");
        let want = format!("TRIUMVIRATE_ENABLE_{}", key.to_uppercase());
        match p.configured {
            Configured::EnvFlag(name) => assert_eq!(name, want.as_str(), "{key} enable flag"),
            Configured::Always => panic!("{key} must be env-gated"),
        }
    }
}

#[test]
// REQ-091, REQ-050, REQ-053, REQ-090: new-seat launch policy (verify, suspension, one process, single attempt, no prewarm).
fn new_seat_launch_policy_fields() {
    for key in ["kimi", "mistral", "qwen"] {
        let p = seat_profile(key).expect("row exists");
        assert!(
            matches!(p.verify_policy, VerifyPolicy::DoctorLive),
            "{key} verify"
        );
        assert!(
            matches!(p.suspension_policy, SuspensionPolicy::Classified),
            "{key} suspension"
        );
        assert!(
            matches!(p.concurrency_limit, ConcurrencyLimit::One),
            "{key} concurrency"
        );
        assert!(
            matches!(p.attempt_policy, AttemptPolicy::Single),
            "{key} attempts"
        );
        assert!(
            matches!(p.quota_policy, QuotaPolicy::TerminalTripBreaker),
            "{key} quota"
        );
        assert!(
            matches!(p.read_only_containment, Containment::SeatbeltPlusAudit),
            "{key} containment"
        );
        assert!(
            matches!(p.prewarm_policy, PrewarmPolicy::Never),
            "{key} prewarm"
        );
        assert!(
            matches!(p.stall_window, StallWindow::Secs(780)),
            "{key} stall window"
        );
        assert!(
            !p.can_write_test_file,
            "{key} cannot write test files (D-10)"
        );
        // W-1 (A-16): fleet and ABE writers through seat-run.
        assert!(p.fleet_buildable, "{key} fleet writer role");
        assert!(p.abe_buildable, "{key} ABE writer role");
    }
}

#[test]
// A-7: provider fields are OpenRouter slugs (moonshotai, mistralai, qwen; grok is x-ai, not xai).
fn provider_slugs_are_openrouter_slugs() {
    assert!(matches!(
        seat_profile("kimi").expect("row").provider,
        Provider::Moonshot
    ));
    assert_eq!(Provider::Moonshot.as_str(), "moonshotai");
    assert!(matches!(
        seat_profile("mistral").expect("row").provider,
        Provider::Mistral
    ));
    assert_eq!(Provider::Mistral.as_str(), "mistralai");
    assert!(matches!(
        seat_profile("qwen").expect("row").provider,
        Provider::Alibaba
    ));
    assert_eq!(Provider::Alibaba.as_str(), "qwen");
    assert!(matches!(
        seat_profile("grok").expect("row").provider,
        Provider::XAi
    ));
    assert_eq!(Provider::XAi.as_str(), "x-ai");
}

#[test]
// REQ-090: the roster contains the three new seat keys; an unknown key has no row.
fn roster_contains_new_seats_and_unknown_key_has_no_row() {
    let keys: Vec<&str> = all_seats().iter().map(|s| s.key).collect();
    for key in ["kimi", "mistral", "qwen"] {
        assert!(keys.contains(&key), "{key} in roster");
    }
    assert!(seat_profile("no-such-seat").is_none());
}

// ---------- REQ-091: seat_state (availability files) ----------

#[test]
// REQ-091: verified is read per call; a passing record takes effect on the next read without a restart.
fn verified_record_takes_effect_on_next_read() {
    let h = tempfile::tempdir().expect("tempdir");
    let before = availability(h.path(), "kimi", true, Some("2.2.0")).expect("read");
    assert!(!before.verified);
    assert!(!before.available());
    assert_eq!(blocker_desc(&before), "unverified");

    write_verified(h.path(), "kimi", &rec("2.2.0")).expect("write");
    let after = availability(h.path(), "kimi", true, Some("2.2.0")).expect("read");
    assert!(after.verified);
    assert!(after.available());
    assert_eq!(blocker_desc(&after), "none");
}

#[test]
// REQ-091: a CLI version change flips verified to absent; an unknown installed version also reads as unverified (fail closed).
fn cli_version_change_or_unknown_version_flips_verified() {
    let h = tempfile::tempdir().expect("tempdir");
    write_verified(h.path(), "kimi", &rec("2.2.0")).expect("write");
    let moved = availability(h.path(), "kimi", true, Some("2.3.0")).expect("read");
    assert!(!moved.verified);
    assert_eq!(blocker_desc(&moved), "unverified");
    let unknown = availability(h.path(), "kimi", true, None).expect("read");
    assert!(!unknown.verified);
    assert_eq!(blocker_desc(&unknown), "unverified");
}

#[test]
// REQ-091: delete_verified removes the record; the seat reads unverified on the next read.
fn delete_verified_makes_seat_unverified() {
    let h = tempfile::tempdir().expect("tempdir");
    write_verified(h.path(), "qwen", &rec("0.25.0")).expect("write");
    assert!(
        availability(h.path(), "qwen", true, Some("0.25.0"))
            .expect("read")
            .available()
    );
    delete_verified(h.path(), "qwen").expect("delete");
    let after = availability(h.path(), "qwen", true, Some("0.25.0")).expect("read");
    assert_eq!(blocker_desc(&after), "unverified");
}

#[test]
// REQ-091 precedence: not configured outranks a valid verified record.
fn not_configured_blocks_even_when_verified() {
    let h = tempfile::tempdir().expect("tempdir");
    write_verified(h.path(), "mistral", &rec("2.26.0")).expect("write");
    let a = availability(h.path(), "mistral", false, Some("2.26.0")).expect("read");
    assert!(!a.available());
    assert_eq!(blocker_desc(&a), "not_configured");
}

#[test]
// REQ-091: a suspension outranks a missing verification, so a suspended seat says suspended after its verified.json is gone.
fn suspended_outranks_missing_verification() {
    let h = tempfile::tempdir().expect("tempdir");
    write_suspended(h.path(), "kimi", "provider terminated access").expect("write");
    let a = availability(h.path(), "kimi", true, Some("2.2.0")).expect("read");
    assert!(!a.available());
    assert_eq!(blocker_desc(&a), "suspended:provider terminated access");
}

#[test]
// REQ-091: suspended.json blocks a seat even when verified.json is valid; only seat clear removes it.
fn suspended_blocks_until_clear() {
    let h = tempfile::tempdir().expect("tempdir");
    write_verified(h.path(), "kimi", &rec("2.2.0")).expect("write verified");
    write_suspended(h.path(), "kimi", "provider terminated access").expect("write suspended");
    let blocked = availability(h.path(), "kimi", true, Some("2.2.0")).expect("read");
    assert!(!blocked.available());
    assert_eq!(
        blocker_desc(&blocked),
        "suspended:provider terminated access"
    );

    clear_suspended(h.path(), "kimi").expect("clear");
    let cleared = availability(h.path(), "kimi", true, Some("2.2.0")).expect("read");
    assert!(cleared.available());
    assert_eq!(blocker_desc(&cleared), "none");
}

#[test]
// REQ-091: an unreadable verified.json is "not verified", never available.
fn unreadable_verified_reads_as_not_verified() {
    let h = tempfile::tempdir().expect("tempdir");
    let dir = h.path().join("seats").join("kimi");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("verified.json"), b"{ not json").expect("write garbage");
    let a = availability(h.path(), "kimi", true, Some("2.2.0")).expect("read");
    assert!(!a.verified);
    assert!(!a.available());
    assert_eq!(blocker_desc(&a), "unverified");
}

#[test]
// REQ-091: an unreadable suspended.json still counts as suspended (fails closed).
fn unreadable_suspended_still_blocks() {
    let h = tempfile::tempdir().expect("tempdir");
    write_verified(h.path(), "kimi", &rec("2.2.0")).expect("write verified");
    let dir = h.path().join("seats").join("kimi");
    std::fs::write(dir.join("suspended.json"), b"\x00\xff garbage").expect("write garbage");
    let a = availability(h.path(), "kimi", true, Some("2.2.0")).expect("read");
    assert!(
        a.suspended.is_some(),
        "unreadable suspended.json must read as suspended"
    );
    assert!(!a.available());
}
