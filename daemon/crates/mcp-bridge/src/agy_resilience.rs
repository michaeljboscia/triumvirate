//! Provider-level resilience for the agy (Antigravity CLI) backend, shared across
//! the inter-agent ask path and fleet so limits are GLOBAL, not per-call-site.
//!
//! - **Concurrency cap (REQ-055):** a process-global semaphore bounds simultaneous
//!   agy children. agy shares one subscription quota pool, so unbounded fan-out
//!   wastes quota; verified concurrency-safe at the default of 3.
//! - **Rate limit (REQ-102):** a token-bucket RPM ceiling throttles Triumvirate's
//!   own agy call rate to avoid self-inflicted 429s.
//! - **Circuit breaker (REQ-101):** on repeated quota/429, OPEN the circuit so the
//!   caller routes gemini-sibling work to codex (a different quota pool); a half-open
//!   probe is allowed after an exponential-backoff cooldown capped at the ~5-hr Ultra
//!   reset window. Per REQ-103, ambiguous repeated failures also bias toward OPEN.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::{Semaphore, SemaphorePermit};

// ---------------------------------------------------------------------------
// Env knobs
// ---------------------------------------------------------------------------

fn agy_max_concurrent() -> usize {
    std::env::var("TRIUMVIRATE_AGY_MAX_CONCURRENT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(3)
}

fn agy_max_rpm() -> f64 {
    std::env::var("TRIUMVIRATE_AGY_MAX_RPM")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|&n| n > 0.0)
        .unwrap_or(30.0)
}

fn breaker_threshold() -> u32 {
    std::env::var("TRIUMVIRATE_AGY_BREAKER_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(3)
}

fn breaker_base_cooldown() -> Duration {
    std::env::var("TRIUMVIRATE_AGY_BREAKER_COOLDOWN_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(120))
}

/// Cap the breaker cooldown at the ~5-hr Ultra quota-reset window (REQ-101).
const BREAKER_MAX_COOLDOWN: Duration = Duration::from_secs(5 * 60 * 60);

// ---------------------------------------------------------------------------
// Concurrency cap (REQ-055)
// ---------------------------------------------------------------------------

fn agy_semaphore() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| Semaphore::new(agy_max_concurrent()))
}

/// Acquire one agy concurrency slot, held for the lifetime of the returned permit.
/// Bounds simultaneous agy children across every caller (ask path + fleet).
pub async fn agy_acquire_slot() -> SemaphorePermit<'static> {
    agy_semaphore()
        .acquire()
        .await
        .expect("agy semaphore is never closed")
}

// ---------------------------------------------------------------------------
// Rate limit — token bucket (REQ-102)
// ---------------------------------------------------------------------------

struct TokenBucket {
    tokens: f64,
    capacity: f64,
    refill_per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    /// Refill based on elapsed time, then try to take one token. Returns `None` if a
    /// token was taken, or `Some(wait)` for how long until one is available.
    fn try_take(&mut self, now: Instant) -> Option<Duration> {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            None
        } else {
            let needed = 1.0 - self.tokens;
            Some(Duration::from_secs_f64(needed / self.refill_per_sec))
        }
    }
}

fn rate_bucket() -> &'static Mutex<TokenBucket> {
    static B: OnceLock<Mutex<TokenBucket>> = OnceLock::new();
    B.get_or_init(|| {
        let rpm = agy_max_rpm();
        // Burst = the concurrency cap, refilling at rpm/60 per second.
        let capacity = (agy_max_concurrent() as f64).max(1.0);
        Mutex::new(TokenBucket {
            tokens: capacity,
            capacity,
            refill_per_sec: rpm / 60.0,
            last: Instant::now(),
        })
    })
}

/// Block until the rate limiter grants one agy call (REQ-102). The mutex is never held
/// across the await — wait time is computed under lock, then released before sleeping.
pub async fn agy_rate_limit() {
    loop {
        let wait = {
            let mut bucket = rate_bucket().lock().expect("agy rate bucket poisoned");
            bucket.try_take(Instant::now())
        };
        match wait {
            None => return,
            Some(d) => tokio::time::sleep(d).await,
        }
    }
}

// ---------------------------------------------------------------------------
// Circuit breaker (REQ-101 / 103)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakerPhase {
    Closed,
    Open,
    HalfOpen,
}

/// Breaker state machine. Kept as a plain struct (no statics) so it is unit-testable
/// with a controlled clock; the process-global wrapper below delegates to it.
struct BreakerState {
    phase: BreakerPhase,
    consecutive: u32,
    open_until: Option<Instant>,
    open_count: u32,
}

impl BreakerState {
    fn new() -> Self {
        Self {
            phase: BreakerPhase::Closed,
            consecutive: 0,
            open_until: None,
            open_count: 0,
        }
    }

    /// True if the agy attempt should be skipped (circuit OPEN and cooling). When the
    /// cooldown has elapsed, transition to half-open and allow one probe (returns false).
    fn should_skip(&mut self, now: Instant) -> bool {
        match self.phase {
            BreakerPhase::Open => match self.open_until {
                Some(until) if now >= until => {
                    self.phase = BreakerPhase::HalfOpen;
                    false
                }
                Some(_) => true,
                None => true,
            },
            _ => false,
        }
    }

    fn record_success(&mut self) {
        self.phase = BreakerPhase::Closed;
        self.consecutive = 0;
        self.open_until = None;
        self.open_count = 0;
    }

