//! GAUNTLET net suite for T-203, written blind from the spec (no implementation read).
//!
//! SPEC_AMBIGUITY: `get_status` takes a `Sessions` (`Arc<Mutex<HashMap<..>>>`) whose Mutex flavour (std or tokio) and
//! the `StatusResponse` field name for `seats` are not in the public API listing, so the /status round-trip cannot be
//! driven here. The seats map is checked through `seats_status()` serialized with serde_json, which is the same wire shape.
//! SPEC_AMBIGUITY: `SeatStatus` (shared_types) is listed by name only. This suite assumes it serializes to the JSON keys
//! BACKEND_STRUCTURE.md section 4 gives: configured, verified, suspended, available, reason.
//! SPEC_AMBIGUITY: `mcp_tools::inter_agent` is assumed to be a public module.
//! SPEC_AMBIGUITY: the `seat clear` CLI (removes suspended.json) lives in the triumvirate binary crate and is out of reach here.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use mcp_bridge::seat_gate::register_installed_cli_version;
use mcp_tools::inter_agent::seats_status;
use serde_json::{Value, json};

const ENV_KEYS: [&str; 4] = [
    "TRIUMVIRATE_HOME",
    "TRIUMVIRATE_ENABLE_KIMI",
    "TRIUMVIRATE_ENABLE_MISTRAL",
    "TRIUMVIRATE_ENABLE_QWEN",
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

/// The `seats` map as /status serves it, for one seat.
fn status_of(seat: &str) -> Value {
    let all = serde_json::to_value(seats_status()).expect("seats map serializes");
    all.get(seat)
        .cloned()
        .unwrap_or_else(|| panic!("seats map has no entry for {seat}"))
}

/// REQ-080: the seats map covers exactly the three launch seats, and each entry carries exactly the five keys.
#[test]
fn seats_map_lists_the_launch_seats_with_five_keys_each() {
    let _f = fixture();
    let all = serde_json::to_value(seats_status()).expect("seats map serializes");
    let keys: Vec<String> = all
        .as_object()
        .expect("seats is an object")
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        keys,
        vec![
            "kimi".to_string(),
            "mistral".to_string(),
            "qwen".to_string()
        ]
    );
    for seat in ["kimi", "mistral", "qwen"] {
        let entry = status_of(seat);
        let mut entry_keys: Vec<String> = entry
            .as_object()
            .expect("entry is an object")
            .keys()
            .cloned()
            .collect();
        entry_keys.sort();
        assert_eq!(
            entry_keys,
            vec!["available", "configured", "reason", "suspended", "verified"],
            "{seat}"
        );
    }
}

/// REQ-080 / REQ-091 precedence 1: with no env flag, the seat reads not configured and the reason is the exact payload.
#[test]
fn unconfigured_seat_reports_exact_reason() {
    let _f = fixture();
    let s = status_of("kimi");
    assert_eq!(s["configured"], json!(false));
    assert_eq!(s["verified"], json!(false));
    assert_eq!(s["suspended"], json!(false));
    assert_eq!(s["available"], json!(false));
    assert_eq!(
        s["reason"],
        json!("seat_not_configured: kimi: set TRIUMVIRATE_ENABLE_KIMI=1 and restart the daemon")
    );
}

/// REQ-080 / REQ-091 precedence 3: the env flag flips configured, and with no verified.json the reason is unverified.
#[test]
fn enabled_seat_without_verification_reports_unverified() {
    let _f = fixture();
    enable("kimi");
    register_installed_cli_version("kimi", "2.2.0");
    let s = status_of("kimi");
    assert_eq!(s["configured"], json!(true));
    assert_eq!(s["verified"], json!(false));
    assert_eq!(s["available"], json!(false));
    assert_eq!(
        s["reason"],
        json!("seat_unverified: kimi: run triumvirate doctor --live in a terminal")
    );
}

/// REQ-091 precedence 2: a suspended seat reports suspended, and that beats the unverified reason.
#[test]
fn suspended_seat_reports_suspension_reason() {
    let f = fixture();
    enable("mistral");
    register_installed_cli_version("mistral", "2.26.1");
    write_suspended(f.home.path(), "mistral", "quota exhausted");
    let s = status_of("mistral");
    assert_eq!(s["configured"], json!(true));
    assert_eq!(s["verified"], json!(false));
    assert_eq!(s["suspended"], json!(true));
    assert_eq!(s["available"], json!(false));
    assert_eq!(
        s["reason"],
        json!("seat_suspended: mistral: quota exhausted (run: triumvirate seat clear mistral)")
    );
}

/// REQ-091: configured, verified at the installed version and not suspended means available, with reason null.
#[test]
fn verified_seat_is_available_with_null_reason() {
    let f = fixture();
    enable("qwen");
    register_installed_cli_version("qwen", "0.25.2");
    write_verified(f.home.path(), "qwen", "0.25.2");
    let s = status_of("qwen");
    assert_eq!(s["configured"], json!(true));
    assert_eq!(s["verified"], json!(true));
    assert_eq!(s["suspended"], json!(false));
    assert_eq!(s["available"], json!(true));
    assert_eq!(s["reason"], json!(null));
}

/// REQ-091: a verified.json for a different CLI version does not count; the seat reads unverified.
#[test]
fn verified_record_for_old_cli_version_does_not_count() {
    let f = fixture();
    enable("qwen");
    register_installed_cli_version("qwen", "0.25.3");
    write_verified(f.home.path(), "qwen", "0.25.2");
    let s = status_of("qwen");
    assert_eq!(s["verified"], json!(false));
    assert_eq!(s["available"], json!(false));
    assert_eq!(
        s["reason"],
        json!("seat_unverified: qwen: run triumvirate doctor --live in a terminal")
    );
}

/// REQ-091: state is read now, not cached at start, so a new verified record shows without a restart.
#[test]
fn status_reflects_a_probe_pass_without_restart() {
    let f = fixture();
    enable("kimi");
    register_installed_cli_version("kimi", "2.2.0");
    assert_eq!(status_of("kimi")["verified"], json!(false));
    write_verified(f.home.path(), "kimi", "2.2.0");
    let s = status_of("kimi");
    assert_eq!(s["verified"], json!(true));
    assert_eq!(s["available"], json!(true));
    assert_eq!(s["reason"], json!(null));
}
