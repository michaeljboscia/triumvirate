//! T-002 (REQ-DS-002, REQ-DS-003, REQ-DS-015): DeepSeek runtime configuration loader.
//!
//! Single authoritative config owner for all 15 env knobs. Every downstream task that
//! reads DeepSeek settings reads them through `DeepSeekConfig::from_env()` — NOT by calling
//! `std::env::var` directly. This keeps defaults centralised and testable.
//!
//! API_KEY is wrapped in a redacted-Debug newtype (`ApiKey`). The `Debug` impl redacts;
//! plaintext is only available through `ApiKey::expose()` and MUST NOT be passed to
//! `tracing::*`, `eprintln!`, panic messages, error chains, argv, or any serde/Display
//! sink. New callers: search for `.expose()` references and confirm the consumer's
//! sink is not loggable before adding one.

use std::path::PathBuf;
use std::time::Duration;

// ─────────────────────────────────────────────────────────────────────────────
// Redacted Debug wrapper for the API key (REQ-DS-003 guardrail).
// ─────────────────────────────────────────────────────────────────────────────

/// Wrapper around the DeepSeek API key whose `Debug` impl never prints the plaintext.
/// Use `.expose()` to get the raw `&str` for `Authorization: Bearer …`; never pass the
/// wrapper itself into a format string that uses `{:?}`.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hard-redacted. If you find yourself wanting to print a hash or last-4-chars here
        // for debugging, ADD A SEPARATE METHOD — never broaden Debug.
        write!(f, "ApiKey(<redacted>)")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Enums parsed from env strings.
// ─────────────────────────────────────────────────────────────────────────────

/// Whether to enable DeepSeek's thinking/CoT mode for v4 models.
/// Default: Enabled (REQ-DS-005/D-07 — thinking is the reasoning-diversity value).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThinkingMode {
    Enabled,
    Disabled,
}

impl ThinkingMode {
    fn from_env_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "disabled" | "off" | "0" | "false" | "no" => Self::Disabled,
            // default — "enabled", "on", "1", "true", "yes", or anything unrecognised:
            _ => Self::Enabled,
        }
    }

    pub fn as_api_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }
}

/// DeepSeek reasoning-effort tier (only High and Max are real on the wire — low/medium
/// auto-map to High; xhigh maps to Max; see findings/round-2-research.md §C).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReasoningEffort {
    High,
    Max,
}

impl ReasoningEffort {
    fn from_env_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "max" | "xhigh" => Self::Max,
            // "low", "medium", "high", or anything else → High (the API does the same mapping):
            _ => Self::High,
        }
    }

    pub fn as_api_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The config struct — 15 knobs, all defaulted per REQ-DS-015.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct DeepSeekConfig {
    /// REQ-DS-002 access path. Default: `https://api.deepseek.com/v1`.
    pub base_url: String,
    /// REQ-DS-003. Required (no default); empty if env var absent.
    pub api_key: ApiKey,
    /// REQ-DS-005. Default: `deepseek-v4-pro`. Set to `deepseek-v4-flash` to swap models.
    pub model: String,
    /// REQ-DS-005. Default: 32768 (generous; shared with reasoning budget).
    pub max_tokens: u32,
    /// REQ-DS-005 / D-07. Default: Enabled.
    pub thinking: ThinkingMode,
    /// REQ-DS-005. Default: High.
    pub reasoning_effort: ReasoningEffort,
    /// REQ-DS-007. Idle/read timeout (rolling). Default: 60 seconds.
    pub read_timeout: Duration,
    /// REQ-DS-024. Absolute outer ceiling. Default: 1800 seconds.
    pub timeout: Duration,
    /// REQ-DS-007. TCP keep-alive. Default: 30 seconds.
    pub tcp_keepalive: Duration,
    /// REQ-DS-006 / A-05. Daemon-side outbound concurrency cap. Default: 8.
    pub max_concurrent: u32,
    /// REQ-DS-006. Daemon-side soft RPM cap. Default: 60.
    pub max_rpm: u32,
    /// REQ-DS-028. Optional runaway-reasoning early-abort. Default: 0 (disabled).
    /// When >0, MUST be < `max_tokens` (validated below).
    pub reasoning_cap_tokens: u32,
    /// REQ-DS-018 / REQ-DS-023. Where per-request JSON log records go.
    /// Default: `$HOME/.triumvirate/deepseek-logs/`.
    pub log_dir: PathBuf,
    /// REQ-DS-023. Cap on `reasoning_content` size persisted to the per-request log.
    /// Default: 262144 (256 KB).
    pub log_reasoning_cap_bytes: usize,
    /// REQ-DS-025. Anti-bulk byte-size intercept threshold on `ask_agent` deepseek payload.
    /// Default: 16384 (16 KB).
    pub bulk_bytes: usize,
}

