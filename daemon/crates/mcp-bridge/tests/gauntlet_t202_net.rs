//! GAUNTLET net suite for T-202, written blind from the spec (no implementation read).
//!
//! Scope: mcp_bridge::stall::{default_stall_window_secs, stall_window, stall_window_env_var, stalled_message, STALLED_PREFIX}.
//! Expected values come from SPEC REQ-051 (new seats start at 780 s, the grok value; claude is Disabled;
//! env override TRIUMVIRATE_STALL_SECS_<SEAT>; override 0 disables; unparseable override is ignored).
//! The existing codex, gemini and deepseek window values are not stated in any readable spec document,
//! so they are not asserted exactly here (see SPEC_AMBIGUITY below).
//!
//! SPEC_AMBIGUITY: the spec does not state the exact pinned stall windows for codex, gemini or deepseek
//! (only grok = 780 and claude = Disabled are given in IMPLEMENTATION_PLAN T-002 and BACKEND_STRUCTURE section 2).
//! Their exact values are therefore untested by this suite.
//! SPEC_AMBIGUITY: the spec does not state the exact text of the stalled_message body beyond the stable
//! prefix "stalled after". This suite asserts only the prefix and that the seconds value appears in the text.

use std::sync::Mutex;
use std::time::Duration;

use mcp_bridge::stall::{
    STALLED_PREFIX, default_stall_window_secs, stall_window, stall_window_env_var, stalled_message,
};

/// Serializes every test that touches the process environment, since set_var and remove_var are
/// process-global and the test harness runs tests on parallel threads.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn clear_override(seat: &str) {
    let var = stall_window_env_var(seat);
    // SAFETY: every env-touching test holds ENV_LOCK, so no other test reads or writes the variable concurrently.
    unsafe { std::env::remove_var(var) };
}

fn set_override(seat: &str, value: &str) {
    let var = stall_window_env_var(seat);
    // SAFETY: every env-touching test holds ENV_LOCK, so no other test reads or writes the variable concurrently.
    unsafe { std::env::set_var(var, value) };
}

// REQ-051: each new seat (kimi, mistral, qwen) starts at the grok value of 780 s.
#[test]
fn new_seats_default_window_is_780_secs() {
    let _guard = lock_env();
    for seat in ["kimi", "mistral", "qwen"] {
        clear_override(seat);
        assert_eq!(default_stall_window_secs(seat), Some(780), "seat {seat}");
    }
}

// REQ-051: grok's measured window is 780 s.
#[test]
fn grok_default_window_is_780_secs() {
    let _guard = lock_env();
    clear_override("grok");
    assert_eq!(default_stall_window_secs("grok"), Some(780));
}

// REQ-051: claude's stall window is Disabled, so it is never stall-killed and has no window.
#[test]
fn claude_has_no_stall_window() {
    let _guard = lock_env();
    clear_override("claude");
    assert_eq!(default_stall_window_secs("claude"), None);
    assert_eq!(stall_window("claude"), None);
}

// Doc contract: "A name with no row has none."
#[test]
fn unknown_agent_has_no_stall_window() {
    let _guard = lock_env();
    assert_eq!(default_stall_window_secs("not-a-seat"), None);
}

// Doc contract: aliases normalize first, so agy and antigravity get gemini's window.
#[test]
fn agy_and_antigravity_share_gemini_window() {
    let _guard = lock_env();
    clear_override("gemini");
    let gemini = default_stall_window_secs("gemini");
    assert!(gemini.is_some(), "gemini must have a measured window");
    assert_eq!(default_stall_window_secs("agy"), gemini);
    assert_eq!(default_stall_window_secs("antigravity"), gemini);
}

// Doc contract: stall_window with no env override returns the default as a Duration.
#[test]
fn stall_window_without_override_is_the_default_duration() {
    let _guard = lock_env();
    clear_override("kimi");
    assert_eq!(stall_window("kimi"), Some(Duration::from_secs(780)));
}

// REQ-051: the override variable is TRIUMVIRATE_STALL_SECS_<SEAT> in uppercase.
#[test]
fn env_var_name_follows_seat_pattern() {
    let _guard = lock_env();
    assert_eq!(stall_window_env_var("grok"), "TRIUMVIRATE_STALL_SECS_GROK");
    assert_eq!(stall_window_env_var("kimi"), "TRIUMVIRATE_STALL_SECS_KIMI");
}

// REQ-051: a parseable TRIUMVIRATE_STALL_SECS_<SEAT> override replaces the default window.
#[test]
fn env_override_replaces_default_window() {
    let _guard = lock_env();
    set_override("mistral", "300");
    let got = stall_window("mistral");
    clear_override("mistral");
    assert_eq!(got, Some(Duration::from_secs(300)));
}

// REQ-051 / doc contract: an override of 0 turns stall detection off for that agent.
#[test]
fn env_override_of_zero_disables_stall_detection() {
    let _guard = lock_env();
    set_override("qwen", "0");
    let got = stall_window("qwen");
    clear_override("qwen");
    assert_eq!(got, None);
}

// Doc contract: an unparseable override is ignored (the default stays in force), never silently disabling the kill.
#[test]
fn unparseable_env_override_falls_back_to_default() {
    let _guard = lock_env();
    set_override("kimi", "abc");
    let got = stall_window("kimi");
    clear_override("kimi");
    assert_eq!(got, Some(Duration::from_secs(780)));
}

// Doc contract: callers match the terminal stalled error on STALLED_PREFIX, so the prefix is fixed.
#[test]
fn stalled_message_starts_with_stable_prefix() {
    assert_eq!(STALLED_PREFIX, "stalled after");
    let msg = stalled_message(780);
    assert!(msg.starts_with(STALLED_PREFIX), "message was: {msg}");
    assert!(msg.contains("780"), "message was: {msg}");
}
