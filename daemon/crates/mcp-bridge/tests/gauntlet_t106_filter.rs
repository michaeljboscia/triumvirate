// GAUNTLET filter suite for T-106, written blind from the spec (no implementation read).
//
// SPEC_AMBIGUITY: delete_verified and clear are not specified for the "file already absent" case (Ok or Err); no test pins it.
// SPEC_AMBIGUITY: the exact rule for which seat keys are rejected as path-escaping is not listed; tests only assert that
//   "../x", "a/b" and "" cannot create anything outside seats/ and are rejected by the write functions.
// SPEC_AMBIGUITY: is_configured's Err conditions and the empty-TRIUMVIRATE_HOME case of triumvirate_home are not specified.
// SPEC_AMBIGUITY: the contents of suspended.json written by write_suspended beyond {"reason","at"} (at = RFC3339) are not specified.
// SPEC_AMBIGUITY: Blocker / SeatAvailability derive list is not given, so tests compare via matches! and field access only.

use mcp_bridge::seat_state::*;
use std::path::Path;

fn rec(version: &str) -> VerifiedRecord {
    VerifiedRecord {
        cli_version: version.to_string(),
        seat_home: "/tmp/gauntlet-seat-home".to_string(),
        probed_at: "2026-10-18T12:00:00Z".to_string(),
        served_model: Some("model-x".to_string()),
    }
}

fn vpath(home: &Path, seat: &str) -> std::path::PathBuf {
    home.join("seats").join(seat).join("verified.json")
}
fn spath(home: &Path, seat: &str) -> std::path::PathBuf {
    home.join("seats").join(seat).join("suspended.json")
}

fn avail(home: &Path, seat: &str, ver: Option<&str>) -> SeatAvailability {
    availability(home, seat, true, ver).expect("availability must be Ok")
}

// ---------- (1) properties and invariants ----------

// REQ-091: available = configured AND verified AND NOT suspended, over the full truth table.
#[test]
fn test_should_available_equal_configured_and_verified_and_not_suspended_for_all_combinations() {
    for c in [false, true] {
        for v in [false, true] {
            for s in [false, true] {
                let sut = SeatAvailability {
                    configured: c,
                    verified: v,
                    suspended: if s { Some("r".to_string()) } else { None },
                };
                assert_eq!(sut.available(), c && v && !s, "c={c} v={v} s={s}");
                assert_eq!(sut.first_blocker().is_none(), sut.available(), "c={c} v={v} s={s}");
            }
        }
    }
}

// REQ-091 precedence: not configured > suspended > unverified, over the full truth table.
#[test]
fn test_should_first_blocker_follow_precedence_for_all_combinations() {
    for c in [false, true] {
        for v in [false, true] {
            for s in [false, true] {
                let sut = SeatAvailability {
                    configured: c,
                    verified: v,
                    suspended: if s { Some("why".to_string()) } else { None },
                };
                let got = sut.first_blocker();
                if !c {
                    assert!(matches!(got, Some(Blocker::NotConfigured)), "c={c} v={v} s={s}");
                } else if s {
                    assert!(matches!(got, Some(Blocker::Suspended(ref r)) if r == "why"), "c={c} v={v} s={s}");
                } else if !v {
                    assert!(matches!(got, Some(Blocker::Unverified)), "c={c} v={v} s={s}");
                } else {
                    assert!(got.is_none(), "c={c} v={v} s={s}");
                }
            }
        }
    }
}

// BACKEND_STRUCTURE s4: read per dispatch, never cached. Alternate state many times; every read reflects the file now.
#[test]
fn test_should_reflect_every_state_change_on_the_next_read_without_caching() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    for i in 0..25 {
        assert!(!avail(home, "kimi", Some("2.2.0")).verified, "iteration {i} before write");
        write_verified(home, "kimi", &rec("2.2.0")).unwrap();
        assert!(avail(home, "kimi", Some("2.2.0")).verified, "iteration {i} after write");
        delete_verified(home, "kimi").unwrap();
        assert!(!avail(home, "kimi", Some("2.2.0")).verified, "iteration {i} after delete");
        write_suspended(home, "kimi", "r").unwrap();
        assert!(avail(home, "kimi", Some("2.2.0")).suspended.is_some(), "iteration {i} suspended");
        clear(home, "kimi").unwrap();
        assert!(avail(home, "kimi", Some("2.2.0")).suspended.is_none(), "iteration {i} cleared");
    }
}