    fn trip(&mut self, now: Instant, base: Duration) {
        self.open_count = self.open_count.saturating_add(1);
        let cooldown = exp_cooldown(base, self.open_count);
        self.phase = BreakerPhase::Open;
        self.open_until = Some(now + cooldown);
        self.consecutive = 0;
    }

    /// Quota/429 failure: trip immediately on a failed half-open probe, else trip at
    /// the threshold.
    fn record_quota(&mut self, now: Instant, threshold: u32, base: Duration) {
        if self.phase == BreakerPhase::HalfOpen {
            self.trip(now, base);
            return;
        }
        self.consecutive = self.consecutive.saturating_add(1);
        if self.consecutive >= threshold {
            self.trip(now, base);
        }
    }

    /// Ambiguous/other failure: a failed half-open probe re-opens; otherwise bias
    /// toward OPEN with a slightly higher bar than quota (REQ-103).
    fn record_other(&mut self, now: Instant, threshold: u32, base: Duration) {
        if self.phase == BreakerPhase::HalfOpen {
            self.trip(now, base);
            return;
        }
        self.consecutive = self.consecutive.saturating_add(1);
        if self.consecutive >= threshold.saturating_add(1) {
            self.trip(now, base);
        }
    }
}

/// Exponential cooldown from a base, doubling per consecutive open, capped at the
/// Ultra reset window.
fn exp_cooldown(base: Duration, open_count: u32) -> Duration {
    let shift = open_count.saturating_sub(1).min(12);
    let mult = 1u64 << shift;
    let secs = base.as_secs().saturating_mul(mult);
    Duration::from_secs(secs).min(BREAKER_MAX_COOLDOWN)
}

fn breaker() -> &'static Mutex<BreakerState> {
    static B: OnceLock<Mutex<BreakerState>> = OnceLock::new();
    B.get_or_init(|| Mutex::new(BreakerState::new()))
}

/// True if the agy attempt should be short-circuited (caller routes straight to the
/// degraded route / codex). Transitions OPEN→half-open when the cooldown elapses.
pub fn agy_breaker_should_skip() -> bool {
    let skip = breaker()
        .lock()
        .expect("agy breaker poisoned")
        .should_skip(Instant::now());
    if skip {
        tracing::warn!("agy circuit breaker OPEN — skipping agy attempt, routing around");
    }
    skip
}

/// Record a successful agy dispatch — closes the circuit.
pub fn agy_breaker_record_success() {
    breaker()
        .lock()
        .expect("agy breaker poisoned")
        .record_success();
}

/// Record a quota/429 agy failure (REQ-101).
pub fn agy_breaker_record_quota() {
    breaker().lock().expect("agy breaker poisoned").record_quota(
        Instant::now(),
        breaker_threshold(),
        breaker_base_cooldown(),
    );
}

/// Record an ambiguous/other agy failure (REQ-103 — biases toward OPEN).
pub fn agy_breaker_record_other_failure() {
    breaker().lock().expect("agy breaker poisoned").record_other(
        Instant::now(),
        breaker_threshold(),
        breaker_base_cooldown(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket_grants_then_throttles() {
        let mut b = TokenBucket {
            tokens: 1.0,
            capacity: 1.0,
            refill_per_sec: 1.0,
            last: Instant::now(),
        };
        let now = b.last;
        assert!(b.try_take(now).is_none(), "first token granted");
        let wait = b.try_take(now).expect("second token throttled");
        assert!(wait > Duration::ZERO && wait <= Duration::from_secs(1));
    }

    #[test]
    fn breaker_trips_at_threshold_and_routes_around() {
        let now = Instant::now();
        let base = Duration::from_secs(120);
        let mut s = BreakerState::new();
        assert!(!s.should_skip(now));
        s.record_quota(now, 3, base);
        s.record_quota(now, 3, base);
        assert!(!s.should_skip(now), "still closed below threshold");
        s.record_quota(now, 3, base);
        assert!(s.should_skip(now), "OPEN at threshold");
    }

    #[test]
    fn breaker_half_opens_after_cooldown_then_closes_on_success() {
        let now = Instant::now();
        let base = Duration::from_secs(120);
        let mut s = BreakerState::new();
        for _ in 0..3 {
            s.record_quota(now, 3, base);
        }
        assert!(s.should_skip(now), "OPEN");
        let later = now + Duration::from_secs(121);
        assert!(!s.should_skip(later), "half-open allows a probe after cooldown");
        s.record_success();
        assert!(!s.should_skip(later), "closed after a successful probe");
    }

    #[test]
    fn breaker_failed_probe_reopens_with_longer_cooldown() {
        let now = Instant::now();
        let base = Duration::from_secs(120);
        let mut s = BreakerState::new();
        for _ in 0..3 {
            s.record_quota(now, 3, base);
        }
        let t1 = now + Duration::from_secs(121);
        assert!(!s.should_skip(t1)); // half-open
        s.record_quota(t1, 3, base); // probe fails → reopen, longer cooldown
        assert!(s.should_skip(t1 + Duration::from_secs(121)), "still open with longer cooldown");
    }

    #[test]
    fn exp_cooldown_doubles_and_caps() {
        let base = Duration::from_secs(120);
        assert_eq!(exp_cooldown(base, 1), Duration::from_secs(120));
        assert_eq!(exp_cooldown(base, 2), Duration::from_secs(240));
        assert_eq!(exp_cooldown(base, 3), Duration::from_secs(480));
        assert_eq!(exp_cooldown(base, 99), BREAKER_MAX_COOLDOWN, "capped at Ultra window");
    }
}
