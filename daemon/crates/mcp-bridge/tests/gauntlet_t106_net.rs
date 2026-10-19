//! GAUNTLET net suite for T-106, written blind from the spec (no implementation read).
//!
//! SPEC_AMBIGUITY: the testable form of the triumvirate_home rule ("taking the two variables as values") is named in a
//! doc comment but no signature is listed, so it is not tested directly. triumvirate_home() is tested through the env.
//! SPEC_AMBIGUITY: the atomic-write helper is described in a doc comment but has no listed signature. It is tested only
//! through write_verified and the directory listing after it.
//! SPEC_AMBIGUITY: the error or refusal for a seat key that tries to leave seats/ is not specified. The test requires only
//! that nothing lands outside seats/, whether the call returns Err or sanitises the key.
//! SPEC_AMBIGUITY: the exact byte layout of verified.json and suspended.json (compact or pretty) is not fixed beyond the
//! example. The tests parse the JSON instead of matching bytes.
//! SPEC_AMBIGUITY: the Suspended reason returned for an unreadable suspended.json is not specified. The test asserts only
//! that the seat is suspended.

use mcp_bridge::seat_state::{self, VerifiedRecord};
use std::fs;
use std::path::{Path, PathBuf};

const INSTALLED: &str = "2.2.0";

fn record(cli: &str, served: Option<&str>) -> VerifiedRecord {
    VerifiedRecord {
        cli_version: cli.to_string(),
        seat_home: "/Users/michaelboscia/.triumvirate/homes/kimi".to_string(),
        probed_at: "2026-10-18T12:00:00Z".to_string(),
        served_model: served.map(|s| s.to_string()),
    }
}

fn seat_dir(home: &Path, seat: &str) -> PathBuf {
    home.join("seats").join(seat)
}

/// Maps the first blocker to a stable word so the test does not need Debug or PartialEq on Blocker.
fn kind(a: &seat_state::SeatAvailability) -> &'static str {
    match a.first_blocker() {
        None => "available",
        Some(seat_state::Blocker::NotConfigured) => "not_configured",
        Some(seat_state::Blocker::Suspended(_)) => "suspended",
        Some(seat_state::Blocker::Unverified) => "unverified",
    }
}

fn suspended_reason_from_blocker(a: &seat_state::SeatAvailability) -> Option<String> {
    match a.first_blocker() {
        Some(seat_state::Blocker::Suspended(r)) => Some(r),
        _ => None,
    }
}

// REQ-091 (1b): verified.json is read per call; a write takes effect on the next read with no reinit.
#[test]
fn verified_write_takes_effect_on_next_read_without_reinit() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    let before = seat_state::availability(home, "kimi", true, Some(INSTALLED)).unwrap();
    assert!(!before.verified);
    assert!(before.suspended.is_none());
    assert!(!before.available());
    assert_eq!(kind(&before), "unverified");

    seat_state::write_verified(home, "kimi", &record(INSTALLED, Some("kimi-k2"))).unwrap();

    let after = seat_state::availability(home, "kimi", true, Some(INSTALLED)).unwrap();
    assert!(after.configured);
    assert!(after.verified);
    assert!(after.suspended.is_none());
    assert!(after.available());
    assert_eq!(kind(&after), "available");
}

// REQ-091 (2): a verified record whose cli_version differs from the installed version counts as absent.
#[test]
fn cli_version_change_flips_verified_to_false() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_verified(home, "kimi", &record("2.2.0", None)).unwrap();

    let same = seat_state::availability(home, "kimi", true, Some("2.2.0")).unwrap();
    assert!(same.verified);
    assert_eq!(kind(&same), "available");

    let bumped = seat_state::availability(home, "kimi", true, Some("2.3.0")).unwrap();
    assert!(!bumped.verified);
    assert!(!bumped.available());
    assert_eq!(kind(&bumped), "unverified");
}

// REQ-091 (2): an unknown installed CLI version cannot match any verified record.
#[test]
fn unknown_installed_version_cannot_match_verified_record() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_verified(home, "kimi", &record("2.2.0", None)).unwrap();

    let a = seat_state::availability(home, "kimi", true, None).unwrap();
    assert!(!a.verified);
    assert_eq!(kind(&a), "unverified");
}

// REQ-091 (3) and REQ-050: suspended.json yields Suspended ahead of Unverified when nothing is verified.
#[test]
fn suspended_outranks_unverified_and_carries_reason() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_suspended(home, "kimi", "403 account suspended").unwrap();

    let a = seat_state::availability(home, "kimi", true, Some(INSTALLED)).unwrap();
    assert!(!a.verified);
    assert_eq!(a.suspended.as_deref(), Some("403 account suspended"));
    assert!(!a.available());
    assert_eq!(kind(&a), "suspended");
    assert_eq!(suspended_reason_from_blocker(&a).as_deref(), Some("403 account suspended"));
}