// REQ-091 (atomic writes): a concurrent reader never observes a torn verified.json (torn would fail closed to false).
#[test]
fn test_should_never_expose_half_written_verified_record_to_concurrent_reader() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    let stop = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|s| {
        let reader = s.spawn(|| {
            let mut reads = 0u32;
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                let a = availability(home, "kimi", true, Some("2.2.0")).expect("read must be Ok");
                assert!(a.verified, "torn or missing verified.json observed on read {reads}");
                reads += 1;
            }
            reads
        });
        for i in 0..150 {
            let mut r = rec("2.2.0");
            r.served_model = Some(format!("{}-{i}", "x".repeat(300_000)));
            write_verified(home, "kimi", &r).unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(reader.join().unwrap() > 0);
    });
}

// ---------- (2) happy paths with exact values ----------

// REQ-091: files live at $TRIUMVIRATE_HOME/seats/<seat>/{verified,suspended}.json.
#[test]
fn test_should_write_state_files_at_the_documented_paths() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "mistral", &rec("2.26.1")).unwrap();
    write_suspended(home, "mistral", "banned").unwrap();
    assert!(vpath(home, "mistral").is_file());
    assert!(spath(home, "mistral").is_file());
}

// BACKEND_STRUCTURE s4: verified.json carries cli_version, seat_home, probed_at, served_model.
#[test]
fn test_should_write_verified_json_with_the_four_documented_fields() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vpath(home, "kimi")).unwrap()).unwrap();
    assert_eq!(v["cli_version"], "2.2.0");
    assert_eq!(v["seat_home"], "/tmp/gauntlet-seat-home");
    assert_eq!(v["probed_at"], "2026-10-18T12:00:00Z");
    assert_eq!(v["served_model"], "model-x");
}

// BACKEND_STRUCTURE s4: served_model is "<id or null>".
#[test]
fn test_should_write_null_served_model_when_none() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let mut r = rec("2.2.0");
    r.served_model = None;
    write_verified(home, "qwen", &r).unwrap();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vpath(home, "qwen")).unwrap()).unwrap();
    assert!(v["served_model"].is_null());
    assert_eq!(v["cli_version"], "2.2.0");
}

// BACKEND_STRUCTURE s4: suspended.json is {"reason","at"} with at RFC3339.
#[test]
fn test_should_write_suspended_json_with_reason_and_rfc3339_at() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_suspended(home, "kimi", "account suspended by provider").unwrap();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(spath(home, "kimi")).unwrap()).unwrap();
    assert_eq!(v["reason"], "account suspended by provider");
    let at = v["at"].as_str().expect("at must be a string");
    assert!(chrono::DateTime::parse_from_rfc3339(at).is_ok(), "at not RFC3339: {at}");
}

// Reality test: writing verified.json makes the next availability read true; version change makes it false.
#[test]
fn test_should_become_verified_after_write_and_unverified_after_cli_version_change() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let before = avail(home, "kimi", Some("2.2.0"));
    assert!(before.configured);
    assert!(!before.verified);
    assert!(before.suspended.is_none());
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    let ok = avail(home, "kimi", Some("2.2.0"));
    assert!(ok.verified && ok.available());
    let changed = avail(home, "kimi", Some("2.3.0"));
    assert!(!changed.verified && !changed.available());
    assert!(matches!(changed.first_blocker(), Some(Blocker::Unverified)));
}

// REQ-091: a second write_verified replaces the record (new version is the one that matches).
#[test]
fn test_should_overwrite_previous_verified_record() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("1.0.0")).unwrap();
    write_verified(home, "kimi", &rec("2.0.0")).unwrap();
    assert!(avail(home, "kimi", Some("2.0.0")).verified);
    assert!(!avail(home, "kimi", Some("1.0.0")).verified);
}

// Reality test: suspended.json yields Suspended ahead of Unverified, carrying the written reason.
#[test]
fn test_should_report_suspended_with_reason_ahead_of_unverified() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_suspended(home, "kimi", "billing lock").unwrap();
    let a = avail(home, "kimi", Some("2.2.0"));
    assert!(!a.verified);
    assert_eq!(a.suspended.as_deref(), Some("billing lock"));
    assert!(!a.available());
    assert!(matches!(a.first_blocker(), Some(Blocker::Suspended(ref r)) if r == "billing lock"));
}

