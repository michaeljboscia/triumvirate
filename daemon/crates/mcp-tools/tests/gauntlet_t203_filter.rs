// GAUNTLET filter suite for T-203, written blind from the spec (no implementation read).
//
// SPEC_AMBIGUITY: the spec says seats_status covers "every seat that needs doctor --live verification" while
// BACKEND_STRUCTURE says "every supported seat". Only kimi, mistral and qwen (the A-2 new seats) are asserted present;
// whether existing seats (claude, codex, gemini, grok, deepseek) appear in the map is not tested.
// SPEC_AMBIGUITY: when a seat is not configured AND has a suspended.json, the spec fixes `reason` (precedence) but not
// whether `suspended` reads true; only `reason`, `available` and `configured` are asserted for that case.
// SPEC_AMBIGUITY: the StatusResponse type and its constructor are not in the listed API (get_status needs a daemon
// context), so the "seats map round-trips through StatusResponse" clause is tested on the map's own serialization only.
// SPEC_AMBIGUITY: accepted values of TRIUMVIRATE_ENABLE_<SEAT> other than "1" are unspecified and not tested.
// SPEC_AMBIGUITY: `triumvirate doctor` and `seat clear` live in the binary crate and are out of reach (suite scope).

use mcp_bridge::seat_gate::{installed_cli_version, register_installed_cli_version};
use mcp_tools::inter_agent::seats_status;
use shared_types::SeatStatus;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

// Process-global env is shared by every test in this binary, so each test holds this lock for its whole body.
static ENV_LOCK: Mutex<()> = Mutex::new(());

const SEATS: [&str; 3] = ["kimi", "mistral", "qwen"];

fn flag_name(seat: &str) -> String {
    format!("TRIUMVIRATE_ENABLE_{}", seat.to_uppercase())
}

struct Env {
    _lock: MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
    saved: Vec<(String, Option<String>)>,
}

