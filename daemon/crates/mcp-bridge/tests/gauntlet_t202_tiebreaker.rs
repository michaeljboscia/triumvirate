// GAUNTLET tiebreaker suite for T-202, written blind from the spec (no implementation read).
// SPEC_AMBIGUITY: The listed API does not expose provider, billing, prompt usage, attempt policy, live progress, parser modes, sight repair, write containment, usage persistence, or model telemetry consumers.
// SPEC_AMBIGUITY: Testing stall_window environment overrides requires process-global environment mutation, but the contract provides no injectable environment API for isolated integration tests.

use std::time::Duration;

use mcp_bridge::stall::{
    STALLED_PREFIX, default_stall_window_secs, stall_window, stall_window_env_var, stalled_message,
};

// REQ-051: Existing seats retain the measured T-001 ask-path stall windows.
#[test]
fn existing_seat_stall_defaults_are_preserved() {
    assert_eq!(default_stall_window_secs("codex"), Some(240));
    assert_eq!(default_stall_window_secs("gemini"), Some(300));
    assert_eq!(default_stall_window_secs("deepseek"), Some(600));
    assert_eq!(default_stall_window_secs("grok"), Some(780));
    assert_eq!(default_stall_window_secs("claude"), None);
}

// REQ-051: Every new seat starts with grok's 780 second stall window.
#[test]
fn new_seat_stall_defaults_are_780_seconds() {
    for seat in ["kimi", "mistral", "qwen"] {
        assert_eq!(default_stall_window_secs(seat), Some(780), "seat {seat}");
    }
}

// REQ-051: Gemini product aliases normalize before stall policy lookup.
#[test]
fn gemini_aliases_share_the_gemini_stall_default() {
    assert_eq!(default_stall_window_secs("agy"), Some(300));
    assert_eq!(default_stall_window_secs("antigravity"), Some(300));
}

// REQ-051: A name without a SeatProfile row has no stall window.
#[test]
fn unknown_seat_has_no_stall_default() {
    assert_eq!(default_stall_window_secs("not-a-seat"), None);
}

// REQ-051: Environment override keys use the canonical documented spelling.
#[test]
fn stall_override_environment_variable_names_are_exact() {
    assert_eq!(stall_window_env_var("grok"), "TRIUMVIRATE_STALL_SECS_GROK");
    assert_eq!(stall_window_env_var("kimi"), "TRIUMVIRATE_STALL_SECS_KIMI");
    assert_eq!(
        stall_window_env_var("mistral"),
        "TRIUMVIRATE_STALL_SECS_MISTRAL"
    );
    assert_eq!(stall_window_env_var("qwen"), "TRIUMVIRATE_STALL_SECS_QWEN");
}

// REQ-051: With no applicable override, the effective new-seat window is the row value.
#[test]
fn effective_new_seat_windows_match_their_defaults() {
    for seat in ["kimi", "mistral", "qwen"] {
        assert_eq!(
            stall_window(seat),
            Some(Duration::from_secs(780)),
            "seat {seat}"
        );
    }
}

// REQ-051: Disabled and unknown seats are never stall-killed.
#[test]
fn disabled_and_unknown_seats_have_no_effective_window() {
    assert_eq!(stall_window("claude"), None);
    assert_eq!(stall_window("not-a-seat"), None);
}

// REQ-051: Stalled errors retain their stable exact prefix and duration text.
#[test]
fn stalled_error_text_is_exact() {
    assert_eq!(STALLED_PREFIX, "stalled after");
    assert_eq!(stalled_message(780), "stalled after 780s without progress");
}
