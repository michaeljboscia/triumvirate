// GAUNTLET tiebreaker suite for T-106, written blind from the spec (no implementation read).
// SPEC_AMBIGUITY: The spec describes a pure environment resolution helper for triumvirate_home but does not list its public signature, so environment precedence cannot be tested without mutating process global state.
// SPEC_AMBIGUITY: The spec does not define exact error text for invalid seat keys or filesystem failures, so those tests assert rejection and containment instead of an unspecified message.
// SPEC_AMBIGUITY: The spec requires atomic writes but does not expose the atomic writer or define its temporary filename, so the public write operations are checked for complete final content and absence of leftover sibling files.

use mcp_bridge::seat_state::{
    Blocker, SeatAvailability, VerifiedRecord, availability, clear, delete_verified,
    write_suspended, write_verified,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TestHome {
    path: PathBuf,
}

impl TestHome {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "triumvirate-t106-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create isolated test home");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn seat_dir(&self, seat: &str) -> PathBuf {
        self.path.join("seats").join(seat)
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn record(version: &str) -> VerifiedRecord {
    VerifiedRecord {
        cli_version: version.to_owned(),
        seat_home: "/isolated/homes/kimi".to_owned(),
        probed_at: "2026-10-18T12:34:56Z".to_owned(),
        served_model: Some("kimi-for-coding".to_owned()),
    }
}

fn assert_suspended(blocker: Option<Blocker>, expected_reason: &str) {
    match blocker {
        Some(Blocker::Suspended(reason)) => assert_eq!(reason, expected_reason),
        Some(Blocker::NotConfigured) => panic!("expected suspended blocker, got not configured"),
        Some(Blocker::Unverified) => panic!("expected suspended blocker, got unverified"),
        None => panic!("expected suspended blocker, got no blocker"),
    }
}

fn assert_not_configured(blocker: Option<Blocker>) {
    match blocker {
        Some(Blocker::NotConfigured) => {}
        Some(Blocker::Suspended(reason)) => {
            panic!("expected not configured blocker, got suspended: {reason}")
        }
        Some(Blocker::Unverified) => panic!("expected not configured blocker, got unverified"),
        None => panic!("expected not configured blocker, got no blocker"),
    }
}

fn assert_unverified(blocker: Option<Blocker>) {
    match blocker {
        Some(Blocker::Unverified) => {}
        Some(Blocker::NotConfigured) => panic!("expected unverified blocker, got not configured"),
        Some(Blocker::Suspended(reason)) => {
            panic!("expected unverified blocker, got suspended: {reason}")
        }
        None => panic!("expected unverified blocker, got no blocker"),
    }
}

fn assert_invalid_seat<T>(result: Result<T, String>, operation: &str) {
    match result {
        Err(message) => assert!(
            !message.trim().is_empty(),
            "{operation} rejected the invalid seat with an empty error"
        ),
        Ok(_) => panic!("{operation} accepted a seat key that escapes seats/"),
    }
}

// REQ-091: available means configured AND verified AND NOT suspended.
#[test]
fn available_obeys_the_complete_truth_table() {
    for configured in [false, true] {
        for verified in [false, true] {
            for suspended in [false, true] {
                let state = SeatAvailability {
                    configured,
                    verified,
                    suspended: suspended.then(|| "account disabled".to_owned()),
                };
                assert_eq!(
                    state.available(),
                    configured && verified && !suspended,
                    "configured={configured}, verified={verified}, suspended={suspended}"
                );
            }
        }
    }
}

// REQ-091: blocker precedence is not configured, then suspended, then unverified.
#[test]
fn first_blocker_follows_required_precedence() {
    let not_configured = SeatAvailability {
        configured: false,
        verified: false,
        suspended: Some("billing hold".to_owned()),
    };
    assert_not_configured(not_configured.first_blocker());

    let suspended = SeatAvailability {
        configured: true,
        verified: false,
        suspended: Some("billing hold".to_owned()),
    };
    assert_suspended(suspended.first_blocker(), "billing hold");

    let unverified = SeatAvailability {
        configured: true,
        verified: false,
        suspended: None,
    };
    assert_unverified(unverified.first_blocker());

    let available = SeatAvailability {
        configured: true,
        verified: true,
        suspended: None,
    };
    assert!(
        available.first_blocker().is_none(),
        "an available seat must have no blocker"
    );
}

// REQ-091 acceptance 1b: a matching verified record makes a configured seat available.
#[test]
fn matching_cli_version_makes_configured_seat_available() {
    let home = TestHome::new("matching-version");
    write_verified(home.path(), "kimi", &record("2.2.0")).expect("write verified record");

    let state =
        availability(home.path(), "kimi", true, Some("2.2.0")).expect("read availability");

    assert!(state.configured, "configured input must be preserved");
    assert!(state.verified, "matching CLI version must verify the seat");
    assert_eq!(state.suspended, None);
    assert!(state.available(), "matching verified seat must be available");
    assert!(
        state.first_blocker().is_none(),
        "available seat must have no blocker"
    );
}

// REQ-091 acceptance 3: a CLI version change invalidates verified.json.
#[test]
fn changed_or_unknown_cli_version_invalidates_verification() {
    let home = TestHome::new("version-invalidation");
    write_verified(home.path(), "kimi", &record("2.2.0")).expect("write verified record");

    let changed =
        availability(home.path(), "kimi", true, Some("2.2.1")).expect("read changed version");
    assert!(!changed.verified, "different installed version must invalidate");
    assert!(!changed.available());
    assert_unverified(changed.first_blocker());

    let unknown =
        availability(home.path(), "kimi", true, None).expect("read unknown installed version");
    assert!(
        !unknown.verified,
        "unknown installed version must invalidate every verified record"
    );
    assert!(!unknown.available());
    assert_unverified(unknown.first_blocker());
}

// REQ-091 acceptance 1b: verified.json is read on every availability call and is never cached.
#[test]
fn verified_state_is_observed_on_the_next_call_without_reinitialization() {
    let home = TestHome::new("verified-per-call");

    let before =
        availability(home.path(), "kimi", true, Some("2.2.0")).expect("read initial availability");
    assert!(!before.verified);
    assert!(!before.available());

    write_verified(home.path(), "kimi", &record("2.2.0")).expect("write verified record");

    let after =
        availability(home.path(), "kimi", true, Some("2.2.0")).expect("read updated availability");
    assert!(after.verified);
    assert!(after.available());
}

// REQ-091 acceptance 2 and 4: suspended.json is read per call and suspension outranks unverified.
#[test]
fn suspension_is_observed_per_call_and_outranks_missing_verification() {
    let home = TestHome::new("suspension-precedence");

    let before =
        availability(home.path(), "qwen", true, Some("0.25.2")).expect("read initial availability");
    assert!(!before.verified);
    assert_eq!(before.suspended, None);
    assert_unverified(before.first_blocker());

    write_suspended(home.path(), "qwen", "provider account suspended")
        .expect("write suspension");

    let after = availability(home.path(), "qwen", true, Some("0.25.2"))
        .expect("read suspended availability");
    assert!(!after.verified);
    assert_eq!(
        after.suspended.as_deref(),
        Some("provider account suspended")
    );
    assert!(!after.available());
    assert_suspended(after.first_blocker(), "provider account suspended");
}

// REQ-054: a spawned probe failure deletes verified.json but does not clear suspension.
#[test]
fn failed_spawned_probe_deletes_verification_but_preserves_suspension() {
    let home = TestHome::new("failed-probe");
    write_verified(home.path(), "mistral", &record("2.26.1"))
        .expect("write verified record");
    write_suspended(home.path(), "mistral", "subscription disabled")
        .expect("write suspension");

    delete_verified(home.path(), "mistral").expect("delete verified record");

    assert!(
        !home.seat_dir("mistral").join("verified.json").exists(),
        "failed spawned probe must remove verified.json"
    );
    assert!(
        home.seat_dir("mistral").join("suspended.json").exists(),
        "failed spawned probe must preserve suspended.json"
    );

    let state = availability(home.path(), "mistral", true, Some("2.26.1"))
        .expect("read state after failed probe");
    assert!(!state.verified);
    assert_eq!(state.suspended.as_deref(), Some("subscription disabled"));
    assert_suspended(state.first_blocker(), "subscription disabled");
}

// REQ-091 acceptance 4: clear removes suspended.json while preserving verified.json.
#[test]
fn clear_removes_only_suspension_and_restores_verified_availability() {
    let home = TestHome::new("clear");
    write_verified(home.path(), "kimi", &record("2.2.0")).expect("write verified record");
    write_suspended(home.path(), "kimi", "billing hold").expect("write suspension");

    clear(home.path(), "kimi").expect("clear suspension");

    assert!(
        home.seat_dir("kimi").join("verified.json").exists(),
        "clear must preserve verified.json"
    );
    assert!(
        !home.seat_dir("kimi").join("suspended.json").exists(),
        "clear must remove suspended.json"
    );

    let state =
        availability(home.path(), "kimi", true, Some("2.2.0")).expect("read cleared state");
    assert!(state.configured);
    assert!(state.verified);
    assert_eq!(state.suspended, None);
    assert!(state.available());
    assert!(state.first_blocker().is_none());
}

// REQ-091 fail-closed rule: malformed verified state is unverified and unreadable suspension remains blocking.
#[test]
fn malformed_state_files_fail_closed() {
    let home = TestHome::new("fail-closed");
    let seat_dir = home.seat_dir("qwen");
    fs::create_dir_all(&seat_dir).expect("create seat directory");
    fs::write(seat_dir.join("verified.json"), b"{not valid json")
        .expect("write malformed verified file");
    fs::create_dir(seat_dir.join("suspended.json"))
        .expect("make suspended path unreadable as a regular file");

    let state =
        availability(home.path(), "qwen", true, Some("0.25.2")).expect("read fail-closed state");

    assert!(
        !state.verified,
        "unreadable verified.json must not verify the seat"
    );
    assert!(
        state.suspended.is_some(),
        "unreadable suspended.json must still suspend the seat"
    );
    assert!(!state.available());
    match state.first_blocker() {
        Some(Blocker::Suspended(_)) => {}
        Some(Blocker::NotConfigured) => {
            panic!("configured seat must not report not configured")
        }
        Some(Blocker::Unverified) => {
            panic!("suspension must outrank malformed verification")
        }
        None => panic!("fail-closed state must have a blocker"),
    }
}

// REQ-091 state-file contract: write_verified publishes one complete verified.json atomically.
#[test]
#[ignore = "GAUNTLET triage FP: the spec fixes the verified.json fields, not compact vs pretty layout; the field values and no-leftover check are covered by the net and filter suites"]
fn write_verified_publishes_exact_complete_json_without_leftovers() {
    let home = TestHome::new("atomic-verified");
    let value = record("2.2.0");

    write_verified(home.path(), "kimi", &value).expect("write verified record");

    let seat_dir = home.seat_dir("kimi");
    let bytes = fs::read(seat_dir.join("verified.json")).expect("read final verified file");
    assert_eq!(
        String::from_utf8(bytes).expect("verified.json must be UTF-8"),
        r#"{"cli_version":"2.2.0","seat_home":"/isolated/homes/kimi","probed_at":"2026-10-18T12:34:56Z","served_model":"kimi-for-coding"}"#
    );

    let mut names: Vec<String> = fs::read_dir(&seat_dir)
        .expect("list seat directory")
        .map(|entry| {
            entry
                .expect("read directory entry")
                .file_name()
                .into_string()
                .expect("state filename must be UTF-8")
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["verified.json".to_owned()],
        "atomic publication must not leave a sibling temporary file"
    );
}

// REQ-091 path containment: a seat key cannot address anything outside seats/.
#[test]
fn seat_keys_cannot_escape_the_seats_directory() {
    let home = TestHome::new("seat-containment");
    let escape_target = home.path().join("escape");
    let invalid = "../escape";

    assert_invalid_seat(
        write_verified(home.path(), invalid, &record("2.2.0")),
        "write_verified",
    );
    assert_invalid_seat(
        write_suspended(home.path(), invalid, "blocked"),
        "write_suspended",
    );
    assert_invalid_seat(
        availability(home.path(), invalid, true, Some("2.2.0")),
        "availability",
    );
    assert_invalid_seat(
        delete_verified(home.path(), invalid),
        "delete_verified",
    );
    assert_invalid_seat(clear(home.path(), invalid), "clear");

    assert!(
        !escape_target.exists(),
        "invalid seat operations must not create paths outside seats/"
    );
}
