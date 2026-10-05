//! How the fleet spawns each agent: the operator-pinned binary, and a sandbox the member can
//! actually commit from.
//!
//! Its own test binary on purpose: it sets process-global environment variables, and as the only
//! test in this process nothing else can observe them (a unit test here would race the fleet
//! crate's other tests).
//!
//! Trial fleet 1 (2026-10-04): the fleet spawned bare "codex", the launchd daemon's PATH found
//! /opt/homebrew/bin/codex 0.133.0 while every consult ran the pinned TRIUMVIRATE_CODEX_BIN
//! 0.154.0, and the old CLI refused the configured model. Then: no sandboxed member could commit
//! in a linked worktree, whose git dir lives in the main repo.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .status()
        .expect("git")
        .success();
    assert!(ok, "git {args:?}");
}

/// A main repo plus a linked worktree on a fleet branch, the shape prepare_fleet makes.
fn linked_worktree(root: &Path) -> (PathBuf, PathBuf) {
    let main = root.join("main");
    std::fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q"]);
    git(&main, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let wt = main.join(".triumvirate").join("worktrees").join("m");
    git(&main, &["worktree", "add", "-q", "-b", "fleet/f-1/T-001", wt.to_str().unwrap()]);
    (main.canonicalize().unwrap(), wt)
}

fn value_after<'a>(argv: &'a [String], flag: &str) -> Vec<&'a str> {
    argv.windows(2).filter(|w| w[0] == flag).map(|w| w[1].as_str()).collect()
}

/// RED IF a codex, gemini-cli or grok fleet member ignores its TRIUMVIRATE_*_BIN, operator
/// connector args (consult-shaped; a sandbox bypass in them would widen the fleet argv) reach a
/// codex fleet argv, the codex member is not granted the git dirs a commit needs (or is granted
/// all of .git, or other fleets' refs), or the grok member keeps the consult's read-only sandbox
/// and 12-turn Fast profile. Structural: it checks the argv, it does not run an agent. That a
/// member really commits with these grants was probed live and is proven by a real fleet run.
#[test]
fn fleet_members_get_pinned_binaries_and_commit_grants() {
    let tmp = tempfile::tempdir().unwrap();
    let (main, wt) = linked_worktree(tmp.path());
    // SAFETY: the only test in this binary, so no other thread reads the environment.
    unsafe {
        std::env::set_var("TRIUMVIRATE_CODEX_BIN", "/pinned/codex");
        std::env::set_var("TRIUMVIRATE_CODEX_ARGS", "--dangerously-bypass-approvals-and-sandbox");
        std::env::set_var("TRIUMVIRATE_GEMINI_BIN", "/pinned/gemini");
        std::env::set_var("TRIUMVIRATE_GEMINI_BACKEND", "gemini-cli");
        std::env::set_var("TRIUMVIRATE_GROK_BIN", "/pinned/grok");
        std::env::remove_var("TRIUMVIRATE_GROK_SANDBOX");
        std::env::remove_var("TRIUMVIRATE_GROK_DEPTH");
    }

    let (bin, argv) = fleet::orchestrator::fleet_agent_command("codex", &wt, "task").expect("codex");
    assert_eq!(bin, "/pinned/codex");
    assert_eq!(argv.first().map(String::as_str), Some("exec"));
    assert_eq!(value_after(&argv, "--sandbox"), vec!["workspace-write"]);
    assert_eq!(argv.last().map(String::as_str), Some("task"));
    assert!(!argv.iter().any(|a| a.contains("dangerously")), "{argv:?}");
    let dotgit = main.join(".git");
    let grants: Vec<PathBuf> = value_after(&argv, "--add-dir").into_iter().map(PathBuf::from).collect();
    let want = vec![
        dotgit.join("worktrees").join("m"),
        dotgit.join("objects"),
        dotgit.join("refs").join("heads").join("fleet").join("f-1"),
        dotgit.join("logs").join("refs").join("heads").join("fleet").join("f-1"),
    ];
    assert_eq!(grants, want, "exactly the dirs a commit writes, never .git itself");
    assert!(want.iter().all(|d| d.is_dir()), "a grant must name an existing directory");

    let (bin, argv) = fleet::orchestrator::fleet_agent_command("gemini", &wt, "task").expect("gemini");
    assert_eq!(bin, "/pinned/gemini");
    assert_eq!(argv, vec!["-p".to_string(), "task".to_string()]);

    let (bin, argv) = fleet::orchestrator::fleet_agent_command("grok", &wt, "task").expect("grok");
    assert_eq!(bin, "/pinned/grok");
    assert!(value_after(&argv, "--sandbox").is_empty(), "no sandbox flag, as the agy fleet arm: {argv:?}");
    assert_eq!(value_after(&argv, "--max-turns"), vec!["30"], "Deep, not Fast's 12: {argv:?}");

    // An operator's explicit grok sandbox outranks the fleet's "off".
    unsafe { std::env::set_var("TRIUMVIRATE_GROK_SANDBOX", "strict") };
    let (_, argv) = fleet::orchestrator::fleet_agent_command("grok", &wt, "task").expect("grok");
    assert_eq!(value_after(&argv, "--sandbox"), vec!["strict"], "{argv:?}");

    // Not a worktree at all, or not on a fleet branch: refuse, never launch uncommittable.
    assert!(fleet::orchestrator::fleet_agent_command("codex", tmp.path(), "task").is_err());
    git(&wt, &["checkout", "-q", "-b", "not-a-fleet-branch"]);
    assert!(fleet::orchestrator::fleet_agent_command("codex", &wt, "task").is_err());
}
