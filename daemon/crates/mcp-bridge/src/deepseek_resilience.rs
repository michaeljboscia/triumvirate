//! T-004 (REQ-DS-006, REQ-DS-010): DeepSeek-specific resilience primitives.
//!
//! Three independent pieces, all driven by `DeepSeekConfig` knobs (T-002):
//!
//!   1. `Semaphore` wrapper for the outbound concurrency cap (max_concurrent, default 8).
//!   2. `TokenBucket` for the soft RPM cap (max_rpm, default 60).
//!   3. `Breaker` — three-state circuit with explicit branches for the two failure
//!      modes DeepSeek actually exhibits: HTTP 402 ("insufficient balance" → HARD,
//!      no auto-recovery) and HTTP 429/5xx ("transient" → exponential cooldown then
//!      half-open lease). This is INTENTIONALLY not a copy of agy_resilience.rs —
//!      agy's reset-window-cooldown is keyed to quota windows; DeepSeek's is keyed
//!      to consecutive transient failures.
//!
//! `classify(status_code) -> Classification` is the single source of truth for
//! "is this a hard stop or a try-again?" decisions. Callers (T-010 runner) must
//! route every non-2xx through it instead of pattern-matching status codes inline.
//!
//! All time-dependent code takes `now: Instant` as a parameter so tests can drive
//! the state machine with a mock clock instead of `tokio::time::sleep`.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

// ─────────────────────────────────────────────────────────────────────────────
// Status-code classification.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classification {
    /// Operator action required (auth, payment, malformed request). Do NOT retry.
    Hard,
    /// Try again after backoff (rate-limit, transient server fault).
    Transient,
}

