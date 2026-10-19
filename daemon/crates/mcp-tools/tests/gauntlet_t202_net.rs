//! GAUNTLET net suite for T-202, written blind from the spec (no implementation read).
//!
//! Scope: mcp_tools::aliases::daemon_target_agents only.
//! Expected set from SPEC REQ-004 and REQ-061: the spawnable CLI peers are the daemon targets. Claude (orchestrator)
//! and deepseek (HTTP, no CLI) are excluded. The three new seats (kimi, mistral, qwen) are session-capable and therefore
//! targets; grok joins gemini and codex.

use mcp_tools::aliases::daemon_target_agents;

fn sorted_targets() -> Vec<&'static str> {
    let mut targets: Vec<&'static str> = daemon_target_agents().to_vec();
    targets.sort_unstable();
    targets
}

// REQ-004, REQ-061: the daemon target set is exactly codex, gemini, grok, kimi, mistral, qwen.
#[test]
fn daemon_targets_are_exactly_the_six_spawnable_peers() {
    assert_eq!(
        sorted_targets(),
        vec!["codex", "gemini", "grok", "kimi", "mistral", "qwen"]
    );
}

// REQ-004: no agent name appears twice in the daemon target list.
#[test]
fn daemon_targets_contain_no_duplicates() {
    let targets = sorted_targets();
    let mut deduped = targets.clone();
    deduped.dedup();
    assert_eq!(targets.len(), 6);
    assert_eq!(deduped.len(), targets.len());
}

// REQ-061: each new seat is session-capable, so kimi, mistral and qwen are daemon targets.
#[test]
fn new_seats_are_daemon_targets() {
    let targets = daemon_target_agents();
    for seat in ["kimi", "mistral", "qwen"] {
        assert!(targets.contains(&seat), "missing daemon target: {seat}");
    }
}

// REQ-004: claude is the orchestrator and deepseek is HTTP with no CLI, so neither is a daemon target.
#[test]
fn claude_and_deepseek_are_not_daemon_targets() {
    let targets = daemon_target_agents();
    assert!(
        !targets.contains(&"claude"),
        "claude must not be a daemon target"
    );
    assert!(
        !targets.contains(&"deepseek"),
        "deepseek must not be a daemon target"
    );
}

// REQ-004: the existing spawnable peers gemini, codex and grok remain daemon targets.
#[test]
fn existing_spawnable_peers_remain_daemon_targets() {
    let targets = daemon_target_agents();
    for seat in ["codex", "gemini", "grok"] {
        assert!(targets.contains(&seat), "missing daemon target: {seat}");
    }
}