#[derive(Debug)]
pub enum ConfigError {
    ReasoningCapTooLarge {
        cap: u32,
        max_tokens: u32,
    },
    /// Codex P5-review fix: a typo in a numeric env var used to silently fall back
    /// to the documented default. For a paid API surface that's a foot-gun — an
    /// operator setting `MAX_TOKENS=oops` got 32768 with no signal. Now fails loud.
    InvalidEnv {
        name: &'static str,
        value: String,
        expected: &'static str,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::ReasoningCapTooLarge { cap, max_tokens } => write!(
                f,
                "TRIUMVIRATE_DEEPSEEK_REASONING_CAP_TOKENS ({cap}) must be < TRIUMVIRATE_DEEPSEEK_MAX_TOKENS ({max_tokens})"
            ),
            ConfigError::InvalidEnv {
                name,
                value,
                expected,
            } => write!(
                f,
                "{name}={value:?} is not a valid value; expected {expected}"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

impl DeepSeekConfig {
    /// Load all 15 knobs from env, applying documented defaults for anything absent.
    /// Returns Err only on validation failures (e.g. reasoning cap >= max_tokens).
    /// An ABSENT or empty `TRIUMVIRATE_DEEPSEEK_API_KEY` is NOT an error here — the
    /// runner is responsible for noticing an empty key and failing loud at request time.
    pub fn from_env() -> Result<Self, ConfigError> {
        let base_url = read_env("TRIUMVIRATE_DEEPSEEK_BASE_URL")
            .unwrap_or_else(|| "https://api.deepseek.com/v1".to_string());

        let api_key = ApiKey::new(
            read_env("TRIUMVIRATE_DEEPSEEK_API_KEY").unwrap_or_default(),
        );

        let model = read_env("TRIUMVIRATE_DEEPSEEK_MODEL")
            .unwrap_or_else(|| "deepseek-v4-pro".to_string());

        // Codex P5-review: numeric env vars now fail loud on parse errors and reject
        // zero for knobs where it's nonsensical (timeouts, concurrency caps, log caps,
        // max_tokens, bulk_bytes). reasoning_cap_tokens explicitly permits zero (= disabled).
        let max_tokens =
            read_env_u32_nonzero("TRIUMVIRATE_DEEPSEEK_MAX_TOKENS")?.unwrap_or(32768);

        let thinking = read_env("TRIUMVIRATE_DEEPSEEK_THINKING")
            .map(|s| ThinkingMode::from_env_str(&s))
            .unwrap_or(ThinkingMode::Enabled);

        let reasoning_effort = read_env("TRIUMVIRATE_DEEPSEEK_REASONING_EFFORT")
            .map(|s| ReasoningEffort::from_env_str(&s))
            .unwrap_or(ReasoningEffort::High);

        let read_timeout = Duration::from_secs(
            read_env_u64_nonzero("TRIUMVIRATE_DEEPSEEK_READ_TIMEOUT_SECS")?.unwrap_or(60),
        );

        let timeout = Duration::from_secs(
            read_env_u64_nonzero("TRIUMVIRATE_DEEPSEEK_TIMEOUT_SECS")?.unwrap_or(1800),
        );

        let tcp_keepalive = Duration::from_secs(
            read_env_u64_nonzero("TRIUMVIRATE_DEEPSEEK_TCP_KEEPALIVE_SECS")?.unwrap_or(30),
        );

        let max_concurrent =
            read_env_u32_nonzero("TRIUMVIRATE_DEEPSEEK_MAX_CONCURRENT")?.unwrap_or(8);

        let max_rpm = read_env_u32_nonzero("TRIUMVIRATE_DEEPSEEK_MAX_RPM")?.unwrap_or(60);

        // Zero is meaningful here (= runaway abort disabled), so we use the plain parser.
        let reasoning_cap_tokens =
            read_env_u32("TRIUMVIRATE_DEEPSEEK_REASONING_CAP_TOKENS")?.unwrap_or(0);

        let log_dir = read_env("TRIUMVIRATE_DEEPSEEK_LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(default_log_dir);

        let log_reasoning_cap_bytes =
            read_env_usize_nonzero("TRIUMVIRATE_DEEPSEEK_LOG_REASONING_CAP_BYTES")?
                .unwrap_or(262_144);

        let bulk_bytes =
            read_env_usize_nonzero("TRIUMVIRATE_DEEPSEEK_BULK_BYTES")?.unwrap_or(16_384);

        // Validation: REQ-DS-028 — when reasoning cap is enabled, it MUST be < max_tokens
        // (otherwise it's mathematically unreachable; see round-3-deltas.md F-01).
        if reasoning_cap_tokens > 0 && reasoning_cap_tokens >= max_tokens {
            return Err(ConfigError::ReasoningCapTooLarge {
                cap: reasoning_cap_tokens,
                max_tokens,
            });
        }

        Ok(Self {
            base_url,
            api_key,
            model,
            max_tokens,
            thinking,
            reasoning_effort,
            read_timeout,
            timeout,
            tcp_keepalive,
            max_concurrent,
            max_rpm,
            reasoning_cap_tokens,
            log_dir,
            log_reasoning_cap_bytes,
            bulk_bytes,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers.
// ─────────────────────────────────────────────────────────────────────────────

fn read_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

// Codex P5-review fix: numeric parsers now fail loud on garbage values (and the
// `_nonzero` variants also reject 0 for knobs where 0 is meaningless). Each helper
// requires a `&'static str` name so the ConfigError can carry the env-var name
// without an allocation.
fn read_env_u32(name: &'static str) -> Result<Option<u32>, ConfigError> {
    parse_env(name, "non-negative integer (u32)")
}

fn read_env_u32_nonzero(name: &'static str) -> Result<Option<u32>, ConfigError> {
    let v = parse_env::<u32>(name, "positive integer (u32, > 0)")?;
    reject_zero(name, v, "positive integer (> 0)")
}

fn read_env_u64_nonzero(name: &'static str) -> Result<Option<u64>, ConfigError> {
    let v = parse_env::<u64>(name, "positive integer (u64, > 0)")?;
    reject_zero(name, v, "positive integer (> 0)")
}

fn read_env_usize_nonzero(name: &'static str) -> Result<Option<usize>, ConfigError> {
    let v = parse_env::<usize>(name, "positive integer (usize, > 0)")?;
    reject_zero(name, v, "positive integer (> 0)")
}

fn parse_env<T>(name: &'static str, expected: &'static str) -> Result<Option<T>, ConfigError>
where
    T: std::str::FromStr,
{
    match read_env(name) {
        None => Ok(None),
        Some(raw) => raw
            .parse::<T>()
            .map(Some)
            .map_err(|_| ConfigError::InvalidEnv {
                name,
                value: raw,
                expected,
            }),
    }
}

fn reject_zero<T>(
    name: &'static str,
    parsed: Option<T>,
    expected: &'static str,
) -> Result<Option<T>, ConfigError>
where
    T: Copy + PartialEq + Default + std::fmt::Display,
{
    match parsed {
        Some(v) if v == T::default() => Err(ConfigError::InvalidEnv {
            name,
            value: format!("{v}"),
            expected,
        }),
        other => Ok(other),
    }
}

fn default_log_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".triumvirate").join("deepseek-logs")
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests.
//
// Tests touch process-global env vars, so they're serialized via a Mutex to avoid
// flakes when `cargo test` runs them concurrently. Each test saves + restores the
// env vars it touches so subsequent tests (in any order) see a clean baseline.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // All 15 env vars this module reads.
    const ENV_VARS: &[&str] = &[
        "TRIUMVIRATE_DEEPSEEK_BASE_URL",
        "TRIUMVIRATE_DEEPSEEK_API_KEY",
        "TRIUMVIRATE_DEEPSEEK_MODEL",
        "TRIUMVIRATE_DEEPSEEK_MAX_TOKENS",
        "TRIUMVIRATE_DEEPSEEK_THINKING",
        "TRIUMVIRATE_DEEPSEEK_REASONING_EFFORT",
        "TRIUMVIRATE_DEEPSEEK_READ_TIMEOUT_SECS",
        "TRIUMVIRATE_DEEPSEEK_TIMEOUT_SECS",
        "TRIUMVIRATE_DEEPSEEK_TCP_KEEPALIVE_SECS",
        "TRIUMVIRATE_DEEPSEEK_MAX_CONCURRENT",
        "TRIUMVIRATE_DEEPSEEK_MAX_RPM",
        "TRIUMVIRATE_DEEPSEEK_REASONING_CAP_TOKENS",
        "TRIUMVIRATE_DEEPSEEK_LOG_DIR",
        "TRIUMVIRATE_DEEPSEEK_LOG_REASONING_CAP_BYTES",
        "TRIUMVIRATE_DEEPSEEK_BULK_BYTES",
    ];

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard: clears `ENV_VARS` on construction and restores their original
    /// values on drop. Codex P5-review: a panicking test under the previous closure
    /// helper left env vars cleared, so the next test ran under a corrupted process
    /// env. Drop runs on unwind, so this is panic-safe.
    struct EnvGuard {
        saved: Vec<(&'static str, Option<String>)>,
        // Lock guard kept alive for the duration of the test; on poison we drain the
        // poison and continue (a previous test panicked but our restore-in-Drop
        // already made the env consistent for whoever holds the lock next).
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn acquire() -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let saved: Vec<(&'static str, Option<String>)> = ENV_VARS
                .iter()
                .map(|k| (*k, std::env::var(k).ok()))
                .collect();
            for k in ENV_VARS {
                // SAFETY: tests are serialized by ENV_LOCK; this scope owns the env namespace.
                unsafe { std::env::remove_var(k); }
            }
            Self { saved, _lock: lock }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for k in ENV_VARS {
                unsafe { std::env::remove_var(k); }
            }
            for (k, v) in &self.saved {
                if let Some(val) = v {
                    unsafe { std::env::set_var(k, val); }
                }
            }
        }
    }

    /// Run `f` with the ENV_VARS list cleared (saved + restored after, even on
    /// panic), holding the process-global ENV_LOCK to prevent concurrent test
    /// interference. Tests should set whichever env vars they need INSIDE `f`.
    fn with_clean_env<F: FnOnce()>(f: F) {
        let _g = EnvGuard::acquire();
        f();
        // _g drops here (or on unwind if f panics) — env is restored either way.
    }

    fn set_env(name: &str, value: &str) {
        // SAFETY: callers run under ENV_LOCK via with_clean_env.
        unsafe { std::env::set_var(name, value); }
    }

    // T-002 reality test: with the API key set and all other knobs unset, from_env()
    // produces a struct whose every field equals the documented default per REQ-DS-015.
    // A stub that always returns Default would fail the api_key assertion.
    #[test]
    fn from_env_returns_documented_defaults_for_all_15_knobs() {
        with_clean_env(|| {
            set_env("TRIUMVIRATE_DEEPSEEK_API_KEY", "sk-test-key-do-not-log");
            let cfg = DeepSeekConfig::from_env().expect("config loads");

            assert_eq!(cfg.base_url, "https://api.deepseek.com/v1");
            assert_eq!(cfg.api_key.expose(), "sk-test-key-do-not-log");
            assert_eq!(cfg.model, "deepseek-v4-pro");
            assert_eq!(cfg.max_tokens, 32768);
            assert_eq!(cfg.thinking, ThinkingMode::Enabled);
            assert_eq!(cfg.reasoning_effort, ReasoningEffort::High);
            assert_eq!(cfg.read_timeout, Duration::from_secs(60));
            assert_eq!(cfg.timeout, Duration::from_secs(1800));
            assert_eq!(cfg.tcp_keepalive, Duration::from_secs(30));
            assert_eq!(cfg.max_concurrent, 8);
            assert_eq!(cfg.max_rpm, 60);
            assert_eq!(cfg.reasoning_cap_tokens, 0);
            // LOG_DIR default is ~/.triumvirate/deepseek-logs (HOME-relative).
            assert!(
                cfg.log_dir.ends_with("deepseek-logs"),
                "log_dir should end with deepseek-logs; got {:?}",
                cfg.log_dir
            );
            assert_eq!(cfg.log_reasoning_cap_bytes, 262_144);
            assert_eq!(cfg.bulk_bytes, 16_384);
        });
    }

    // The capitaliser-trap equivalent for the secret-redaction guard.
    // A Debug impl that exposes the inner string (e.g. via #[derive(Debug)] or
    // {:?} on the whole struct) would fail this.
    #[test]
    fn api_key_debug_is_redacted() {
        let key = ApiKey::new("sk-VERY-SECRET-token-value-12345");
        let debug_str = format!("{:?}", key);
        assert!(
            !debug_str.contains("sk-VERY-SECRET-token-value-12345"),
            "ApiKey Debug must NOT expose the secret; got: {debug_str}"
        );
        assert!(
            debug_str.contains("redacted") || debug_str.contains("ApiKey"),
            "Debug should mark this as a redacted ApiKey; got: {debug_str}"
        );
    }

    // The whole DeepSeekConfig must also redact the secret via #[derive(Debug)]
    // — confirms the ApiKey wrapper actually flows through the struct's Debug.
    #[test]
    fn deepseek_config_debug_does_not_leak_api_key() {
        with_clean_env(|| {
            set_env("TRIUMVIRATE_DEEPSEEK_API_KEY", "sk-do-not-leak-this-string-XYZ");
            let cfg = DeepSeekConfig::from_env().unwrap();
            let debug_str = format!("{:?}", cfg);
            assert!(
                !debug_str.contains("sk-do-not-leak-this-string-XYZ"),
                "DeepSeekConfig Debug leaked the API key; got: {debug_str}"
            );
        });
    }

    #[test]
    fn from_env_parses_overrides() {
        with_clean_env(|| {
            set_env("TRIUMVIRATE_DEEPSEEK_API_KEY", "sk-x");
            set_env("TRIUMVIRATE_DEEPSEEK_MODEL", "deepseek-v4-flash");
            set_env("TRIUMVIRATE_DEEPSEEK_MAX_TOKENS", "8000");
            set_env("TRIUMVIRATE_DEEPSEEK_THINKING", "disabled");
            set_env("TRIUMVIRATE_DEEPSEEK_REASONING_EFFORT", "max");
            set_env("TRIUMVIRATE_DEEPSEEK_BASE_URL", "http://127.0.0.1:9999");
            set_env("TRIUMVIRATE_DEEPSEEK_READ_TIMEOUT_SECS", "10");
            set_env("TRIUMVIRATE_DEEPSEEK_TIMEOUT_SECS", "300");
            set_env("TRIUMVIRATE_DEEPSEEK_MAX_CONCURRENT", "2");
            set_env("TRIUMVIRATE_DEEPSEEK_BULK_BYTES", "1024");

            let cfg = DeepSeekConfig::from_env().unwrap();
            assert_eq!(cfg.model, "deepseek-v4-flash");
            assert_eq!(cfg.max_tokens, 8000);
            assert_eq!(cfg.thinking, ThinkingMode::Disabled);
            assert_eq!(cfg.reasoning_effort, ReasoningEffort::Max);
            assert_eq!(cfg.base_url, "http://127.0.0.1:9999");
            assert_eq!(cfg.read_timeout, Duration::from_secs(10));
            assert_eq!(cfg.timeout, Duration::from_secs(300));
            assert_eq!(cfg.max_concurrent, 2);
            assert_eq!(cfg.bulk_bytes, 1024);
        });
    }

    #[test]
    fn reasoning_effort_xhigh_maps_to_max() {
        assert_eq!(ReasoningEffort::from_env_str("xhigh"), ReasoningEffort::Max);
        assert_eq!(ReasoningEffort::from_env_str("max"), ReasoningEffort::Max);
        assert_eq!(ReasoningEffort::from_env_str("high"), ReasoningEffort::High);
        assert_eq!(ReasoningEffort::from_env_str("low"), ReasoningEffort::High);
        assert_eq!(ReasoningEffort::from_env_str("medium"), ReasoningEffort::High);
    }

    #[test]
    fn thinking_mode_parses_truthy_falsy() {
        assert_eq!(ThinkingMode::from_env_str("enabled"), ThinkingMode::Enabled);
        assert_eq!(ThinkingMode::from_env_str("on"), ThinkingMode::Enabled);
        assert_eq!(ThinkingMode::from_env_str("true"), ThinkingMode::Enabled);
        assert_eq!(ThinkingMode::from_env_str("disabled"), ThinkingMode::Disabled);
        assert_eq!(ThinkingMode::from_env_str("off"), ThinkingMode::Disabled);
        assert_eq!(ThinkingMode::from_env_str("0"), ThinkingMode::Disabled);
        // garbage defaults to enabled (the seat's purpose):
        assert_eq!(ThinkingMode::from_env_str("???"), ThinkingMode::Enabled);
    }

    // REQ-DS-028 validation: reasoning cap >= max_tokens must error out at config load
    // (otherwise the cap is mathematically unreachable — the round-3 paradox we fixed).
    #[test]
    fn reasoning_cap_must_be_less_than_max_tokens() {
        with_clean_env(|| {
            set_env("TRIUMVIRATE_DEEPSEEK_API_KEY", "sk-x");
            set_env("TRIUMVIRATE_DEEPSEEK_MAX_TOKENS", "1000");
            set_env("TRIUMVIRATE_DEEPSEEK_REASONING_CAP_TOKENS", "2000");
            let err = DeepSeekConfig::from_env().unwrap_err();
            match err {
                ConfigError::ReasoningCapTooLarge { cap, max_tokens } => {
                    assert_eq!(cap, 2000);
                    assert_eq!(max_tokens, 1000);
                }
                other => panic!("expected ReasoningCapTooLarge, got {other:?}"),
            }
        });
    }

    // Codex P5-review regression: garbage values must FAIL LOUD, not silently
    // fall back to the default. Previously `MAX_TOKENS=oops` quietly became 32768.
    #[test]
    fn invalid_numeric_env_fails_loud() {
        with_clean_env(|| {
            set_env("TRIUMVIRATE_DEEPSEEK_API_KEY", "sk-x");
            set_env("TRIUMVIRATE_DEEPSEEK_MAX_TOKENS", "oops");
            let err = DeepSeekConfig::from_env().unwrap_err();
            match err {
                ConfigError::InvalidEnv { name, value, .. } => {
                    assert_eq!(name, "TRIUMVIRATE_DEEPSEEK_MAX_TOKENS");
                    assert_eq!(value, "oops");
                }
                other => panic!("expected InvalidEnv, got {other:?}"),
            }
        });
    }

    // Codex P5-review regression: zero is nonsensical for timeouts, concurrency,
    // and byte caps — reject it instead of producing a runtime no-op.
    #[test]
    fn zero_rejected_on_knobs_where_it_makes_no_sense() {
        for var in &[
            "TRIUMVIRATE_DEEPSEEK_TIMEOUT_SECS",
            "TRIUMVIRATE_DEEPSEEK_READ_TIMEOUT_SECS",
            "TRIUMVIRATE_DEEPSEEK_TCP_KEEPALIVE_SECS",
            "TRIUMVIRATE_DEEPSEEK_MAX_CONCURRENT",
            "TRIUMVIRATE_DEEPSEEK_MAX_RPM",
            "TRIUMVIRATE_DEEPSEEK_MAX_TOKENS",
            "TRIUMVIRATE_DEEPSEEK_BULK_BYTES",
            "TRIUMVIRATE_DEEPSEEK_LOG_REASONING_CAP_BYTES",
        ] {
            with_clean_env(|| {
                set_env("TRIUMVIRATE_DEEPSEEK_API_KEY", "sk-x");
                set_env(var, "0");
                let err = DeepSeekConfig::from_env().unwrap_err();
                match err {
                    ConfigError::InvalidEnv { name, .. } => assert_eq!(name, *var),
                    other => panic!("expected InvalidEnv for {var}, got {other:?}"),
                }
            });
        }
    }

    // Codex P5-review regression: a panicking test under the closure must still
    // restore env vars, otherwise subsequent tests see a corrupted process env.
    // Uses catch_unwind to assert the panic happened AND that env was restored.
    #[test]
    fn with_clean_env_restores_on_panic() {
        // Mark a sentinel BEFORE the panicking call and confirm it survives.
        let sentinel = "TRIUMVIRATE_DEEPSEEK_BASE_URL";
        let sentinel_value = "http://before-panic.example.invalid";
        // Set OUTSIDE the closure so it's part of `saved` — but we need to set it
        // under the lock to avoid racing with concurrent tests.
        let lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        unsafe { std::env::set_var(sentinel, sentinel_value); }
        drop(lock);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_clean_env(|| {
                set_env(sentinel, "http://during-panic.example.invalid");
                panic!("simulated test failure");
            });
        }));
        assert!(result.is_err(), "panic should have propagated");

        // The sentinel must be back to its pre-test value (proves Drop ran on unwind).
        let lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let observed = std::env::var(sentinel).ok();
        // Clean up our sentinel before releasing the lock so we don't leak it.
        unsafe { std::env::remove_var(sentinel); }
        drop(lock);
        assert_eq!(
            observed.as_deref(),
            Some(sentinel_value),
            "EnvGuard::drop must restore env on unwind"
        );
    }

    #[test]
    fn reasoning_cap_zero_means_disabled_no_validation() {
        with_clean_env(|| {
            set_env("TRIUMVIRATE_DEEPSEEK_API_KEY", "sk-x");
            set_env("TRIUMVIRATE_DEEPSEEK_REASONING_CAP_TOKENS", "0");
            let cfg = DeepSeekConfig::from_env().expect("cap=0 is disabled, must load");
            assert_eq!(cfg.reasoning_cap_tokens, 0);
        });
    }
}