/// REQ-DS-006/010 single source of truth. Codex P5-review note: kept narrow and
/// explicit — every status code the runner cares about is enumerated.
pub fn classify(status: u16) -> Classification {
    match status {
        // Hard:
        // - 400 malformed JSON / unknown field
        // - 401 bad/missing API key
        // - 402 insufficient balance (DeepSeek-specific signal)
        // - 403 forbidden
        // - 422 schema-valid but semantically invalid
        400 | 401 | 402 | 403 | 422 => Classification::Hard,
        // Transient:
        // - 429 rate-limited
        // - 5xx server faults
        429 | 500..=599 => Classification::Transient,
        // Everything else (1xx, 2xx that somehow reached us, 3xx redirects, ...)
        // — treat as transient. The runner won't retry forever; the breaker will
        // open after `transient_threshold` consecutive failures.
        _ => Classification::Transient,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Concurrency cap (Semaphore wrapper).
// ─────────────────────────────────────────────────────────────────────────────

/// Cheap clone — wraps the inner `Semaphore` in an `Arc` so multiple runner
/// tasks share one cap.
#[derive(Clone)]
pub struct ConcurrencyCap {
    sem: Arc<Semaphore>,
}

impl ConcurrencyCap {
    pub fn new(max_concurrent: u32) -> Self {
        // u32 → usize: the cap is always a small operator-set number.
        Self {
            sem: Arc::new(Semaphore::new(max_concurrent as usize)),
        }
    }

    /// Acquire one permit for the duration of the returned guard. Drop the guard
    /// to release. Awaits if the cap is currently saturated.
    pub async fn acquire(&self) -> tokio::sync::OwnedSemaphorePermit {
        // unwrap is safe: we never call `Semaphore::close()`.
        self.sem
            .clone()
            .acquire_owned()
            .await
            .expect("ConcurrencyCap semaphore was unexpectedly closed")
    }

    pub fn available(&self) -> usize {
        self.sem.available_permits()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Token bucket (RPM cap).
// ─────────────────────────────────────────────────────────────────────────────

/// Standard leaky token bucket. The bucket starts FULL (so the first burst of up
/// to `capacity` requests is free). `try_take(now)` is non-blocking and pure —
/// callers that want to await the next available token compose this with
/// `tokio::time::sleep_until(self.next_available(now))`.
pub struct TokenBucket {
    capacity: f64,
    available: f64,
    refill_per_sec: f64,
    last_refill: Instant,
}

impl TokenBucket {
    /// `max_rpm` requests per minute, capacity equal to one minute's worth.
    pub fn new(max_rpm: u32, now: Instant) -> Self {
        let capacity = max_rpm as f64;
        Self {
            capacity,
            available: capacity,
            refill_per_sec: capacity / 60.0,
            last_refill: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        if now <= self.last_refill {
            return;
        }
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.available = (self.available + elapsed * self.refill_per_sec).min(self.capacity);
        self.last_refill = now;
    }

    /// Returns true if a token was taken; false if the bucket was empty.
    pub fn try_take(&mut self, now: Instant) -> bool {
        self.refill(now);
        if self.available >= 1.0 {
            self.available -= 1.0;
            true
        } else {
            false
        }
    }

    /// When the next token will be available (== `now` if one is already available).
    pub fn next_available(&self, now: Instant) -> Instant {
        // Refill is monotone in `now`, so we compute against the (uncommitted)
        // projected available count without mutating state.
        let projected = if now > self.last_refill {
            let elapsed = now.duration_since(self.last_refill).as_secs_f64();
            (self.available + elapsed * self.refill_per_sec).min(self.capacity)
        } else {
            self.available
        };
        if projected >= 1.0 {
            return now;
        }
        let needed = 1.0 - projected;
        let wait_secs = needed / self.refill_per_sec;
        now + Duration::from_secs_f64(wait_secs)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Three-state breaker.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakerState {
    /// Default state. Requests pass through.
    Closed,
    /// HTTP 402 received. The account has insufficient balance — no automatic
    /// recovery. Operator must intervene (refill, rotate key). The breaker stays
    /// here until explicitly reset.
    HardOpenInsufficientBalance,
    /// Consecutive transient failures (429/5xx) crossed the threshold. Stays
    /// open until `until`. `attempts` records how many *transient-open* cycles
    /// this is (for backoff growth).
    OpenTransient { until: Instant, attempts: u32 },
    /// One request is allowed through to probe recovery. If it succeeds → Closed;
    /// if it fails → OpenTransient with grown cooldown. The lease expires at
    /// `lease_expires_at` to prevent a stuck probe from holding the door open.
    HalfOpen { lease_expires_at: Instant, attempts: u32 },
}

/// The outcome a caller reports to the breaker after each attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    HardError(u16),
    TransientError(u16),
}

/// What `try_acquire(now)` returns — either the caller may proceed, or the
/// breaker is open and the caller must abort (with a reason).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcquireDecision {
    Allow,
    BlockHard,
    BlockTransient { retry_after: Duration },
}

pub struct BreakerConfig {
    /// Consecutive transient failures before opening. Default: 3.
    pub transient_threshold: u32,
    /// Base cooldown on first open. Default: 30s.
    pub base_cooldown: Duration,
    /// Multiplier for each subsequent open. Default: 2.0 (capped at `max_cooldown`).
    pub backoff_multiplier: f64,
    /// Hard ceiling on cooldown growth. Default: 10 minutes.
    pub max_cooldown: Duration,
    /// How long a half-open probe has to complete. Default: 60s.
    pub half_open_lease: Duration,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            transient_threshold: 3,
            base_cooldown: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            max_cooldown: Duration::from_secs(600),
            half_open_lease: Duration::from_secs(60),
        }
    }
}

pub struct Breaker {
    state: BreakerState,
    consecutive_transient: u32,
    cfg: BreakerConfig,
}

impl Breaker {
    pub fn new(cfg: BreakerConfig) -> Self {
        Self {
            state: BreakerState::Closed,
            consecutive_transient: 0,
            cfg,
        }
    }

    pub fn state(&self) -> BreakerState {
        self.state
    }

    /// Gate a request. Pass `now` so tests can drive transitions without sleeping.
    /// If this returns `Allow` AND the state was `OpenTransient` but the cooldown
    /// has elapsed, the breaker transitions to `HalfOpen` as a side effect.
    pub fn try_acquire(&mut self, now: Instant) -> AcquireDecision {
        match self.state {
            BreakerState::Closed => AcquireDecision::Allow,
            BreakerState::HardOpenInsufficientBalance => AcquireDecision::BlockHard,
            BreakerState::OpenTransient { until, attempts } => {
                if now >= until {
                    // Cooldown elapsed → grant one half-open probe.
                    self.state = BreakerState::HalfOpen {
                        lease_expires_at: now + self.cfg.half_open_lease,
                        attempts,
                    };
                    AcquireDecision::Allow
                } else {
                    AcquireDecision::BlockTransient {
                        retry_after: until.duration_since(now),
                    }
                }
            }
            BreakerState::HalfOpen {
                lease_expires_at,
                attempts,
            } => {
                if now >= lease_expires_at {
                    // The probe never reported back. Treat as a transient failure
                    // (re-open with the next-step cooldown) and refuse this caller.
                    let next_cooldown = self.next_cooldown(attempts + 1);
                    self.state = BreakerState::OpenTransient {
                        until: now + next_cooldown,
                        attempts: attempts + 1,
                    };
                    AcquireDecision::BlockTransient {
                        retry_after: next_cooldown,
                    }
                } else {
                    // Lease is still valid but a probe is already in flight; refuse.
                    // (The single in-flight probe is allowed by the transition above;
                    // any subsequent caller hits this branch.)
                    AcquireDecision::BlockTransient {
                        retry_after: lease_expires_at.duration_since(now),
                    }
                }
            }
        }
    }

    /// Report the result of a request the breaker previously allowed.
    pub fn record(&mut self, outcome: Outcome, now: Instant) {
        match outcome {
            Outcome::HardError(402) => {
                self.state = BreakerState::HardOpenInsufficientBalance;
                self.consecutive_transient = 0;
            }
            Outcome::HardError(_) => {
                // Non-402 hard errors (400/401/403/422) reset the transient
                // streak but don't open the breaker — those indicate caller bugs,
                // not provider faults, so we let the caller learn and continue.
                self.consecutive_transient = 0;
            }
            Outcome::TransientError(_) => {
                self.consecutive_transient += 1;
                // Step the open count based on current state:
                let attempts = match self.state {
                    BreakerState::OpenTransient { attempts, .. }
                    | BreakerState::HalfOpen { attempts, .. } => attempts + 1,
                    _ => 1,
                };
                if self.consecutive_transient >= self.cfg.transient_threshold
                    || matches!(self.state, BreakerState::HalfOpen { .. })
                {
                    let cooldown = self.next_cooldown(attempts);
                    self.state = BreakerState::OpenTransient {
                        until: now + cooldown,
                        attempts,
                    };
                }
            }
            Outcome::Success => {
                self.consecutive_transient = 0;
                self.state = BreakerState::Closed;
            }
        }
    }

    fn next_cooldown(&self, attempts: u32) -> Duration {
        // attempts=1 → base; attempts=2 → base*mult; capped at max.
        let exp = attempts.saturating_sub(1) as i32;
        let factor = self.cfg.backoff_multiplier.powi(exp);
        let secs = (self.cfg.base_cooldown.as_secs_f64() * factor)
            .min(self.cfg.max_cooldown.as_secs_f64());
        Duration::from_secs_f64(secs)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests — drive the breaker with a mock Instant clock.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── classify ────────────────────────────────────────────────────────────

    #[test]
    fn classify_matches_spec_table() {
        // Hard codes
        assert_eq!(classify(400), Classification::Hard);
        assert_eq!(classify(401), Classification::Hard);
        assert_eq!(classify(402), Classification::Hard);
        assert_eq!(classify(403), Classification::Hard);
        assert_eq!(classify(422), Classification::Hard);
        // Transient codes
        assert_eq!(classify(429), Classification::Transient);
        assert_eq!(classify(500), Classification::Transient);
        assert_eq!(classify(502), Classification::Transient);
        assert_eq!(classify(503), Classification::Transient);
        assert_eq!(classify(599), Classification::Transient);
    }

    // ── breaker ─────────────────────────────────────────────────────────────

    fn fresh_breaker() -> Breaker {
        Breaker::new(BreakerConfig::default())
    }

    // Reality test from the IMPL_PLAN: {Ok,Ok,Err(402)} → HardOpenInsufficientBalance.
    // A stub that returns Closed regardless would fail the 402 case.
    #[test]
    fn breaker_402_opens_hard_no_recovery() {
        let mut b = fresh_breaker();
        let t0 = Instant::now();
        b.record(Outcome::Success, t0);
        b.record(Outcome::Success, t0);
        b.record(Outcome::HardError(402), t0);
        assert_eq!(b.state(), BreakerState::HardOpenInsufficientBalance);

        // Advancing time does NOT recover (operator must reset).
        let way_later = t0 + Duration::from_secs(86_400);
        assert_eq!(b.try_acquire(way_later), AcquireDecision::BlockHard);
        assert_eq!(b.state(), BreakerState::HardOpenInsufficientBalance);
    }

    // Reality test: {Err(429),Err(429),Err(429)} → OpenTransient with cooldown>0.
    #[test]
    fn breaker_three_consecutive_429_opens_transient_with_cooldown() {
        let mut b = fresh_breaker();
        let t0 = Instant::now();
        b.record(Outcome::TransientError(429), t0);
        b.record(Outcome::TransientError(429), t0);
        b.record(Outcome::TransientError(429), t0);
        match b.state() {
            BreakerState::OpenTransient { until, attempts } => {
                assert!(until > t0, "cooldown must be in the future");
                assert_eq!(attempts, 1);
            }
            other => panic!("expected OpenTransient, got {other:?}"),
        }

        // Mid-cooldown: BlockTransient with positive retry_after.
        let mid = t0 + Duration::from_secs(5);
        match b.try_acquire(mid) {
            AcquireDecision::BlockTransient { retry_after } => {
                assert!(retry_after > Duration::ZERO);
            }
            other => panic!("expected BlockTransient mid-cooldown, got {other:?}"),
        }
    }

    // Reality test: advance mock clock past cooldown → HalfOpen.
    #[test]
    fn breaker_transitions_to_half_open_after_cooldown() {
        let mut b = fresh_breaker();
        let t0 = Instant::now();
        for _ in 0..3 {
            b.record(Outcome::TransientError(503), t0);
        }
        // Default base_cooldown is 30s — advance 31s and try to acquire.
        let past_cooldown = t0 + Duration::from_secs(31);
        assert_eq!(b.try_acquire(past_cooldown), AcquireDecision::Allow);
        match b.state() {
            BreakerState::HalfOpen { lease_expires_at, .. } => {
                assert!(lease_expires_at > past_cooldown);
            }
            other => panic!("expected HalfOpen after cooldown, got {other:?}"),
        }
    }

    // Half-open success → Closed; half-open failure → OpenTransient with longer
    // cooldown. Confirms the breaker can fully recover AND escalates on re-fail.
    #[test]
    fn breaker_half_open_success_closes_failure_reopens_with_backoff() {
        let mut b = fresh_breaker();
        let t0 = Instant::now();
        for _ in 0..3 {
            b.record(Outcome::TransientError(503), t0);
        }
        let after_cd1 = t0 + Duration::from_secs(31);
        let _ = b.try_acquire(after_cd1); // → HalfOpen
        // Recover.
        b.record(Outcome::Success, after_cd1);
        assert_eq!(b.state(), BreakerState::Closed);

        // Reopen with 3 more transient failures.
        let now = after_cd1 + Duration::from_secs(1);
        for _ in 0..3 {
            b.record(Outcome::TransientError(429), now);
        }
        let cd1_until = match b.state() {
            BreakerState::OpenTransient { until, attempts } => {
                assert_eq!(attempts, 1, "fresh open after recovery is attempt 1");
                until
            }
            other => panic!("expected OpenTransient after reopen, got {other:?}"),
        };
        let cd1_dur = cd1_until.duration_since(now);

        // Drive into half-open and FAIL the probe.
        let after_cd2_entry = cd1_until + Duration::from_secs(1);
        let _ = b.try_acquire(after_cd2_entry); // → HalfOpen
        b.record(Outcome::TransientError(429), after_cd2_entry); // probe fails
        match b.state() {
            BreakerState::OpenTransient { until, attempts } => {
                let cd2_dur = until.duration_since(after_cd2_entry);
                assert_eq!(attempts, 2, "second open after failed probe is attempt 2");
                assert!(cd2_dur > cd1_dur, "backoff must grow: {cd2_dur:?} > {cd1_dur:?}");
            }
            other => panic!("expected OpenTransient after failed probe, got {other:?}"),
        }
    }

    // Non-402 hard errors (400/401/403/422) reset the transient streak but do
    // NOT open the breaker — they signal caller bugs, not provider faults.
    #[test]
    fn breaker_400_class_does_not_open_breaker() {
        let mut b = fresh_breaker();
        let t0 = Instant::now();
        b.record(Outcome::HardError(400), t0);
        b.record(Outcome::HardError(401), t0);
        b.record(Outcome::HardError(422), t0);
        assert_eq!(b.state(), BreakerState::Closed);
        assert_eq!(b.try_acquire(t0), AcquireDecision::Allow);
    }

    // ── token bucket ────────────────────────────────────────────────────────

    #[test]
    fn token_bucket_starts_full_then_refills_at_rpm_rate() {
        let t0 = Instant::now();
        let mut bucket = TokenBucket::new(60, t0); // 60 RPM = 1 token/sec
        // Drain the full bucket — should be exactly 60 tokens.
        for i in 0..60 {
            assert!(bucket.try_take(t0), "token {} should be available at t0", i);
        }
        // Next take must fail (no time has passed).
        assert!(!bucket.try_take(t0));

        // Advance 1.5 seconds → ~1.5 tokens refilled → one take succeeds, next fails.
        let later = t0 + Duration::from_millis(1500);
        assert!(bucket.try_take(later));
        assert!(!bucket.try_take(later));
    }

    #[test]
    fn token_bucket_next_available_predicts_correct_wait() {
        let t0 = Instant::now();
        let mut bucket = TokenBucket::new(60, t0);
        // Drain.
        for _ in 0..60 {
            assert!(bucket.try_take(t0));
        }
        // At 60 RPM, one token refills per second.
        let next = bucket.next_available(t0);
        let wait = next.duration_since(t0);
        // Allow some float tolerance: 0.9–1.1s window.
        assert!(
            wait >= Duration::from_millis(900) && wait <= Duration::from_millis(1100),
            "next_available wait should be ~1s; got {wait:?}"
        );
    }

    // ── concurrency cap ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn concurrency_cap_holds_then_releases() {
        let cap = ConcurrencyCap::new(2);
        assert_eq!(cap.available(), 2);
        let _p1 = cap.acquire().await;
        let _p2 = cap.acquire().await;
        assert_eq!(cap.available(), 0);
        drop(_p1);
        // Brief yield so the semaphore bookkeeping settles.
        tokio::task::yield_now().await;
        assert_eq!(cap.available(), 1);
    }
}
