// GAUNTLET filter suite for T-202, written blind from the spec (no implementation read).
//
// SPEC_AMBIGUITY: the spec names no accessor for SeatProfile rows in the listed public API, so
// "changing the profile value changes the consumer output" cannot be expressed here.
// SPEC_AMBIGUITY: whether `stall_window_env_var` normalizes aliases (agy -> gemini) is not stated; only canonical keys are tested.
// SPEC_AMBIGUITY: exact text of `stalled_message` beyond the STALLED_PREFIX and the seconds figure is not pinned by the listed API.
// SPEC_AMBIGUITY: new-seat env var naming is inferred from `TRIUMVIRATE_STALL_SECS_<SEAT>` (REQ-051, upper-case key).

use mcp_bridge::stall::{
    STALLED_PREFIX, default_stall_window_secs, stall_window, stall_window_env_var, stalled_message,
};
use std::sync::Mutex;
use std::time::Duration;

// Serializes tests that mutate process environment variables.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn with_env<R>(var: &str, value: Option<&str>, f: impl FnOnce() -> R) -> R {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var(var).ok();
    // SAFETY: access is serialized by ENV_LOCK; each test uses its own agent's variable.
    unsafe {
        match value {
            Some(v) => std::env::set_var(var, v),
            None => std::env::remove_var(var),
        }
    }
    let out = f();
    unsafe {
        match prev {
            Some(v) => std::env::set_var(var, v),
            None => std::env::remove_var(var),
        }
    }
    out
}

// REQ-051 / T-001 pinned: codex 240 s.
#[test]
fn test_should_return_240_for_codex_default_window() {
    assert_eq!(default_stall_window_secs("codex"), Some(240));
}

// REQ-051 / T-001 pinned: gemini 300 s.
#[test]
fn test_should_return_300_for_gemini_default_window() {
    assert_eq!(default_stall_window_secs("gemini"), Some(300));
}

// REQ-051 / T-001 pinned: deepseek 420 s.
#[test]
fn test_should_return_420_for_deepseek_default_window() {
    assert_eq!(default_stall_window_secs("deepseek"), Some(420));
}

// REQ-051 / T-002 reality test: grok 780 s.
#[test]
fn test_should_return_780_for_grok_default_window() {
    assert_eq!(default_stall_window_secs("grok"), Some(780));
}

// REQ-051: claude is Disabled, never stall-killed.
#[test]
fn test_should_return_none_for_claude_default_window() {
    assert_eq!(default_stall_window_secs("claude"), None);
}

// REQ-051: new seats start at 780 s, the grok value.
#[test]
fn test_should_return_780_for_kimi_default_window() {
    assert_eq!(default_stall_window_secs("kimi"), Some(780));
}

// REQ-051: new seats start at 780 s.
#[test]
fn test_should_return_780_for_mistral_default_window() {
    assert_eq!(default_stall_window_secs("mistral"), Some(780));
}

// REQ-051: new seats start at 780 s.
#[test]
fn test_should_return_780_for_qwen_default_window() {
    assert_eq!(default_stall_window_secs("qwen"), Some(780));
}

// Doc comment: aliases normalize first, agy gets gemini's window.
#[test]
fn test_should_give_agy_the_gemini_window() {
    assert_eq!(default_stall_window_secs("agy"), Some(300));
}

// Doc comment: antigravity gets gemini's window.
#[test]
fn test_should_give_antigravity_the_gemini_window() {
    assert_eq!(default_stall_window_secs("antigravity"), Some(300));
}

// Doc comment: a name with no row has no window.
#[test]
fn test_should_return_none_for_unknown_agent() {
    assert_eq!(default_stall_window_secs("not-a-real-seat"), None);
}

// Doc comment: a name with no row has no window (empty string edge).
#[test]
fn test_should_return_none_for_empty_agent_name() {
    assert_eq!(default_stall_window_secs(""), None);
}

// Doc comment: env var for grok is exactly TRIUMVIRATE_STALL_SECS_GROK.
#[test]
fn test_should_name_grok_env_var() {
    assert_eq!(stall_window_env_var("grok"), "TRIUMVIRATE_STALL_SECS_GROK");
}

// REQ-051: TRIUMVIRATE_STALL_SECS_<SEAT> for the new seats.
#[test]
fn test_should_name_new_seat_env_vars() {
    assert_eq!(stall_window_env_var("kimi"), "TRIUMVIRATE_STALL_SECS_KIMI");
    assert_eq!(stall_window_env_var("mistral"), "TRIUMVIRATE_STALL_SECS_MISTRAL");
    assert_eq!(stall_window_env_var("qwen"), "TRIUMVIRATE_STALL_SECS_QWEN");
}

