// GAUNTLET tiebreaker suite for T-203, written blind from the spec (no implementation read).
// SPEC_AMBIGUITY: The listed API does not expose doctor static checks, the live probe runner, seat clear, or verified-file deletion after probe failure.
// SPEC_AMBIGUITY: The listed API exposes the seats map but not GET /status or a StatusResponse deserializer, so HTTP response round-trip behavior cannot be tested here.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use mcp_bridge::seat_gate::{installed_cli_version, register_installed_cli_version};
use mcp_tools::inter_agent::seats_status;
use serde_json::{Value, json};

static ENV_LOCK: Mutex<()> = Mutex::new(());

const ENABLE_KEYS: [&str; 3] = [
    "TRIUMVIRATE_ENABLE_KIMI",
    "TRIUMVIRATE_ENABLE_MISTRAL",
    "TRIUMVIRATE_ENABLE_QWEN",
];

struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl EnvGuard {
    fn isolated(home: &Path) -> Self {
        let mut saved = Vec::new();
        for key in std::iter::once("TRIUMVIRATE_HOME").chain(ENABLE_KEYS) {
            saved.push((key, std::env::var_os(key)));
        }
        unsafe {
            std::env::set_var("TRIUMVIRATE_HOME", home);
            for key in ENABLE_KEYS {
                std::env::remove_var(key);
            }
        }
        Self { saved }
    }

    fn enable(&self, seat: &str) {
        let key = format!("TRIUMVIRATE_ENABLE_{}", seat.to_ascii_uppercase());
        unsafe { std::env::set_var(key, "1") };
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

fn status_json() -> BTreeMap<String, Value> {
    serde_json::from_value(serde_json::to_value(seats_status()).unwrap()).unwrap()
}

fn write_verified(home: &Path, seat: &str, version: &str) {
    let state_dir = home.join("seats").join(seat);
    fs::create_dir_all(&state_dir).unwrap();
    let body = json!({
        "cli_version": version,
        "seat_home": home.join("homes").join(seat),
        "probed_at": "2026-10-18T12:00:00Z",
        "served_model": null
    });
    fs::write(
        state_dir.join("verified.json"),
        serde_json::to_vec(&body).unwrap(),
    )
    .unwrap();
}

// REQ-091: Installed CLI versions are absent until registered and then read back exactly.
#[test]
fn installed_cli_version_registry_round_trips_exact_values() {
    let seat = "t203-tiebreaker-registry-seat";
    assert_eq!(installed_cli_version(seat), None);
    register_installed_cli_version(seat, "2.26.1-test");
    assert_eq!(installed_cli_version(seat), Some("2.26.1-test".to_string()));
}

// REQ-080 and REQ-091: Status contains exactly the three doctor-verified seats and exact not-configured refusals.
#[test]
fn disabled_new_seats_report_exact_not_configured_status() {
    let _lock = ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let _env = EnvGuard::isolated(home.path());

    assert_eq!(
        status_json(),
        BTreeMap::from([
            (
                "kimi".to_string(),
                json!({
                    "configured": false,
                    "verified": false,
                    "suspended": false,
                    "available": false,
                    "reason": "seat_not_configured: kimi: set TRIUMVIRATE_ENABLE_KIMI=1 and restart the daemon"
                })
            ),
            (
                "mistral".to_string(),
                json!({
                    "configured": false,
                    "verified": false,
                    "suspended": false,
                    "available": false,
                    "reason": "seat_not_configured: mistral: set TRIUMVIRATE_ENABLE_MISTRAL=1 and restart the daemon"
                })
            ),
            (
                "qwen".to_string(),
                json!({
                    "configured": false,
                    "verified": false,
                    "suspended": false,
                    "available": false,
                    "reason": "seat_not_configured: qwen: set TRIUMVIRATE_ENABLE_QWEN=1 and restart the daemon"
                })
            ),
        ])
    );
}

// REQ-091: Enabling an unverified seat changes only configured state and yields the exact unverified refusal.
#[test]
fn enabled_seat_without_verification_reports_exact_unverified_status() {
    let _lock = ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let env = EnvGuard::isolated(home.path());
    env.enable("kimi");

    assert_eq!(
        status_json()["kimi"],
        json!({
            "configured": true,
            "verified": false,
            "suspended": false,
            "available": false,
            "reason": "seat_unverified: kimi: run triumvirate doctor --live in a terminal"
        })
    );
}

// REQ-080 and REQ-091: A configured seat verified against its registered CLI version is available.
#[test]
fn matching_registered_and_verified_versions_make_a_seat_available() {
    let _lock = ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let env = EnvGuard::isolated(home.path());
    env.enable("mistral");
    register_installed_cli_version("mistral", "2.26.1-tiebreaker");
    write_verified(home.path(), "mistral", "2.26.1-tiebreaker");

    assert_eq!(
        status_json()["mistral"],
        json!({
            "configured": true,
            "verified": true,
            "suspended": false,
            "available": true,
            "reason": null
        })
    );
}

// REQ-091: A CLI version mismatch invalidates an otherwise present verification record.
#[test]
fn cli_upgrade_invalidates_the_verified_record() {
    let _lock = ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let env = EnvGuard::isolated(home.path());
    env.enable("qwen");
    register_installed_cli_version("qwen", "0.25.3-tiebreaker");
    write_verified(home.path(), "qwen", "0.25.2-tiebreaker");

    assert_eq!(
        status_json()["qwen"],
        json!({
            "configured": true,
            "verified": false,
            "suspended": false,
            "available": false,
            "reason": "seat_unverified: qwen: run triumvirate doctor --live in a terminal"
        })
    );
}

// REQ-091: Suspension takes precedence over verification and reports the exact clearing instruction.
#[test]
fn suspension_precedes_verification_in_status() {
    let _lock = ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let env = EnvGuard::isolated(home.path());
    env.enable("mistral");
    register_installed_cli_version("mistral", "2.26.1-tiebreaker");
    write_verified(home.path(), "mistral", "2.26.1-tiebreaker");
    let state_dir = home.path().join("seats").join("mistral");
    fs::write(
        state_dir.join("suspended.json"),
        br#"{"reason":"quota exhausted","at":"2026-10-18T12:00:00Z"}"#,
    )
    .unwrap();

    assert_eq!(
        status_json()["mistral"],
        json!({
            "configured": true,
            "verified": true,
            "suspended": true,
            "available": false,
            "reason": "seat_suspended: mistral: quota exhausted (run: triumvirate seat clear mistral)"
        })
    );
}