impl Env {
    fn new() -> Env {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let mut keys: Vec<String> = SEATS.iter().map(|s| flag_name(s)).collect();
        keys.push("TRIUMVIRATE_HOME".to_string());
        let saved = keys
            .into_iter()
            .map(|k| {
                let v = std::env::var(&k).ok();
                (k, v)
            })
            .collect();
        for s in SEATS {
            unsafe { std::env::remove_var(flag_name(s)) };
        }
        unsafe { std::env::set_var("TRIUMVIRATE_HOME", dir.path()) };
        Env {
            _lock: lock,
            dir,
            saved,
        }
    }
    fn enable(&self, seat: &str) {
        unsafe { std::env::set_var(flag_name(seat), "1") };
    }
    fn disable(&self, seat: &str) {
        unsafe { std::env::remove_var(flag_name(seat)) };
    }
    fn seat_dir(&self, seat: &str) -> PathBuf {
        let d = self.dir.path().join("seats").join(seat);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn write_verified(&self, seat: &str, cli_version: &str) {
        let body = format!(
            "{{\"cli_version\":\"{cli_version}\",\"seat_home\":\"{}/homes/{seat}\",\"probed_at\":\"2026-10-18T12:00:00Z\",\"served_model\":null}}",
            self.dir.path().display()
        );
        std::fs::write(self.seat_dir(seat).join("verified.json"), body).unwrap();
    }
    fn write_suspended(&self, seat: &str, reason: &str) {
        let body = format!("{{\"reason\":\"{reason}\",\"at\":\"2026-10-18T12:00:00Z\"}}");
        std::fs::write(self.seat_dir(seat).join("suspended.json"), body).unwrap();
    }
    fn remove(&self, seat: &str, file: &str) {
        std::fs::remove_file(self.seat_dir(seat).join(file)).unwrap();
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            match v {
                Some(v) => unsafe { std::env::set_var(k, v) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
    }
}

fn status(seat: &str) -> SeatStatus {
    seats_status()
        .remove(seat)
        .unwrap_or_else(|| panic!("seat {seat} missing from seats_status"))
}

fn not_configured(seat: &str) -> String {
    format!(
        "seat_not_configured: {seat}: set TRIUMVIRATE_ENABLE_{}=1 and restart the daemon",
        seat.to_uppercase()
    )
}
fn unverified(seat: &str) -> String {
    format!("seat_unverified: {seat}: run triumvirate doctor --live in a terminal")
}
fn suspended(seat: &str, why: &str) -> String {
    format!("seat_suspended: {seat}: {why} (run: triumvirate seat clear {seat})")
}

// REQ-080, REQ-091: a fresh install lists every new seat as unconfigured with the exact refusal text.
#[test]
fn test_should_report_all_new_seats_not_configured_with_exact_reason_by_default() {
    let _env = Env::new();
    let map = seats_status();
    for seat in SEATS {
        let s = map.get(seat).unwrap_or_else(|| panic!("{seat} missing"));
        assert!(!s.configured, "{seat} configured");
        assert!(!s.verified, "{seat} verified");
        assert!(!s.suspended, "{seat} suspended");
        assert!(!s.available, "{seat} available");
        assert_eq!(s.reason.as_deref(), Some(not_configured(seat).as_str()));
    }
}

// Reality test: setting the env flag flips configured; unverified is then the reason.
#[test]
fn test_should_flip_configured_and_report_unverified_when_env_flag_set() {
    let env = Env::new();
    env.enable("kimi");
    let s = status("kimi");
    assert!(s.configured);
    assert!(!s.verified);
    assert!(!s.suspended);
    assert!(!s.available);
    assert_eq!(s.reason.as_deref(), Some(unverified("kimi").as_str()));
}

// REQ-091: the flag of one seat configures only that seat.
#[test]
fn test_should_not_configure_other_seats_when_one_flag_is_set() {
    let env = Env::new();
    env.enable("kimi");
    assert!(status("kimi").configured);
    assert!(!status("qwen").configured);
    assert!(!status("mistral").configured);
    assert_eq!(
        status("qwen").reason.as_deref(),
        Some(not_configured("qwen").as_str())
    );
}

// REQ-091: state is read now, so removing the flag unconfigures the seat again.
#[test]
fn test_should_unconfigure_seat_when_flag_removed_again() {
    let env = Env::new();
    env.enable("kimi");
    assert!(status("kimi").configured);
    env.disable("kimi");
    let s = status("kimi");
    assert!(!s.configured);
    assert_eq!(s.reason.as_deref(), Some(not_configured("kimi").as_str()));
}

// REQ-080: configured, verified (cli_version matches installed) and not suspended is available with a null reason.
#[test]
fn test_should_report_available_with_null_reason_when_verified_and_version_matches() {
    let env = Env::new();
    env.enable("mistral");
    register_installed_cli_version("mistral", "2.26.1");
    env.write_verified("mistral", "2.26.1");
    let s = status("mistral");
    assert!(s.configured && s.verified && s.available);
    assert!(!s.suspended);
    assert_eq!(s.reason, None);
}

// BACKEND_STRUCTURE s4: verified.json is treated as absent when cli_version differs from the installed version.
#[test]
fn test_should_treat_verified_as_absent_when_cli_version_differs() {
    let env = Env::new();
    env.enable("mistral");
    register_installed_cli_version("mistral", "2.26.2");
    env.write_verified("mistral", "2.26.1");
    let s = status("mistral");
    assert!(!s.verified);
    assert!(!s.available);
    assert_eq!(s.reason.as_deref(), Some(unverified("mistral").as_str()));
}

// A-8 / seat_gate docs: a seat with no registered version cannot match a verified.json (fails closed).
// qwen is never registered anywhere in this file.
#[test]
fn test_should_fail_closed_when_no_installed_version_registered() {
    let env = Env::new();
    env.enable("qwen");
    assert_eq!(installed_cli_version("qwen"), None);
    env.write_verified("qwen", "0.25.2");
    let s = status("qwen");
    assert!(!s.verified);
    assert!(!s.available);
    assert_eq!(s.reason.as_deref(), Some(unverified("qwen").as_str()));
}

// REQ-091: suspended.json makes the seat unavailable with the exact seat_suspended payload.
#[test]
fn test_should_report_suspended_with_exact_reason_text() {
    let env = Env::new();
    env.enable("mistral");
    register_installed_cli_version("mistral", "2.26.1");
    env.write_verified("mistral", "2.26.1");
    env.write_suspended("mistral", "quota exhausted");
    let s = status("mistral");
    assert!(s.configured);
    assert!(s.suspended);
    assert!(!s.available);
    assert_eq!(
        s.reason.as_deref(),
        Some(suspended("mistral", "quota exhausted").as_str())
    );
}

// REQ-091 precedence: suspended beats unverified.
#[test]
fn test_should_prefer_suspended_over_unverified_in_reason() {
    let env = Env::new();
    env.enable("kimi");
    env.write_suspended("kimi", "billing hold");
    let s = status("kimi");
    assert!(s.suspended);
    assert!(!s.verified);
    assert!(!s.available);
    assert_eq!(
        s.reason.as_deref(),
        Some(suspended("kimi", "billing hold").as_str())
    );
}

// REQ-091 precedence: not configured beats suspended.
#[test]
fn test_should_prefer_not_configured_over_suspended_in_reason() {
    let env = Env::new();
    env.write_suspended("kimi", "billing hold");
    let s = status("kimi");
    assert!(!s.configured);
    assert!(!s.available);
    assert_eq!(s.reason.as_deref(), Some(not_configured("kimi").as_str()));
}

// REQ-091: files are read per call (never cached): clearing suspended.json frees the seat at once.
#[test]
fn test_should_become_available_when_suspended_file_removed() {
    let env = Env::new();
    env.enable("mistral");
    register_installed_cli_version("mistral", "2.26.1");
    env.write_verified("mistral", "2.26.1");
    env.write_suspended("mistral", "quota");
    assert!(!status("mistral").available);
    env.remove("mistral", "suspended.json");
    let s = status("mistral");
    assert!(!s.suspended);
    assert!(s.available);
    assert_eq!(s.reason, None);
}

// REQ-054: deleting verified.json (failed probe) revokes verification on the next read.
#[test]
fn test_should_lose_verification_when_verified_file_deleted() {
    let env = Env::new();
    env.enable("mistral");
    register_installed_cli_version("mistral", "2.26.1");
    env.write_verified("mistral", "2.26.1");
    assert!(status("mistral").verified);
    env.remove("mistral", "verified.json");
    let s = status("mistral");
    assert!(!s.verified);
    assert!(!s.available);
    assert_eq!(s.reason.as_deref(), Some(unverified("mistral").as_str()));
}

// REQ-091: verified.json of one seat never verifies another seat.
#[test]
fn test_should_not_verify_seat_from_another_seats_file() {
    let env = Env::new();
    env.enable("kimi");
    env.enable("mistral");
    register_installed_cli_version("mistral", "2.26.1");
    register_installed_cli_version("kimi", "2.2.0");
    env.write_verified("mistral", "2.26.1");
    assert!(status("mistral").verified);
    assert!(!status("kimi").verified);
}

// REQ-091 fail closed: an unparseable verified.json is not verification.
#[test]
fn test_should_not_verify_when_verified_file_is_garbage() {
    let env = Env::new();
    env.enable("kimi");
    register_installed_cli_version("kimi", "2.2.0");
    std::fs::write(
        env.seat_dir("kimi").join("verified.json"),
        "not json at all",
    )
    .unwrap();
    let s = status("kimi");
    assert!(!s.verified);
    assert!(!s.available);
}

// REQ-091 precedence over all 8 combinations of (configured, verified, suspended) for one seat.
#[test]
fn test_should_follow_precedence_over_every_state_combination() {
    for bits in 0..8u8 {
        let (cfg, ver, sus) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0);
        let env = Env::new();
        register_installed_cli_version("kimi", "2.2.0");
        if cfg {
            env.enable("kimi");
        }
        if ver {
            env.write_verified("kimi", "2.2.0");
        }
        if sus {
            env.write_suspended("kimi", "why");
        }
        let s = status("kimi");
        let (want_reason, want_avail) = if !cfg {
            (Some(not_configured("kimi")), false)
        } else if sus {
            (Some(suspended("kimi", "why")), false)
        } else if !ver {
            (Some(unverified("kimi")), false)
        } else {
            (None, true)
        };
        assert_eq!(s.configured, cfg, "bits {bits}");
        assert_eq!(s.available, want_avail, "bits {bits}");
        assert_eq!(s.reason, want_reason, "bits {bits}");
        if cfg {
            assert_eq!(s.verified, ver, "bits {bits}");
            assert_eq!(s.suspended, sus, "bits {bits}");
        }
    }
}

// REQ-080 invariant: available implies reason is null; not available implies reason is a named refusal code.
#[test]
fn test_should_have_reason_iff_not_available() {
    let env = Env::new();
    env.enable("kimi");
    env.enable("mistral");
    register_installed_cli_version("mistral", "2.26.1");
    env.write_verified("mistral", "2.26.1");
    for (seat, s) in seats_status() {
        if s.available {
            assert_eq!(s.reason, None, "{seat}");
        } else {
            let r = s
                .reason
                .unwrap_or_else(|| panic!("{seat} unavailable without reason"));
            assert!(r.starts_with("seat_"), "{seat}: {r}");
        }
    }
}

// Reality test: the seats map serializes with the five documented keys and round-trips unchanged.
#[test]
fn test_should_serialize_seats_map_with_documented_keys_and_round_trip() {
    let env = Env::new();
    env.enable("mistral");
    register_installed_cli_version("mistral", "2.26.1");
    env.write_verified("mistral", "2.26.1");
    let map = seats_status();
    let v = serde_json::to_value(&map).unwrap();
    let kimi = v
        .get("kimi")
        .and_then(|x| x.as_object())
        .expect("kimi object");
    let mut keys: Vec<&str> = kimi.keys().map(|k| k.as_str()).collect();
    keys.sort();
    assert_eq!(
        keys,
        ["available", "configured", "reason", "suspended", "verified"]
    );
    assert_eq!(kimi["configured"], serde_json::json!(false));
    assert_eq!(kimi["reason"], serde_json::json!(not_configured("kimi")));
    assert_eq!(v["mistral"]["available"], serde_json::json!(true));
    assert!(v["mistral"]["reason"].is_null());
    let back: BTreeMap<String, SeatStatus> = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(serde_json::to_value(&back).unwrap(), v);
}
