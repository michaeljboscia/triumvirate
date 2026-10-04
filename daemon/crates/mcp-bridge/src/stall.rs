//! Stall windows: how long an agent may go without real progress before its run is killed.
//!
//! Measured, not guessed. From `~/.triumvirate/outbox.jsonl` on 2026-10-03, for every request that
//! ended DONE, the longest gap between real progress events (the stuck detector's own STUCK events
//! excluded). Share of SUCCESSFUL runs each window would have killed:
//!
//! | agent    |     n | 240 s | 300 s | 420 s | 600 s | 780 s |
//! |----------|-------|-------|-------|-------|-------|-------|
//! | codex    | 1,710 |  0.0% |  0.0% |  0.0% |  0.0% |  0.0% |
//! | gemini   | 5,521 |  0.2% |  0.1% |  0.1% |  0.0% |  0.0% |
//! | deepseek |   537 |  1.1% |  0.9% |  0.2% |  0.0% |  0.0% |
//! | grok     |   592 |  9.5% |  6.8% |  3.0% |  1.2% |  0.0% |
//!
//! Design record: temporal-migration/docs/designs/triumvirate-fleet.md section 4a. The 900 s
//! limits stay as backstops; these sit under them.
//!
//! The windows were fit on the ASK path's event stream. The fleet path's progress clock is output
//! growth, a different signal, so fleet uses these windows only to LOG what it would have done.

use std::time::Duration;

/// The production default for `agent`, in seconds. `None` for an agent with no measured window
/// (it is never stall-killed). `gemini` covers both backends (gemini-cli and agy).
pub fn default_stall_window_secs(agent: &str) -> Option<u64> {
    match agent {
        "codex" => Some(240),
        "gemini" | "agy" | "antigravity" => Some(300),
        "deepseek" => Some(420),
        "grok" => Some(780),
        _ => None,
    }
}

/// The env var that overrides one agent's window, e.g. `TRIUMVIRATE_STALL_SECS_GROK`.
pub fn stall_window_env_var(agent: &str) -> String {
    format!("TRIUMVIRATE_STALL_SECS_{}", agent.to_ascii_uppercase())
}

/// The window in force for `agent`: the env override when it parses, else the default. An
/// override of `0` turns stall detection off for that agent. An unparseable override is ignored
/// with a warning rather than silently disabling the kill.
pub fn stall_window(agent: &str) -> Option<Duration> {
    let var = stall_window_env_var(agent);
    match std::env::var(&var) {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(Duration::from_secs(secs)),
            Err(_) => {
                tracing::warn!(var = %var, value = %raw, "unparseable stall window override; using the default");
                default_stall_window_secs(agent).map(Duration::from_secs)
            }
        },
        Err(_) => default_stall_window_secs(agent).map(Duration::from_secs),
    }
}

/// The terminal error text for a stalled run. Callers match on the prefix, so keep it stable.
pub fn stalled_message(secs: u64) -> String {
    format!("{STALLED_PREFIX} {secs} s without progress")
}

pub const STALLED_PREFIX: &str = "stalled after";

#[cfg(test)]
mod tests {
    use super::*;

    /// The production values, pinned. A test that only ran with an env override could not see a
    /// wrong default, so this one reads the defaults directly.
    /// RED IF: any default drifts from the measured table in the module doc.
    #[test]
    fn production_defaults_are_the_measured_windows() {
        assert_eq!(default_stall_window_secs("codex"), Some(240));
        assert_eq!(default_stall_window_secs("gemini"), Some(300));
        assert_eq!(default_stall_window_secs("agy"), Some(300));
        assert_eq!(default_stall_window_secs("deepseek"), Some(420));
        assert_eq!(default_stall_window_secs("grok"), Some(780));
        assert_eq!(default_stall_window_secs("claude"), None);
        // Every window sits under the 900 s backstop, or the backstop fires first and the
        // stall outcome never happens.
        for agent in ["codex", "gemini", "deepseek", "grok"] {
            assert!(default_stall_window_secs(agent).unwrap() < 900, "{agent}");
        }
    }

    #[test]
    fn env_var_names_are_per_agent() {
        assert_eq!(stall_window_env_var("grok"), "TRIUMVIRATE_STALL_SECS_GROK");
        assert_eq!(stall_window_env_var("codex"), "TRIUMVIRATE_STALL_SECS_CODEX");
    }

    #[test]
    fn the_message_carries_the_prefix_and_the_seconds() {
        assert_eq!(stalled_message(240), "stalled after 240 s without progress");
        assert!(stalled_message(1).starts_with(STALLED_PREFIX));
    }
}