// Property: every seat's env var is distinct and shares the prefix.
#[test]
fn test_should_give_distinct_env_var_per_seat() {
    let seats = ["codex", "gemini", "grok", "claude", "deepseek", "kimi", "mistral", "qwen"];
    let vars: Vec<String> = seats.iter().map(|s| stall_window_env_var(s)).collect();
    for v in &vars {
        assert!(v.starts_with("TRIUMVIRATE_STALL_SECS_"), "bad prefix: {v}");
    }
    let mut dedup = vars.clone();
    dedup.sort();
    dedup.dedup();
    assert_eq!(dedup.len(), seats.len());
}

// Doc comment: with no override, stall_window is the default.
#[test]
fn test_should_use_default_when_no_override() {
    let var = stall_window_env_var("codex");
    let got = with_env(&var, None, || stall_window("codex"));
    assert_eq!(got, Some(Duration::from_secs(240)));
}

// Doc comment: with no override and no default, stall_window is None (claude).
#[test]
fn test_should_be_none_for_claude_without_override() {
    let var = stall_window_env_var("claude");
    let got = with_env(&var, None, || stall_window("claude"));
    assert_eq!(got, None);
}

// Doc comment: an override that parses wins over the default.
#[test]
fn test_should_use_parsed_override() {
    let var = stall_window_env_var("gemini");
    let got = with_env(&var, Some("123"), || stall_window("gemini"));
    assert_eq!(got, Some(Duration::from_secs(123)));
}

// Doc comment: an override of 0 turns stall detection off.
#[test]
fn test_should_disable_when_override_is_zero() {
    let var = stall_window_env_var("deepseek");
    let got = with_env(&var, Some("0"), || stall_window("deepseek"));
    assert_eq!(got, None);
}

// Doc comment: an unparseable override is ignored (default stays in force).
#[test]
fn test_should_ignore_unparseable_override() {
    let var = stall_window_env_var("grok");
    let got = with_env(&var, Some("soon"), || stall_window("grok"));
    assert_eq!(got, Some(Duration::from_secs(780)));
}

// Doc comment: a negative value does not parse as seconds, so it is ignored.
#[test]
fn test_should_ignore_negative_override() {
    let var = stall_window_env_var("kimi");
    let got = with_env(&var, Some("-5"), || stall_window("kimi"));
    assert_eq!(got, Some(Duration::from_secs(780)));
}

// REQ-051: override applies to a seat with no default window too (claude).
#[test]
fn test_should_apply_override_to_seat_without_default() {
    let var = stall_window_env_var("claude");
    let got = with_env(&var, Some("50"), || stall_window("claude"));
    assert_eq!(got, Some(Duration::from_secs(50)));
}

// REQ-051: new seat env override is honored.
#[test]
fn test_should_apply_override_to_new_seat() {
    let var = stall_window_env_var("qwen");
    let got = with_env(&var, Some("90"), || stall_window("qwen"));
    assert_eq!(got, Some(Duration::from_secs(90)));
}

// Isolation: one agent's override must not leak to another agent.
#[test]
fn test_should_not_leak_override_across_agents() {
    let var = stall_window_env_var("mistral");
    let (mine, other) = with_env(&var, Some("11"), || (stall_window("mistral"), stall_window("codex")));
    assert_eq!(mine, Some(Duration::from_secs(11)));
    assert_eq!(other, Some(Duration::from_secs(240)));
}

// Doc comment: an unknown agent has no window.
#[test]
fn test_should_return_none_window_for_unknown_agent() {
    assert_eq!(stall_window("not-a-real-seat"), None);
}

// Doc comment: STALLED_PREFIX is stable.
#[test]
fn test_should_expose_stable_stalled_prefix() {
    assert_eq!(STALLED_PREFIX, "stalled after");
}

// Doc comment: callers match on the prefix; message carries the seconds.
#[test]
fn test_should_start_stalled_message_with_prefix_and_include_secs() {
    let m = stalled_message(780);
    assert!(m.starts_with(STALLED_PREFIX), "got: {m}");
    assert!(m.contains("780"), "got: {m}");
}

// Property: for any secs the message keeps the prefix and its number.
#[test]
fn test_should_keep_prefix_for_many_values() {
    for secs in [0u64, 1, 59, 240, 300, 420, 780, 900, 86_400, u64::MAX] {
        let m = stalled_message(secs);
        assert!(m.starts_with(STALLED_PREFIX), "got: {m}");
        assert!(m.contains(&secs.to_string()), "got: {m}");
    }
}

// Property: distinct secs produce distinct messages.
#[test]
fn test_should_differ_by_secs() {
    assert_ne!(stalled_message(240), stalled_message(300));
}

// Property: default window is deterministic across calls.
#[test]
fn test_should_be_pure_for_default_window() {
    for s in ["codex", "gemini", "grok", "claude", "deepseek", "kimi", "mistral", "qwen"] {
        assert_eq!(default_stall_window_secs(s), default_stall_window_secs(s));
    }
}