// REQ-091: suspended outranks unverified even after the failing probe deleted verified.json.
#[test]
fn test_should_stay_suspended_after_verified_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    write_suspended(home, "kimi", "suspended").unwrap();
    delete_verified(home, "kimi").unwrap();
    let a = avail(home, "kimi", Some("2.2.0"));
    assert!(matches!(a.first_blocker(), Some(Blocker::Suspended(ref r)) if r == "suspended"));
}

// REQ-091 / REQ-054: a verified seat that is suspended is not available even though verified is true.
#[test]
fn test_should_not_be_available_when_verified_but_suspended() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    write_suspended(home, "kimi", "r").unwrap();
    let a = avail(home, "kimi", Some("2.2.0"));
    assert!(a.verified);
    assert!(a.suspended.is_some());
    assert!(!a.available());
}

// REQ-054: a probe that spawned and failed deletes the verified record.
#[test]
fn test_should_delete_verified_record_on_failed_spawned_probe() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    delete_verified(home, "kimi").unwrap();
    assert!(!vpath(home, "kimi").exists());
    assert!(!avail(home, "kimi", Some("2.2.0")).verified);
}

// REQ-054: deleting verified.json does not touch suspended.json.
#[test]
fn test_should_keep_suspended_json_when_verified_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    write_suspended(home, "kimi", "r").unwrap();
    delete_verified(home, "kimi").unwrap();
    assert!(spath(home, "kimi").is_file());
}

// Reality test: only clear() removes suspended.json.
#[test]
fn test_should_remove_suspended_json_only_via_clear() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_suspended(home, "kimi", "r").unwrap();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    delete_verified(home, "kimi").unwrap();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    assert!(avail(home, "kimi", Some("2.2.0")).suspended.is_some(), "writing/deleting verified must not clear suspension");
    clear(home, "kimi").unwrap();
    assert!(!spath(home, "kimi").exists());
    assert!(avail(home, "kimi", Some("2.2.0")).suspended.is_none());
}

// `seat clear` removes the suspension only; verified.json survives.
#[test]
fn test_should_leave_verified_json_in_place_when_clear_runs() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    write_suspended(home, "kimi", "r").unwrap();
    clear(home, "kimi").unwrap();
    let a = avail(home, "kimi", Some("2.2.0"));
    assert!(a.verified && a.suspended.is_none() && a.available());
}

// Seat isolation: state for one seat does not leak to another.
#[test]
fn test_should_keep_seat_state_isolated_between_seats() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    write_suspended(home, "qwen", "r").unwrap();
    assert!(avail(home, "kimi", Some("2.2.0")).verified);
    assert!(avail(home, "kimi", Some("2.2.0")).suspended.is_none());
    assert!(!avail(home, "qwen", Some("2.2.0")).verified);
    assert!(avail(home, "qwen", Some("2.2.0")).suspended.is_some());
    assert!(!avail(home, "mistral", Some("2.2.0")).verified);
    clear(home, "qwen").unwrap();
    assert!(avail(home, "kimi", Some("2.2.0")).verified);
}

// availability passes `configured` through unchanged.
#[test]
fn test_should_pass_configured_flag_through_to_result() {
    let dir = tempfile::tempdir().unwrap();
    write_verified(dir.path(), "kimi", &rec("2.2.0")).unwrap();
    let off = availability(dir.path(), "kimi", false, Some("2.2.0")).unwrap();
    assert!(!off.configured && off.verified && !off.available());
    assert!(matches!(off.first_blocker(), Some(Blocker::NotConfigured)));
    let on = availability(dir.path(), "kimi", true, Some("2.2.0")).unwrap();
    assert!(on.configured && on.available());
}

// A home with no seats directory at all reads as configured-but-nothing-else, not an error.
#[test]
fn test_should_read_missing_state_as_unverified_and_unsuspended() {
    let dir = tempfile::tempdir().unwrap();
    let a = availability(&dir.path().join("does-not-exist"), "kimi", true, Some("2.2.0")).unwrap();
    assert!(a.configured);
    assert!(!a.verified);
    assert!(a.suspended.is_none());
    assert!(matches!(a.first_blocker(), Some(Blocker::Unverified)));
}

// write functions create the seats/<seat>/ directory themselves in a fresh home.
#[test]
fn test_should_create_seat_directory_on_first_write_in_fresh_home() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("fresh-home");
    write_suspended(&home, "mistral", "r").unwrap();
    write_verified(&home, "qwen", &rec("0.25.0")).unwrap();
    assert!(spath(&home, "mistral").is_file());
    assert!(vpath(&home, "qwen").is_file());
}

