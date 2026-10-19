// GAUNTLET tiebreaker suite for T-202, written blind from the spec (no implementation read).

use mcp_tools::aliases::daemon_target_agents;

// REQ-004 and REQ-061: Daemon targets are exactly the spawnable CLI seats in profile row order.
#[test]
fn daemon_targets_are_the_six_spawnable_cli_seats() {
    assert_eq!(
        daemon_target_agents(),
        &["codex", "gemini", "grok", "kimi", "mistral", "qwen"]
    );
}

// REQ-004 and REQ-061: The orchestrator and HTTP-only peer are excluded from daemon targets.
#[test]
fn daemon_targets_exclude_claude_and_deepseek() {
    assert!(!daemon_target_agents().contains(&"claude"));
    assert!(!daemon_target_agents().contains(&"deepseek"));
}