// REQ-091 precedence: not configured outranks suspended and unverified.
#[test]
fn not_configured_outranks_suspended_and_unverified() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_suspended(home, "kimi", "quota exhausted").unwrap();

    let a = seat_state::availability(home, "kimi", false, Some(INSTALLED)).unwrap();
    assert!(!a.configured);
    assert!(!a.available());
    assert_eq!(kind(&a), "not_configured");
}

// REQ-091 precedence and REQ-050: a suspension outranks a missing verification after the failing probe deletes verified.json.
#[test]
fn suspension_survives_failed_probe_deleting_verified() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_verified(home, "kimi", &record(INSTALLED, None)).unwrap();
    seat_state::write_suspended(home, "kimi", "billing failure").unwrap();
    seat_state::delete_verified(home, "kimi").unwrap();

    let a = seat_state::availability(home, "kimi", true, Some(INSTALLED)).unwrap();
    assert!(!a.verified);
    assert_eq!(kind(&a), "suspended");
    assert_eq!(suspended_reason_from_blocker(&a).as_deref(), Some("billing failure"));
}

// REQ-050 and REQ-091 (4): only clear() removes suspended.json; a fresh verified record does not.
#[test]
fn only_clear_removes_suspension() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_suspended(home, "kimi", "403 account suspended").unwrap();

    seat_state::write_verified(home, "kimi", &record(INSTALLED, None)).unwrap();
    let still = seat_state::availability(home, "kimi", true, Some(INSTALLED)).unwrap();
    assert_eq!(kind(&still), "suspended");

    seat_state::delete_verified(home, "kimi").unwrap();
    seat_state::clear(home, "kimi").unwrap();
    let cleared = seat_state::availability(home, "kimi", true, Some(INSTALLED)).unwrap();
    assert!(cleared.suspended.is_none());
    assert_eq!(kind(&cleared), "unverified");

    seat_state::write_verified(home, "kimi", &record(INSTALLED, None)).unwrap();
    let ready = seat_state::availability(home, "kimi", true, Some(INSTALLED)).unwrap();
    assert!(ready.available());
    assert_eq!(kind(&ready), "available");
}

// REQ-054: delete_verified removes verified.json and leaves suspended.json alone.
#[test]
fn delete_verified_removes_only_verified_file() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_verified(home, "kimi", &record(INSTALLED, None)).unwrap();
    seat_state::write_suspended(home, "kimi", "403 account suspended").unwrap();

    seat_state::delete_verified(home, "kimi").unwrap();

    assert!(!seat_dir(home, "kimi").join("verified.json").exists());
    assert!(seat_dir(home, "kimi").join("suspended.json").exists());
}

// REQ-054 and BACKEND_STRUCTURE section 4: verified.json carries the four named fields.
#[test]
fn verified_json_has_the_named_fields_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_verified(home, "kimi", &record(INSTALLED, None)).unwrap();

    let bytes = fs::read(seat_dir(home, "kimi").join("verified.json")).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["cli_version"], "2.2.0");
    assert_eq!(v["seat_home"], "/Users/michaelboscia/.triumvirate/homes/kimi");
    assert_eq!(v["probed_at"], "2026-10-18T12:00:00Z");
    assert!(v["served_model"].is_null());
}

// REQ-091 and BACKEND_STRUCTURE section 4: a Some served_model is written as its string id.
#[test]
fn verified_json_records_served_model_when_present() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_verified(home, "kimi", &record(INSTALLED, Some("kimi-k2"))).unwrap();

    let bytes = fs::read(seat_dir(home, "kimi").join("verified.json")).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["served_model"], "kimi-k2");
}

// REQ-050 and BACKEND_STRUCTURE section 4: suspended.json is {"reason":...,"at":<RFC3339>}.
#[test]
fn suspended_json_has_reason_and_rfc3339_at() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_suspended(home, "kimi", "403 account suspended").unwrap();

    let bytes = fs::read(seat_dir(home, "kimi").join("suspended.json")).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["reason"], "403 account suspended");
    let at = v["at"].as_str().expect("at must be a string");
    assert!(chrono::DateTime::parse_from_rfc3339(at).is_ok(), "at is not RFC3339: {at}");
}

// REQ-091 fail closed: an unreadable verified.json is "not verified", not an error that makes the seat available.
#[test]
fn unreadable_verified_json_is_not_verified() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_verified(home, "kimi", &record(INSTALLED, None)).unwrap();
    fs::write(seat_dir(home, "kimi").join("verified.json"), b"{not json").unwrap();

    let a = seat_state::availability(home, "kimi", true, Some(INSTALLED)).unwrap();
    assert!(!a.verified);
    assert!(!a.available());
    assert_eq!(kind(&a), "unverified");
}

