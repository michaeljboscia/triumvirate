// GAUNTLET filter suite for T-203, written blind from the spec (no implementation read).
//
// SPEC_AMBIGUITY: behavior for an empty seat name or an empty version string is unspecified and not tested.

use mcp_bridge::seat_gate::{installed_cli_version, register_installed_cli_version};

// A-8: an unregistered seat has no installed version (the gate then fails closed).
#[test]
fn test_should_return_none_for_seat_never_registered() {
    assert_eq!(installed_cli_version("t203-filter-never-registered"), None);
}

// A-8: register then read returns the exact version string.
#[test]
fn test_should_return_registered_version_exactly() {
    register_installed_cli_version("t203-filter-a", "2.2.0");
    assert_eq!(installed_cli_version("t203-filter-a"), Some("2.2.0".to_string()));
}

// seat_gate docs: re-registering after a CLI upgrade replaces the earlier value.
#[test]
fn test_should_replace_version_when_registered_again() {
    register_installed_cli_version("t203-filter-b", "0.25.1");
    register_installed_cli_version("t203-filter-b", "0.25.2");
    assert_eq!(installed_cli_version("t203-filter-b"), Some("0.25.2".to_string()));
}

// Registry is per seat: one seat's version never leaks to another.
#[test]
fn test_should_keep_versions_independent_per_seat() {
    register_installed_cli_version("t203-filter-c1", "1.0.0");
    register_installed_cli_version("t203-filter-c2", "2.0.0");
    assert_eq!(installed_cli_version("t203-filter-c1"), Some("1.0.0".to_string()));
    assert_eq!(installed_cli_version("t203-filter-c2"), Some("2.0.0".to_string()));
    assert_eq!(installed_cli_version("t203-filter-c3"), None);
}

// Version strings are stored verbatim (no trimming, no normalization of prefixes).
#[test]
fn test_should_store_version_text_verbatim() {
    register_installed_cli_version("t203-filter-d", "v2.26.1-rc.1+build5");
    assert_eq!(installed_cli_version("t203-filter-d"), Some("v2.26.1-rc.1+build5".to_string()));
}

// Seat names are case sensitive keys.
#[test]
fn test_should_treat_seat_names_case_sensitively() {
    register_installed_cli_version("t203-filter-e", "1.1.1");
    assert_eq!(installed_cli_version("T203-FILTER-E"), None);
}

// Concurrent registration of distinct seats loses none.
#[test]
fn test_should_keep_all_versions_under_concurrent_registration() {
    let hs: Vec<_> = (0..16)
        .map(|i| {
            std::thread::spawn(move || {
                register_installed_cli_version(&format!("t203-filter-par-{i}"), &format!("9.0.{i}"))
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    for i in 0..16 {
        assert_eq!(installed_cli_version(&format!("t203-filter-par-{i}")), Some(format!("9.0.{i}")));
    }
}