// Atomic write: no sibling temp file is left behind after a write.
#[test]
fn test_should_leave_only_the_final_file_after_atomic_writes() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    for i in 0..5 {
        write_verified(home, "kimi", &rec(&format!("2.2.{i}"))).unwrap();
    }
    write_suspended(home, "kimi", "r").unwrap();
    let mut names: Vec<String> = std::fs::read_dir(home.join("seats").join("kimi"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["suspended.json".to_string(), "verified.json".to_string()]);
}

// Reasons with quotes, newlines and unicode survive the JSON round trip exactly.
#[test]
fn test_should_round_trip_awkward_suspension_reason_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let reason = "quota \"hard\" stop\nline two \\ backslash \u{1F512} \u{00E9}";
    write_suspended(home, "kimi", reason).unwrap();
    assert_eq!(avail(home, "kimi", Some("2.2.0")).suspended.as_deref(), Some(reason));
}

// ---------- (3) boundaries and edges ----------

// "an unknown installed CLI version cannot match a verified record".
#[test]
fn test_should_treat_verified_as_absent_when_installed_version_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    let a = avail(home, "kimi", None);
    assert!(!a.verified);
    assert!(matches!(a.first_blocker(), Some(Blocker::Unverified)));
}

// An unknown version cannot match even a record whose cli_version is the empty string.
#[test]
fn test_should_not_match_empty_cli_version_record_against_unknown_version() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("")).unwrap();
    assert!(!avail(home, "kimi", None).verified);
}

// Version comparison is exact string inequality: any difference invalidates.
#[test]
fn test_should_invalidate_verified_on_any_version_string_difference() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    for other in ["2.2.1", "2.2", "2.2.0 ", " 2.2.0", "2.2.00", "2.2.0-beta", "v2.2.0", "3.2.0", "2.2.0\n", ""] {
        assert!(!avail(home, "kimi", Some(other)).verified, "version {other:?} must not match 2.2.0");
    }
    assert!(avail(home, "kimi", Some("2.2.0")).verified);
}

// Downgrade also invalidates (differs, not "older than").
#[test]
fn test_should_invalidate_verified_on_version_downgrade() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    assert!(!avail(home, "kimi", Some("2.1.9")).verified);
}

// Re-verifying under the new version restores availability.
#[test]
fn test_should_become_verified_again_after_reprobe_at_new_version() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    assert!(!avail(home, "kimi", Some("2.3.0")).verified);
    write_verified(home, "kimi", &rec("2.3.0")).unwrap();
    assert!(avail(home, "kimi", Some("2.3.0")).verified);
}

// Fail closed: unparseable verified.json is "not verified".
#[test]
fn test_should_fail_closed_on_corrupt_verified_json() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir_all(home.join("seats").join("kimi")).unwrap();
    for body in ["", "not json", "{", "[]", "null", "{\"cli_version\":2}", "{\"seat_home\":\"/x\"}"] {
        std::fs::write(vpath(home, "kimi"), body).unwrap();
        let a = avail(home, "kimi", Some("2.2.0"));
        assert!(!a.verified, "body {body:?} must not verify");
        assert!(!a.available());
    }
}

// Fail closed: a verified.json that cannot be read (a directory in its place) is "not verified".
#[test]
fn test_should_fail_closed_on_unreadable_verified_json() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir_all(vpath(home, "kimi")).unwrap();
    let a = avail(home, "kimi", Some("2.2.0"));
    assert!(!a.verified);
}

// Fail closed: unparseable suspended.json is still "suspended".
#[test]
fn test_should_fail_closed_on_corrupt_suspended_json() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    for body in ["", "garbage", "{", "[]", "{\"at\":\"x\"}"] {
        std::fs::write(spath(home, "kimi"), body).unwrap();
        let a = avail(home, "kimi", Some("2.2.0"));
        assert!(a.suspended.is_some(), "body {body:?} must still count as suspended");
        assert!(!a.available());
        assert!(matches!(a.first_blocker(), Some(Blocker::Suspended(_))));
    }
}

// Fail closed: a suspended.json that cannot be read (a directory in its place) is still "suspended".
#[test]
fn test_should_fail_closed_on_unreadable_suspended_json() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_verified(home, "kimi", &rec("2.2.0")).unwrap();
    std::fs::create_dir_all(spath(home, "kimi")).unwrap();
    let a = avail(home, "kimi", Some("2.2.0"));
    assert!(a.suspended.is_some());
    assert!(!a.available());
}

