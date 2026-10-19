//! Seven Primes T-104 (REQ-030, REQ-031, REQ-033, REQ-035) containment against the real tools: macOS `sandbox-exec`
//! and `git`. FULL_TESTS runs this target from T-104 on, and the `macos-latest` CI job runs it so the seatbelt cases
//! count in the authoritative verdict (they are compiled out on Linux, where seatbelt does not exist).
//!
//! Every test root lives under CARGO_TARGET_TMPDIR, NOT the system temp dir: the profile deliberately allows writes
//! under /private/var/folders and /private/tmp (the CLIs need scratch space), so a repo created there would be
//! writable and a "write is blocked" assertion could never fail.

use std::path::{Path, PathBuf};
use std::process::Command;

use mcp_bridge::containment::{audit_repo, snapshot_repo};
// The seatbelt cases are macOS-only, so the profile renderer is imported only there (Linux clippy: unused import).
#[cfg(target_os = "macos")]
use mcp_bridge::containment::render_seatbelt_profile;

/// A fresh directory under the crate's target tmp dir, unique per test.
fn root(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("seven-primes-containment")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Canonical, because seatbelt matches resolved paths.
    std::fs::canonicalize(&dir).unwrap()
}

fn git(repo: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .expect("git must be on PATH")
        .success();
    assert!(ok, "git {args:?}");
}

/// A repo with one committed file, then a PRE-EXISTING tracked edit and a pre-existing untracked file: the state a
/// reviewed working tree is usually in.
fn repo_with_preexisting_changes(dir: &Path) {
    git(dir, &["init", "-q"]);
    std::fs::write(dir.join("tracked.txt"), "v1\n").unwrap();
    std::fs::write(dir.join("other.txt"), "o1\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "base"]);
    std::fs::write(dir.join("tracked.txt"), "v2 pre-existing edit\n").unwrap();
    std::fs::write(dir.join("untracked-before.txt"), "u\n").unwrap();
}

/// REQ-035 (2): a NEW tracked edit plus a NEW untracked file are both reported, the two pre-existing changes are not,
/// and the audit touches nothing (all four paths keep their bytes).
/// RED IF: the audit misses a new write, blames a pre-existing change, or reverts anything.
#[test]
fn audit_reports_only_new_writes_and_reverts_nothing() {
    let dir = root("audit");
    repo_with_preexisting_changes(&dir);
    let before = snapshot_repo(&dir).unwrap();
    assert_eq!(
        audit_repo(&before, &snapshot_repo(&dir).unwrap()).unwrap(),
        Vec::<String>::new()
    );

    std::fs::write(dir.join("other.txt"), "o2 written by the seat\n").unwrap();
    std::fs::write(dir.join("untracked-new.txt"), "n\n").unwrap();
    let after = snapshot_repo(&dir).unwrap();
    assert_eq!(
        audit_repo(&before, &after).unwrap(),
        ["other.txt", "untracked-new.txt"]
    );

    // Nothing reverted: every path still has the bytes the run left.
    for (f, want) in [
        ("tracked.txt", "v2 pre-existing edit\n"),
        ("untracked-before.txt", "u\n"),
        ("other.txt", "o2 written by the seat\n"),
        ("untracked-new.txt", "n\n"),
    ] {
        assert_eq!(std::fs::read_to_string(dir.join(f)).unwrap(), want, "{f}");
    }

    // Editing a file that already had a pre-existing edit is a change too (same status, new content).
    std::fs::write(dir.join("tracked.txt"), "v3 seat edit on top\n").unwrap();
    assert!(
        audit_repo(&before, &snapshot_repo(&dir).unwrap())
            .unwrap()
            .contains(&"tracked.txt".to_string())
    );
}

/// REQ-035 (1): under the rendered profile a write into the repo FAILS and a write into the seat home SUCCEEDS.
/// RED IF: the profile stops denying repo writes, or stops allowing the seat home (a seat that cannot write its own
/// session files would fail every call).
#[cfg(target_os = "macos")]
#[test]
fn seatbelt_blocks_repo_writes_and_allows_the_seat_home() {
    let base = root("seatbelt");
    let (repo, home, out, tmp) = (
        base.join("repo"),
        base.join("home"),
        base.join("out"),
        base.join("tmp"),
    );
    for d in [&repo, &home, &out, &tmp] {
        std::fs::create_dir_all(d).unwrap();
    }
    let profile = render_seatbelt_profile(&home, &out, &tmp).unwrap();
    let sh = |target: &Path| {
        Command::new("/usr/bin/sandbox-exec")
            .args(["-p", &profile, "/bin/sh", "-c"])
            .arg(format!("echo x > '{}'", target.display()))
            .output()
            .expect("/usr/bin/sandbox-exec must exist on macOS: this test is the seatbelt proof and must not be skipped")
    };
    let blocked = sh(&repo.join("f"));
    assert!(
        !blocked.status.success(),
        "repo write must fail under the profile"
    );
    assert!(
        String::from_utf8_lossy(&blocked.stderr).contains("Operation not permitted"),
        "the failure must be the sandbox's: {}",
        String::from_utf8_lossy(&blocked.stderr)
    );
    assert!(!repo.join("f").exists());

    for allowed in [&home, &out, &tmp] {
        let ok = sh(&allowed.join("f"));
        assert!(
            ok.status.success(),
            "{}: {}",
            allowed.display(),
            String::from_utf8_lossy(&ok.stderr)
        );
        assert_eq!(std::fs::read_to_string(allowed.join("f")).unwrap(), "x\n");
    }
}

/// REQ-035 (3): the seatbelt and the audit together. A write the sandbox blocks never reaches the repo, so the audit
/// of a sandboxed run that TRIED to write is clean, while the same write without the sandbox is caught by the audit.
/// RED IF: either layer stops working on its own.
#[cfg(target_os = "macos")]
#[test]
fn seatbelt_and_audit_agree() {
    let base = root("layers");
    let (repo, home) = (base.join("repo"), base.join("home"));
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    repo_with_preexisting_changes(&repo);
    let profile = render_seatbelt_profile(&home, &home, &home).unwrap();
    let write = format!("echo leak > '{}'", repo.join("leak.txt").display());

    let before = snapshot_repo(&repo).unwrap();
    let _ = Command::new("/usr/bin/sandbox-exec")
        .args(["-p", &profile, "/bin/sh", "-c", &write])
        .status()
        .unwrap();
    assert_eq!(
        audit_repo(&before, &snapshot_repo(&repo).unwrap()).unwrap(),
        Vec::<String>::new()
    );

    assert!(
        Command::new("/bin/sh")
            .args(["-c", &write])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        audit_repo(&before, &snapshot_repo(&repo).unwrap()).unwrap(),
        ["leak.txt"]
    );
}
