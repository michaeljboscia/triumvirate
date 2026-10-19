//! GAUNTLET net suite for T-104, written blind from the spec (no implementation read).
//!
//! SPEC_AMBIGUITY: `write_seat_files` and `child_env` take a `&SeatProfile`, but the listed public API gives neither
//!   SeatProfile's constructor nor its fields, so no test can build that input. The REQ-030, REQ-031 and REQ-033 tests
//!   that run through those two functions (HOME and vendor home var relocation, daemon-written files, env scrub before
//!   env_add) are NOT written. Only the SEAT_CREDENTIAL_SCRUB list is tested directly.
//! SPEC_AMBIGUITY: `audit_repo` returns `Result<Vec<String>, String>`. The spec says a non-empty result "fails the call as
//!   read_only_violation: <paths>" but does not say whether audit_repo returns Err("read_only_violation: ...") or
//!   Ok(paths) for the caller to map. This suite asserts Ok(paths) for a violation.
//! SPEC_AMBIGUITY: the order of the paths returned by `audit_repo` is not specified. Tests compare sorted lists.
//! SPEC_AMBIGUITY: `RepoSnapshot.entries` key format (repo-relative path string) is inferred from "Path -> content hash".
//! SPEC_AMBIGUITY: no error text is specified for `seat_home`, `check_credentials_file`, `scan_stray_keys` or
//!   `render_seatbelt_profile`. Error cases assert `is_err()` only.
//! SPEC_AMBIGUITY: `scan_stray_keys` does not enumerate which names count as metered-API keys. This suite asserts only
//!   that OPENAI_API_KEY (named in the REQ-033 scrub list) is reported and that PATH is not.

use std::fs;
use std::path::Path;
use std::process::Command;

use mcp_bridge::containment::{
    audit_repo, check_credentials_file, render_seatbelt_profile, scan_stray_keys, seat_home, snapshot_repo,
    SEAT_CREDENTIAL_SCRUB,
};

/// Run git with a fixed identity so commits work on any machine.
fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-C"])
        .arg(cwd)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t104")
        .env("GIT_AUTHOR_EMAIL", "t104@example.invalid")
        .env("GIT_COMMITTER_NAME", "t104")
        .env("GIT_COMMITTER_EMAIL", "t104@example.invalid")
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed");
}

/// A repo with one commit, a pre-existing tracked edit and a pre-existing untracked file, made before any snapshot.
fn dirty_repo(dir: &Path) {
    git(dir, &["init", "-q"]);
    fs::write(dir.join("tracked.txt"), "base\n").unwrap();
    fs::write(dir.join("other.txt"), "other base\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "base"]);
    fs::write(dir.join("tracked.txt"), "pre-existing tracked edit\n").unwrap();
    fs::write(dir.join("pre_untracked.txt"), "pre-existing untracked\n").unwrap();
}

// REQ-033: the scrub list names exactly these inherited credential and endpoint variables and prefixes.
#[test]
fn credential_scrub_list_is_exact() {
    assert_eq!(
        SEAT_CREDENTIAL_SCRUB.to_vec(),
        vec![
            "MISTRAL_API_KEY",
            "OPENAI_API_KEY",
            "OPENAI_BASE_URL",
            "DASHSCOPE_*",
            "BAILIAN_*",
            "QWENCLOUD_*",
            "KIMI_*",
            "MOONSHOT_*",
        ]
    );
}

// REQ-030 / BACKEND section 5: a valid seat key resolves to $TRIUMVIRATE_HOME/homes/<seat>.
#[test]
fn seat_home_is_triumvirate_home_homes_seat() {
    let root = tempfile::tempdir().unwrap();
    let got = seat_home(root.path(), "kimi").expect("valid seat key");
    assert_eq!(got, root.path().join("homes").join("kimi"));
}

// Seat key is a path component: keys that could address anything outside homes/ are refused.
#[test]
fn seat_home_refuses_keys_that_escape_homes() {
    let root = tempfile::tempdir().unwrap();
    for bad in ["..", "../escape", "kimi/../../escape", "/abs/path", "a/b"] {
        assert!(
            seat_home(root.path(), bad).is_err(),
            "seat key {bad:?} must be refused"
        );
    }
}

// REQ-035 / BACKEND section 5: the profile denies all file writes first, then allows only the listed subpaths.
#[test]
fn seatbelt_profile_denies_then_allows_listed_paths() {
    let root = tempfile::tempdir().unwrap();
    let seat = root.path().join("homes").join("kimi");
    let out = root.path().join("out");
    let tmp = root.path().join("tmp");
    for d in [&seat, &out, &tmp] {
        fs::create_dir_all(d).unwrap();
    }
    let p = render_seatbelt_profile(&seat, &out, &tmp).expect("render");

    for line in ["(version 1)", "(allow default)", "(deny file-write*)"] {
        assert!(p.contains(line), "profile missing {line}");
    }
    let deny = p.find("(deny file-write*)").unwrap();
    let seat_allow_text = format!(
        "(allow file-write* (subpath \"{}\"))",
        seat.canonicalize().unwrap().display()
    );
    let seat_allow = p.find(&seat_allow_text).expect("seat home allow line present, canonicalized");
    assert!(deny < seat_allow, "deny must come before the seat home allow");
    assert!(p.contains(&format!(
        "(allow file-write* (subpath \"{}\"))",
        out.canonicalize().unwrap().display()
    )));
    assert!(p.contains("(allow file-write* (subpath \"/private/var/folders\"))"));
    assert!(p.contains("(allow file-write* (subpath \"/private/tmp\"))"));
    for dev in ["/dev/null", "/dev/stdout", "/dev/stderr", "/dev/dtracehelper", "/dev/tty"] {
        assert!(p.contains(dev), "profile must allow {dev}");
    }
}

// REQ-035 acceptance (2): on a dirty repo, one new tracked mutation and one new untracked file fail the audit,
// and all content is left untouched.
#[test]
fn audit_reports_new_tracked_and_untracked_mutations_and_reverts_nothing() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path();
    dirty_repo(repo);
    let before = snapshot_repo(repo).expect("snapshot before");
    assert!(before.entries.contains_key("tracked.txt"));
    assert!(before.entries.contains_key("pre_untracked.txt"));

    fs::write(repo.join("other.txt"), "seat wrote this\n").unwrap();
    fs::write(repo.join("new_untracked.txt"), "seat wrote this too\n").unwrap();
    let after = snapshot_repo(repo).expect("snapshot after");

    let mut changed = audit_repo(&before, &after).expect("audit returns the changed paths");
    changed.sort();
    assert_eq!(changed, vec!["new_untracked.txt".to_string(), "other.txt".to_string()]);

    assert_eq!(fs::read_to_string(repo.join("tracked.txt")).unwrap(), "pre-existing tracked edit\n");
    assert_eq!(fs::read_to_string(repo.join("pre_untracked.txt")).unwrap(), "pre-existing untracked\n");
    assert_eq!(fs::read_to_string(repo.join("other.txt")).unwrap(), "seat wrote this\n");
    assert_eq!(fs::read_to_string(repo.join("new_untracked.txt")).unwrap(), "seat wrote this too\n");
}