// A seat key becomes a directory name and must not address anything outside seats/.
#[test]
fn test_should_reject_path_escaping_seat_keys_on_write() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    for bad in ["../evil", "a/b", "..", "", "/abs"] {
        assert!(write_verified(&home, bad, &rec("1")).is_err(), "write_verified accepted {bad:?}");
        assert!(write_suspended(&home, bad, "r").is_err(), "write_suspended accepted {bad:?}");
    }
    assert!(!dir.path().join("evil").exists());
    assert!(!home.join("evil").exists());
    assert!(!home.join("verified.json").exists());
    assert!(!home.join("suspended.json").exists());
    assert!(!dir.path().join("verified.json").exists());
    assert!(!dir.path().join("suspended.json").exists());
}

// A path-escaping seat key can never be read as an available seat, even if a file sits at the escaped location.
#[test]
fn test_should_not_read_state_through_path_escaping_seat_key() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join("seats")).unwrap();
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        outside.join("verified.json"),
        r#"{"cli_version":"2.2.0","seat_home":"/x","probed_at":"2026-10-18T12:00:00Z","served_model":null}"#,
    )
    .unwrap();
    // seats/../../outside from home/seats resolves to dir/outside.
    if let Ok(a) = availability(&home, "../../outside", true, Some("2.2.0")) {
        assert!(!a.available(), "escaped seat key must never be available");
    }
}

// Very long reason is stored and returned intact.
#[test]
fn test_should_round_trip_large_suspension_reason() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let reason = "r".repeat(200_000);
    write_suspended(home, "kimi", &reason).unwrap();
    assert_eq!(avail(home, "kimi", Some("2.2.0")).suspended.as_deref(), Some(reason.as_str()));
}

// An empty reason still counts as suspended (existence of the file, not its text).
#[test]
fn test_should_treat_empty_reason_as_suspended() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_suspended(home, "kimi", "").unwrap();
    let a = avail(home, "kimi", Some("2.2.0"));
    assert_eq!(a.suspended.as_deref(), Some(""));
    assert!(!a.available());
}

// ---------- configured: TRIUMVIRATE_ENABLE_<SEAT>=1 ----------

// REQ-091: default off; only "1" enables, so a typo cannot turn a seat on.
#[test]
fn test_should_enable_seat_only_when_env_is_exactly_one() {
    let var = "TRIUMVIRATE_ENABLE_GAUNTLETT106CFG";
    // SAFETY: unique variable name touched by this one test only.
    unsafe { std::env::remove_var(var) };
    assert_eq!(is_configured("gauntlett106cfg"), Ok(false), "unset must be off");
    for off in ["0", "true", "yes", "on", "", " 1", "1 ", "11", "TRUE", "enabled", "-1"] {
        unsafe { std::env::set_var(var, off) };
        assert_eq!(is_configured("gauntlett106cfg"), Ok(false), "value {off:?} must not enable");
    }
    unsafe { std::env::set_var(var, "1") };
    assert_eq!(is_configured("gauntlett106cfg"), Ok(true));
    unsafe { std::env::remove_var(var) };
    assert_eq!(is_configured("gauntlett106cfg"), Ok(false));
}

// REQ-091: the env var is per seat (TRIUMVIRATE_ENABLE_<SEAT>), so one seat's flag does not enable another.
#[test]
fn test_should_scope_enable_flag_to_its_own_seat() {
    let on = "TRIUMVIRATE_ENABLE_GAUNTLETT106A";
    // SAFETY: unique variable names touched by this one test only.
    unsafe { std::env::remove_var("TRIUMVIRATE_ENABLE_GAUNTLETT106B") };
    unsafe { std::env::set_var(on, "1") };
    assert_eq!(is_configured("gauntlett106a"), Ok(true));
    assert_eq!(is_configured("gauntlett106b"), Ok(false));
    unsafe { std::env::remove_var(on) };
}

// ---------- triumvirate_home ----------

// triumvirate_home: $TRIUMVIRATE_HOME when set, else ~/.triumvirate.
#[test]
fn test_should_resolve_triumvirate_home_from_env_or_default_dot_triumvirate() {
    let got = triumvirate_home().expect("home must resolve");
    match std::env::var("TRIUMVIRATE_HOME") {
        Ok(v) if !v.is_empty() => assert_eq!(got, std::path::PathBuf::from(v)),
        _ => assert!(got.ends_with(".triumvirate"), "default must end with .triumvirate, got {got:?}"),
    }
}
