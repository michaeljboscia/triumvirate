//! GAUNTLET net suite for T-203, written blind from the spec (no implementation read).
//!
//! SPEC_AMBIGUITY: `seat_gate_with` takes a `&SeatAvailability`, but the public API listing gives no constructor or
//! fields for it, so the pure decision cannot be driven directly. Every gate case here goes through `seat_gate` with
//! TRIUMVIRATE_HOME pointed at a tempdir (the state source the spec names for `seat_gate`).
//! SPEC_AMBIGUITY: the Surface and Origin variant lists are taken from BACKEND_STRUCTURE.md section 3 (the listing shows
//! only the imports).
//! SPEC_AMBIGUITY: the "<surface>" segment of `seat_surface_deferred` is rendered as the variant name, following the
//! A-2 example `qwen on Prewarm ships in v1.1`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use mcp_bridge::seat_gate::{
    Origin, Refusal, RefusalCode, Surface, installed_cli_version, register_installed_cli_version,
    seat_gate,
};

const ENV_KEYS: [&str; 4] = [
    "TRIUMVIRATE_HOME",
    "TRIUMVIRATE_ENABLE_KIMI",
    "TRIUMVIRATE_ENABLE_MISTRAL",
    "TRIUMVIRATE_ENABLE_QWEN",
];

/// Launch seats and the installed CLI pins (A-9): kimi 2.2.0, vibe 2.26.1, qwen 0.25.2.
const LAUNCH_SEATS: [(&str, &str); 3] =
    [("kimi", "2.2.0"), ("mistral", "2.26.1"), ("qwen", "0.25.2")];

const ALLOWED_SURFACES: [&str; 4] = ["AskAgent", "Session", "Jury", "DoctorLiveProbe"];
const DEFERRED_SURFACES: [&str; 5] = [
    "PeerReviewRotation",
    "FleetSpawn",
    "FleetTemporal",
    "AbeDispatch",
    "Prewarm",
];
const ALL_SURFACES: [&str; 10] = [
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
];

static LOCK: Mutex<()> = Mutex::new(());