// REQ-035 acceptance (2): a pre-existing edit that is changed again counts as CHANGED.
#[test]
fn audit_reports_further_change_to_a_pre_existing_edit() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path();
    dirty_repo(repo);
    let before = snapshot_repo(repo).expect("snapshot before");
    fs::write(repo.join("tracked.txt"), "edited again by the run\n").unwrap();
    let after = snapshot_repo(repo).expect("snapshot after");
    assert_eq!(audit_repo(&before, &after).unwrap(), vec!["tracked.txt".to_string()]);
}

// REQ-035 (audit spec): a path in before that is gone from after changed too (the run restored a pre-existing edit).
#[test]
fn audit_reports_a_pre_existing_edit_the_run_restored() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path();
    dirty_repo(repo);
    let before = snapshot_repo(repo).expect("snapshot before");
    git(repo, &["checkout", "--", "tracked.txt"]);
    let after = snapshot_repo(repo).expect("snapshot after");
    assert!(!after.entries.contains_key("tracked.txt"));
    assert_eq!(audit_repo(&before, &after).unwrap(), vec!["tracked.txt".to_string()]);
}

// REQ-035 (audit spec): no change between snapshots means an empty result.
#[test]
fn audit_is_empty_when_nothing_changed() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path();
    dirty_repo(repo);
    let before = snapshot_repo(repo).expect("snapshot before");
    let after = snapshot_repo(repo).expect("snapshot after");
    assert_eq!(audit_repo(&before, &after).unwrap(), Vec::<String>::new());
}

// REQ-035 (snapshot spec): a clean repo has no changed, staged or untracked paths, so the snapshot is empty.
#[test]
fn snapshot_of_clean_repo_is_empty() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path();
    git(repo, &["init", "-q"]);
    fs::write(repo.join("only.txt"), "clean\n").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "base"]);
    assert!(snapshot_repo(repo).expect("snapshot").entries.is_empty());
}

// REQ-035 (snapshot spec): a staged-only new file is part of the snapshot and is reported by the audit.
#[test]
fn staged_new_file_is_reported() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path();
    dirty_repo(repo);
    let before = snapshot_repo(repo).expect("snapshot before");
    fs::write(repo.join("staged_new.txt"), "staged\n").unwrap();
    git(repo, &["add", "staged_new.txt"]);
    let after = snapshot_repo(repo).expect("snapshot after");
    assert!(after.entries.contains_key("staged_new.txt"));
    assert_eq!(audit_repo(&before, &after).unwrap(), vec!["staged_new.txt".to_string()]);
}

