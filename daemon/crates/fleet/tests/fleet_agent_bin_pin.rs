//! The fleet spawns the operator-pinned agent binaries, never a bare PATH lookup.
//!
//! Its own test binary on purpose: it sets process-global environment variables, and as the only
//! test in this process nothing else can observe them (a unit test here would race the fleet
//! crate's other tests).
//!
//! Trial fleet 1 (2026-10-04): the fleet spawned bare "codex", the launchd daemon's PATH found
//! /opt/homebrew/bin/codex 0.133.0 while every consult ran the pinned TRIUMVIRATE_CODEX_BIN
//! 0.154.0, and the old CLI refused the configured model.

use std::path::Path;

/// RED IF a codex or gemini-cli fleet member ignores TRIUMVIRATE_*_BIN, or operator connector
/// args (consult-shaped; a sandbox bypass in them would widen the fleet argv) reach a fleet argv.
#[test]
fn fleet_members_run_the_pinned_binaries_with_the_fleet_argv_only() {
    // SAFETY: the only test in this binary, so no other thread reads the environment.
    unsafe {
        std::env::set_var("TRIUMVIRATE_CODEX_BIN", "/pinned/codex");
        std::env::set_var("TRIUMVIRATE_CODEX_ARGS", "--dangerously-bypass-approvals-and-sandbox");
        std::env::set_var("TRIUMVIRATE_GEMINI_BIN", "/pinned/gemini");
        std::env::set_var("TRIUMVIRATE_GEMINI_BACKEND", "gemini-cli");
    }
    let (bin, argv) = fleet::orchestrator::fleet_agent_command("codex", Path::new("/wt"), "task").expect("codex");
    assert_eq!(bin, "/pinned/codex");
    assert_eq!(argv, fleet::orchestrator::fleet_codex_argv("task"));
    // Not only "matches the helper": a helper that started appending the env args would match too (Grok).
    assert!(!argv.iter().any(|a| a.contains("dangerously")), "{argv:?}");

    let (bin, argv) = fleet::orchestrator::fleet_agent_command("gemini", Path::new("/wt"), "task").expect("gemini");
    assert_eq!(bin, "/pinned/gemini");
    assert_eq!(argv, vec!["-p".to_string(), "task".to_string()]);
}