struct EnvRestore(Vec<(&'static str, Option<OsString>)>);

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (key, value) in &self.0 {
            // SAFETY: every test holds LOCK, so no other test thread touches the environment.
            unsafe {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

struct Fixture {
    // Field order matters: the environment is restored before the lock is released.
    _restore: EnvRestore,
    _lock: MutexGuard<'static, ()>,
    home: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let lock = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let saved: Vec<(&'static str, Option<OsString>)> =
        ENV_KEYS.iter().map(|k| (*k, std::env::var_os(k))).collect();
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: serialized by LOCK; restored by EnvRestore on drop.
    unsafe {
        for k in ENV_KEYS {
            std::env::remove_var(k);
        }
        std::env::set_var("TRIUMVIRATE_HOME", home.path());
    }
    Fixture {
        _restore: EnvRestore(saved),
        _lock: lock,
        home,
    }
}

fn enable(seat: &str) {
    let key = format!("TRIUMVIRATE_ENABLE_{}", seat.to_uppercase());
    // SAFETY: serialized by LOCK.
    unsafe { std::env::set_var(&key, "1") };
}

fn seat_dir(home: &Path, seat: &str) -> PathBuf {
    let dir = home.join("seats").join(seat);
    std::fs::create_dir_all(&dir).expect("mkdir seat dir");
    dir
}

fn write_verified(home: &Path, seat: &str, cli_version: &str) {
    let body = format!(
        r#"{{"cli_version":"{cli_version}","seat_home":"{}","probed_at":"2026-10-18T12:00:00Z","served_model":null}}"#,
        home.display()
    );
    std::fs::write(seat_dir(home, seat).join("verified.json"), body).expect("write verified.json");
}

fn write_suspended(home: &Path, seat: &str, reason: &str) {
    let body = format!(r#"{{"reason":"{reason}","at":"2026-10-18T12:00:00Z"}}"#);
    std::fs::write(seat_dir(home, seat).join("suspended.json"), body)
        .expect("write suspended.json");
}

/// Configured, registered at `version`, and verified against that version.
fn make_available(f: &Fixture, seat: &str, version: &str) {
    enable(seat);
    register_installed_cli_version(seat, version);
    write_verified(f.home.path(), seat, version);
}

fn surface(name: &str) -> Surface {
    match name {
        "AskAgent" => Surface::AskAgent,
        "Session" => Surface::Session,
        "Jury" => Surface::Jury,
        "BlindValidate" => Surface::BlindValidate,
        "DoctorLiveProbe" => Surface::DoctorLiveProbe,
        "PeerReviewRotation" => Surface::PeerReviewRotation,
        "FleetSpawn" => Surface::FleetSpawn,
        "FleetTemporal" => Surface::FleetTemporal,
        "AbeDispatch" => Surface::AbeDispatch,
        "Prewarm" => Surface::Prewarm,
        other => panic!("unknown surface name in suite: {other}"),
    }
}

fn origin(name: &str) -> Origin {
    match name {
        "Interactive" => Origin::Interactive,
        "Unattended" => Origin::Unattended,
        other => panic!("unknown origin name in suite: {other}"),
    }
}

fn expect_ok(res: Result<(), Refusal>, ctx: &str) {
    if let Err(r) = res {
        panic!("{ctx}: expected Ok, got {}: {}", r.code.as_str(), r.message);
    }
}

fn expect_refused(res: Result<(), Refusal>, ctx: &str) -> (&'static str, String) {
    match res {
        Err(r) => {
            let code = r.code.as_str();
            (code, r.message)
        }
        Ok(()) => panic!("{ctx}: expected a refusal, got Ok"),
    }
}

// ---------- REQ-091: refusal code strings (payload prefix) ----------

/// REQ-091: each refusal code renders its exact snake_case string, which starts every payload.
#[test]
fn refusal_codes_render_exact_strings() {
    assert_eq!(
        RefusalCode::SeatNotConfigured.as_str(),
        "seat_not_configured"
    );
    assert_eq!(RefusalCode::SeatSuspended.as_str(), "seat_suspended");
    assert_eq!(RefusalCode::SeatUnverified.as_str(), "seat_unverified");
    assert_eq!(
        RefusalCode::SeatSurfaceRefused.as_str(),
        "seat_surface_refused"
    );
    assert_eq!(
        RefusalCode::SeatSurfaceDeferred.as_str(),
        "seat_surface_deferred"
    );
    assert_eq!(
        RefusalCode::SeatCannotWriteHere.as_str(),
        "seat_cannot_write_here"
    );
}

// ---------- REQ-091: installed version registry (fails closed) ----------

/// REQ-091: a seat with no registered installed version reads as None, so it can never match a verified.json.
#[test]
fn unregistered_seat_has_no_installed_version() {
    let _f = fixture();
    assert_eq!(installed_cli_version("seat-never-registered-t203"), None);
}

/// REQ-091: registering stores the exact version string, and a second registration replaces the first.
#[test]
fn registered_version_reads_back_and_overwrites() {
    let _f = fixture();
    register_installed_cli_version("kimi", "2.2.0");
    assert_eq!(installed_cli_version("kimi"), Some("2.2.0".to_string()));
    register_installed_cli_version("kimi", "2.2.1");
    assert_eq!(installed_cli_version("kimi"), Some("2.2.1".to_string()));
    register_installed_cli_version("kimi", "2.2.0");
}

// ---------- REQ-091: precedence ----------

/// REQ-091 precedence 1: an unconfigured seat refuses with the exact payload, whatever the surface.
#[test]
fn not_configured_refuses_with_exact_payload() {
    let _f = fixture();
    let (code, msg) = expect_refused(
        seat_gate("kimi", Surface::AskAgent, Origin::Interactive, false),
        "kimi ask",
    );
    assert_eq!(code, "seat_not_configured");
    assert_eq!(
        msg,
        "seat_not_configured: kimi: set TRIUMVIRATE_ENABLE_KIMI=1 and restart the daemon"
    );
}

/// REQ-091 precedence 1 over the surface rule: unconfigured qwen on a deferred surface is still not_configured.
#[test]
fn not_configured_beats_deferred_surface() {
    let _f = fixture();
    let (code, msg) = expect_refused(
        seat_gate("qwen", Surface::Prewarm, Origin::Unattended, false),
        "qwen prewarm",
    );
    assert_eq!(code, "seat_not_configured");
    assert_eq!(
        msg,
        "seat_not_configured: qwen: set TRIUMVIRATE_ENABLE_QWEN=1 and restart the daemon"
    );
}

/// REQ-091 precedence 3 and A-2: configured but unverified refuses ask, while DoctorLiveProbe (how a seat gets
/// verified) is allowed both with and without a TTY.
#[test]
fn unverified_refuses_ask_but_live_probe_is_allowed() {
    let f = fixture();
    enable("kimi");
    register_installed_cli_version("kimi", "2.2.0");
    let (code, msg) = expect_refused(
        seat_gate("kimi", Surface::AskAgent, Origin::Interactive, true),
        "kimi ask",
    );
    assert_eq!(code, "seat_unverified");
    assert_eq!(
        msg,
        "seat_unverified: kimi: run triumvirate doctor --live in a terminal"
    );
    expect_ok(
        seat_gate("kimi", Surface::DoctorLiveProbe, Origin::Interactive, true),
        "probe with tty",
    );
    expect_ok(
        seat_gate("kimi", Surface::DoctorLiveProbe, Origin::Unattended, false),
        "probe without tty (A-2)",
    );
    assert!(!f.home.path().join("seats/kimi/verified.json").exists());
}

/// REQ-091 precedence 2: suspended beats unverified, and suspension also refuses the live probe.
#[test]
fn suspended_beats_unverified_and_applies_to_probe() {
    let f = fixture();
    enable("kimi");
    register_installed_cli_version("kimi", "2.2.0");
    write_suspended(f.home.path(), "kimi", "quota exhausted");
    let (code, msg) = expect_refused(
        seat_gate("kimi", Surface::AskAgent, Origin::Interactive, false),
        "kimi ask",
    );
    assert_eq!(code, "seat_suspended");
    assert_eq!(
        msg,
        "seat_suspended: kimi: quota exhausted (run: triumvirate seat clear kimi)"
    );
    let (probe_code, _) = expect_refused(
        seat_gate("kimi", Surface::DoctorLiveProbe, Origin::Interactive, true),
        "probe",
    );
    assert_eq!(probe_code, "seat_suspended");
}

/// REQ-091: a verified.json whose cli_version differs from the installed version is treated as absent.
#[test]
fn cli_version_change_flips_verified_to_unverified() {
    let f = fixture();
    make_available(&f, "mistral", "2.26.1");
    expect_ok(
        seat_gate("mistral", Surface::AskAgent, Origin::Unattended, false),
        "mistral ask before upgrade",
    );
    register_installed_cli_version("mistral", "2.27.0");
    let (code, msg) = expect_refused(
        seat_gate("mistral", Surface::AskAgent, Origin::Unattended, false),
        "after upgrade",
    );
    assert_eq!(code, "seat_unverified");
    assert_eq!(
        msg,
        "seat_unverified: mistral: run triumvirate doctor --live in a terminal"
    );
    register_installed_cli_version("mistral", "2.26.1");
}

/// REQ-091: a passing doctor --live takes effect with no restart (no caching of the state files).
#[test]
fn passing_probe_takes_effect_without_restart() {
    let f = fixture();
    enable("qwen");
    register_installed_cli_version("qwen", "0.25.2");
    expect_refused(
        seat_gate("qwen", Surface::Session, Origin::Interactive, false),
        "before probe",
    );
    write_verified(f.home.path(), "qwen", "0.25.2");
    expect_ok(
        seat_gate("qwen", Surface::Session, Origin::Interactive, false),
        "after probe",
    );
}

// ---------- REQ-090 / A-2: surface table ----------

/// A-2 table, all launch seats, every (surface, origin, tty) triple. Zero triples may yield seat_surface_refused.
#[test]
fn surface_table_matches_amendment_a2_for_every_launch_seat() {
    let f = fixture();
    for (seat, version) in LAUNCH_SEATS {
        make_available(&f, seat, version);
    }
    let mut checked = 0usize;
    for (seat, _) in LAUNCH_SEATS {
        for origin_name in ["Interactive", "Unattended"] {
            for tty in [true, false] {
                for surface_name in ALL_SURFACES {
                    let ctx = format!("{seat} {surface_name} {origin_name} tty={tty}");
                    let res = seat_gate(seat, surface(surface_name), origin(origin_name), tty);
                    if ALLOWED_SURFACES.contains(&surface_name) {
                        expect_ok(res, &ctx);
                    } else if surface_name == "BlindValidate" {
                        let (code, msg) = expect_refused(res, &ctx);
                        assert_eq!(code, "seat_cannot_write_here", "{ctx}");
                        assert_eq!(
                            msg,
                            format!(
                                "seat_cannot_write_here: {seat}: blind validators must write test files and {seat} is read-only"
                            ),
                            "{ctx}"
                        );
                    } else {
                        assert!(
                            DEFERRED_SURFACES.contains(&surface_name),
                            "{ctx}: unclassified surface"
                        );
                        let (code, msg) = expect_refused(res, &ctx);
                        assert_eq!(code, "seat_surface_deferred", "{ctx}");
                        assert_eq!(
                            msg,
                            format!(
                                "seat_surface_deferred: {seat} on {surface_name} ships in v1.1"
                            ),
                            "{ctx}"
                        );
                    }
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 3 * 2 * 2 * 10, "triple count");
}

/// REQ-090 spot check: an unattended ask to a verified seat spawns (passes the gate), per A-2.
#[test]
fn unattended_ask_on_verified_seat_is_allowed() {
    let f = fixture();
    make_available(&f, "kimi", "2.2.0");
    expect_ok(
        seat_gate("kimi", Surface::AskAgent, Origin::Unattended, false),
        "kimi unattended ask",
    );
    expect_ok(
        seat_gate("kimi", Surface::Jury, Origin::Unattended, false),
        "kimi unattended jury",
    );
    assert!(f.home.path().join("seats/kimi/verified.json").exists());
}