// REQ-035 acceptance (3): writes outside the repo (seat home, output dir, temp) do not trip the audit.
#[test]
fn writes_outside_the_repo_do_not_trip_the_audit() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    let seat = root.path().join("homes").join("kimi");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&seat).unwrap();
    dirty_repo(&repo);
    let before = snapshot_repo(&repo).expect("snapshot before");
    fs::write(seat.join("session.json"), "{}").unwrap();
    let after = snapshot_repo(&repo).expect("snapshot after");
    assert_eq!(audit_repo(&before, &after).unwrap(), Vec::<String>::new());
}

// REQ-033: credentials.env must be exactly mode 0600.
#[test]
fn credentials_file_passes_only_at_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let p = root.path().join("credentials.env");
    fs::write(&p, "KEY=placeholder\n").unwrap();

    fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(check_credentials_file(&p).is_ok(), "0600 must pass");

    fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(check_credentials_file(&p).is_err(), "0644 must fail");

    fs::set_permissions(&p, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(check_credentials_file(&p).is_err(), "0400 must fail: the spec says exactly 0600");
}

// REQ-033: a missing credentials.env fails the check.
#[test]
fn missing_credentials_file_fails() {
    let root = tempfile::tempdir().unwrap();
    assert!(check_credentials_file(&root.path().join("credentials.env")).is_err());
}

// REQ-033: a metered OpenAI key in the env is reported by name; PATH is not reported.
#[test]
fn stray_key_scan_reports_the_metered_key_name_not_its_value() {
    let fake_value = ["sk", "t104", "fakevalue"].join("-");
    let env = vec![
        ("PATH".to_string(), "/usr/bin:/bin".to_string()),
        ("OPENAI_API_KEY".to_string(), fake_value.clone()),
    ];
    let found = scan_stray_keys(&env).expect("scan returns Ok");
    assert!(found.contains(&"OPENAI_API_KEY".to_string()));
    assert!(!found.contains(&"PATH".to_string()));
    assert!(!found.iter().any(|name| name.contains(&fake_value)));
}

// REQ-033: an env with no metered key reports nothing.
#[test]
fn stray_key_scan_is_empty_for_a_clean_env() {
    let env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string())];
    assert!(scan_stray_keys(&env).expect("scan returns Ok").is_empty());
}

#[cfg(target_os = "macos")]
fn run_under_profile(profile_text: &str, script: &str) -> std::process::Output {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("profile.sb");
    fs::write(&profile, profile_text).unwrap();
    Command::new("/usr/bin/sandbox-exec")
        .arg("-f")
        .arg(&profile)
        .arg("/bin/sh")
        .arg("-c")
        .arg(script)
        .output()
        .expect("sandbox-exec runs")
}

// REQ-035 acceptance (1): under the rendered profile a write into the repo fails and a write into the seat home succeeds.
#[cfg(target_os = "macos")]
#[test]
fn seatbelt_blocks_repo_write_and_allows_seat_home_write() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    let seat = root.path().join("homes").join("kimi");
    let out = root.path().join("out");
    let tmp = root.path().join("tmp");
    for d in [&repo, &seat, &out, &tmp] {
        fs::create_dir_all(d).unwrap();
    }
    let profile = render_seatbelt_profile(&seat, &out, &tmp).expect("render");

    let repo_write = run_under_profile(&profile, &format!("echo x > '{}/f'", repo.display()));
    assert!(!repo_write.status.success(), "repo write must fail under the seatbelt");
    assert!(!repo.join("f").exists());

    let seat_write = run_under_profile(&profile, &format!("echo x > '{}/f'", seat.display()));
    assert!(
        seat_write.status.success(),
        "seat home write must succeed: {}",
        String::from_utf8_lossy(&seat_write.stderr)
    );
    assert_eq!(fs::read_to_string(seat.join("f")).unwrap(), "x\n");
}

// REQ-035 acceptance (3): writes to the per-call output dir and to temp are allowed under the profile.
#[cfg(target_os = "macos")]
#[test]
fn seatbelt_allows_output_dir_and_temp_writes() {
    let root = tempfile::tempdir().unwrap();
    let seat = root.path().join("homes").join("kimi");
    let out = root.path().join("out");
    let tmp = root.path().join("tmp");
    for d in [&seat, &out, &tmp] {
        fs::create_dir_all(d).unwrap();
    }
    let profile = render_seatbelt_profile(&seat, &out, &tmp).expect("render");

    let out_write = run_under_profile(&profile, &format!("echo x > '{}/export.json'", out.display()));
    assert!(out_write.status.success(), "output dir write must succeed");
    let tmp_write = run_under_profile(&profile, &format!("echo x > '{}/scratch'", tmp.display()));
    assert!(tmp_write.status.success(), "temp write must succeed");
}