// REQ-091 fail closed: an unreadable suspended.json still means suspended.
#[test]
fn unreadable_suspended_json_is_still_suspended() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_verified(home, "kimi", &record(INSTALLED, None)).unwrap();
    fs::create_dir_all(seat_dir(home, "kimi")).unwrap();
    fs::write(seat_dir(home, "kimi").join("suspended.json"), b"\xff\xfe garbage").unwrap();

    let a = seat_state::availability(home, "kimi", true, Some(INSTALLED)).unwrap();
    assert!(a.suspended.is_some());
    assert!(!a.available());
    assert_eq!(kind(&a), "suspended");
}

// REQ-091 atomic writes: a successful write leaves only the target file, no sibling temp file.
#[test]
fn atomic_write_leaves_no_temp_sibling() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_verified(home, "kimi", &record(INSTALLED, None)).unwrap();

    let mut names: Vec<String> = fs::read_dir(seat_dir(home, "kimi"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["verified.json".to_string()]);
}

// REQ-091 atomic writes: a reader racing a writer never sees a half-written verified.json.
#[test]
fn concurrent_reader_never_sees_half_written_record() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_verified(home, "kimi", &record("2.2.0", None)).unwrap();
    let path = seat_dir(home, "kimi").join("verified.json");

    std::thread::scope(|s| {
        s.spawn(|| {
            for i in 0..300 {
                let cli = if i % 2 == 0 { "2.2.0" } else { "2.3.0" };
                seat_state::write_verified(home, "kimi", &record(cli, None)).unwrap();
            }
        });
        for _ in 0..2000 {
            let bytes = fs::read(&path).expect("verified.json must always exist after the first write");
            let v: serde_json::Value =
                serde_json::from_slice(&bytes).expect("torn read: verified.json was not complete JSON");
            let cli = v["cli_version"].as_str().expect("cli_version missing");
            assert!(cli == "2.2.0" || cli == "2.3.0", "unexpected cli_version: {cli}");
        }
    });
}

// REQ-091 and REQ-050: state is per seat; kimi's suspension does not affect mistral.
#[test]
fn seat_state_is_isolated_per_seat() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    seat_state::write_suspended(home, "kimi", "403 account suspended").unwrap();

    let mistral = seat_state::availability(home, "mistral", true, Some(INSTALLED)).unwrap();
    assert!(mistral.suspended.is_none());
    assert_eq!(kind(&mistral), "unverified");

    seat_state::write_verified(home, "mistral", &record(INSTALLED, None)).unwrap();
    let mistral_ready = seat_state::availability(home, "mistral", true, Some(INSTALLED)).unwrap();
    assert!(mistral_ready.available());

    let kimi = seat_state::availability(home, "kimi", true, Some(INSTALLED)).unwrap();
    assert_eq!(kind(&kimi), "suspended");
}

// Public API doc note (seat_state.rs): a seat key becomes a directory name and must not address anything outside seats/.
#[test]
fn seat_key_cannot_escape_seats_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();

    let _ = seat_state::write_suspended(&home, "../escape", "x");
    let _ = seat_state::write_verified(&home, "../escape", &record(INSTALLED, None));

    assert!(!tmp.path().join("escape").exists(), "write escaped the home directory");
    assert!(!home.join("escape").exists(), "write escaped seats/");
}

// REQ-091 env rule: TRIUMVIRATE_HOME wins when set; unset falls back to ~/.triumvirate.
// Only this test touches TRIUMVIRATE_HOME, so parallel tests cannot race it.
#[test]
fn triumvirate_home_uses_env_then_home_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let want = tmp.path().to_path_buf();

    unsafe { std::env::set_var("TRIUMVIRATE_HOME", &want) };
    let from_env = seat_state::triumvirate_home().unwrap();
    assert_eq!(from_env, want);

    unsafe { std::env::remove_var("TRIUMVIRATE_HOME") };
    let home_dir = std::env::var("HOME").expect("HOME must be set for this test");
    let fallback = seat_state::triumvirate_home().unwrap();
    assert_eq!(fallback, PathBuf::from(home_dir).join(".triumvirate"));
}

// REQ-091 env rule: only the exact value "1" enables a seat; a typo cannot turn it on.
// Uses a seat name no other test touches, so the env var is private to this test.
#[test]
fn is_configured_only_enables_on_exact_one() {
    let var = "TRIUMVIRATE_ENABLE_GAUNTLETPROBE";

    unsafe { std::env::remove_var(var) };
    assert!(!seat_state::is_configured("gauntletprobe").unwrap());

    unsafe { std::env::set_var(var, "1") };
    assert!(seat_state::is_configured("gauntletprobe").unwrap());

    for not_enabled in ["true", "yes", "0", "", "11"] {
        unsafe { std::env::set_var(var, not_enabled) };
        assert!(
            !seat_state::is_configured("gauntletprobe").unwrap(),
            "value {not_enabled:?} must not enable the seat"
        );
    }

    unsafe { std::env::remove_var(var) };
}
