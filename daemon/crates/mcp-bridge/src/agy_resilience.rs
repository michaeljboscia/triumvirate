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

/// The resolved concurrency + RPM ceilings, for reporting at startup. These are read from
/// env with defaults, so the only way to know the ceiling a running daemon is ENFORCING is
/// to ask the process itself. Reading the env var from outside answers a different question.
pub fn agy_limits() -> (usize, f64) {
    (agy_max_concurrent(), agy_max_rpm())
}

/// Cap the breaker cooldown at the ~5-hr Ultra quota-reset window (REQ-101).
const BREAKER_MAX_COOLDOWN: Duration = Duration::from_secs(5 * 60 * 60);

/// Floor for the half-open probe lease.
const HALF_OPEN_LEASE_MIN: Duration = Duration::from_secs(30 * 60);

/// Max time a half-open probe may be in flight before the breaker assumes the probing
/// request was cancelled/dropped (never recorded a result) and lets a new probe take
/// over — prevents a stuck-inflight deadlock. Sized to comfortably EXCEED any real
/// probe (connector timeout x3, incl. the one retry) so a slow-but-alive probe can
/// never trigger a second concurrent probe / stale-result overwrite — only a genuinely
/// abandoned probe (which never completes) ever hits the lease.
fn half_open_lease() -> Duration {
    (crate::agy::agy_connector_timeout() * 3).max(HALF_OPEN_LEASE_MIN)
}

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
    let started = Instant::now();
    let mut throttled = false;
    loop {
        let wait = {
            let mut bucket = rate_bucket().lock().expect("agy rate bucket poisoned");
            bucket.try_take(Instant::now())
        };
        match wait {
            None => {
                // Report only calls we actually DELAYED. Emitting on every call would make
                // the throttle indistinguishable from normal traffic; the whole question is
                // "are we hitting our own ceiling, and how hard?"
                if throttled {
                    let waited = started.elapsed();
                    tracing::warn!(
                        waited_ms = waited.as_millis() as u64,
                        "agy call throttled by local RPM ceiling"
                    );
                    crate::posthog::record_rate_limit_wait(waited.as_millis() as u64, agy_max_rpm());
                }
                return;
            }
            Some(d) => {
                throttled = true;
                tokio::time::sleep(d).await;
            }
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
    /// True while a single half-open probe is in flight; blocks a stampede of
    /// concurrent probes the instant the cooldown elapses (H1).
    half_open_inflight: bool,
    /// When the in-flight half-open probe started, for lease expiry (cancelled probe).
    half_open_since: Option<Instant>,
    /// Calls refused during the current OPEN epoch. Reported once, as an aggregate, rather
    /// than one event per refusal (see posthog::record_breaker_event).
    shed: u64,
}

impl BreakerState {
    fn new() -> Self {
        Self {
            phase: BreakerPhase::Closed,
            consecutive: 0,
            open_until: None,
            open_count: 0,
            half_open_inflight: false,
            half_open_since: None,
            shed: 0,
        }
    }

    /// True if the agy attempt should be skipped (circuit OPEN and cooling). When the
    /// cooldown elapses, transition to half-open and allow exactly ONE probe (returns
    /// false, marking the probe in-flight); all other concurrent callers keep skipping
    /// until that probe resolves (record_success/quota/other) — no stampede (H1).
    fn should_skip(&mut self, now: Instant, lease: Duration) -> bool {
        match self.phase {
            BreakerPhase::Open => match self.open_until {
                Some(until) if now >= until => {
                    self.phase = BreakerPhase::HalfOpen;
                    self.half_open_inflight = true;
                    self.half_open_since = Some(now);
                    false
                }
                _ => true,
            },
            BreakerPhase::HalfOpen => {
                if !self.half_open_inflight {
                    self.half_open_inflight = true;
                    self.half_open_since = Some(now);
                    return false;
                }
                // A probe is in flight. If it has been outstanding past the lease, the
                // probing request was likely cancelled and never recorded a result — let
                // a new probe take over rather than wedge the breaker forever. The lease
                // exceeds any real probe, so this never fires for a slow-but-alive probe.
                match self.half_open_since {
                    Some(since) if now.saturating_duration_since(since) > lease => {
                        self.half_open_since = Some(now);
                        false
                    }
                    _ => true,
                }
            }
            BreakerPhase::Closed => false,
        }
    }

    fn record_success(&mut self) {
        self.phase = BreakerPhase::Closed;
        self.consecutive = 0;
        self.open_until = None;
        self.open_count = 0;
        self.half_open_inflight = false;
        self.half_open_since = None;
        self.shed = 0;
    }

    fn trip(&mut self, now: Instant, base: Duration) {
        self.open_count = self.open_count.saturating_add(1);
        let cooldown = exp_cooldown(base, self.open_count);
        self.phase = BreakerPhase::Open;
        self.open_until = Some(now + cooldown);
        self.consecutive = 0;
        self.half_open_inflight = false;
        self.half_open_since = None;
        // New OPEN epoch: shed is counted per epoch, so it must not carry over.
        self.shed = 0;
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
    let lease = half_open_lease();
    let (skip, phase, shed) = {
        let mut b = breaker().lock().expect("agy breaker poisoned");
        let skip = b.should_skip(Instant::now(), lease);
        if skip {
            b.shed = b.shed.saturating_add(1);
        }
        (skip, b.phase, b.shed)
    };
    if skip {
        tracing::warn!(shed, "agy circuit breaker OPEN — skipping agy attempt, routing around");
        // Emitted HERE, not at the call sites, so every caller (ask path + fleet) counts
        // once and identically. Only the FIRST refusal of each OPEN epoch emits: while open,
        // every call is refused, so per-call events would make event volume track traffic
        // volume for a signal that is one bit. The running total ships with "recovered".
        if shed == 1 {
            crate::posthog::record_breaker_event(
                "opened_shedding",
                "gemini",
                "breaker open — agy calls now routed around",
                None,
            );
        }
    } else if phase == BreakerPhase::HalfOpen {
        // Carry the running shed total on every probe. If the breaker opens on exhausted
        // daily quota and NEVER recovers (or the daemon restarts first), "recovered" never
        // fires and the count would be lost exactly when it matters most: a long outage is
        // when you most need to know how much traffic you could not serve. A probe fires
        // once per cooldown, so a long OPEN period checkpoints itself without a timer loop.
        crate::posthog::record_breaker_event(
            "half_open_probe",
            "gemini",
            "single probe allowed after cooldown",
            Some(shed),
        );
    }
    skip
}

/// Record a successful agy dispatch — closes the circuit.
pub fn agy_breaker_record_success() {
    // Only a RECOVERY is newsworthy: a success while already Closed is the normal case and
    // would drown the signal. Compare phase across the transition, inside the lock.
    let recovered = {
        let mut b = breaker().lock().expect("agy breaker poisoned");
        let was = b.phase;
        let shed = b.shed;
        b.record_success();
        (was != BreakerPhase::Closed).then_some(shed)
    };
    if let Some(shed) = recovered {
        // The shed total lands HERE, closing the epoch: "the breaker was open and it cost
        // us N calls." That is the number worth charting, and it is only knowable now.
        crate::posthog::record_breaker_event(
            "recovered",
            "gemini",
            "probe succeeded, circuit closed",
            Some(shed),
        );
    }
}

/// Record a quota/429 agy failure (REQ-101).
pub fn agy_breaker_record_quota() {
    if let Some(shed) = record_and_report(|b, now| {
        b.record_quota(now, breaker_threshold(), breaker_base_cooldown())
    }) {
        crate::posthog::record_breaker_event(
            "tripped_quota",
            "gemini",
            "repeated agy quota/429",
            Some(shed),
        );
    }
}

/// Record an ambiguous/other agy failure (REQ-103 — biases toward OPEN).
pub fn agy_breaker_record_other_failure() {
    if let Some(shed) = record_and_report(|b, now| {
        b.record_other(now, breaker_threshold(), breaker_base_cooldown())
    }) {
        crate::posthog::record_breaker_event(
            "tripped_other",
            "gemini",
            "repeated ambiguous agy failures",
            Some(shed),
        );
    }
}

/// Apply a failure to the breaker and report whether it TRANSITIONED into Open. Emitting on
/// every failure would conflate "one bad call" with "the circuit just opened"; only the
/// transition changes what the system does.
fn record_and_report(apply: impl FnOnce(&mut BreakerState, Instant)) -> Option<u64> {
    let mut b = breaker().lock().expect("agy breaker poisoned");
    let was_open = b.phase == BreakerPhase::Open;
    // Read shed BEFORE applying: trip() resets it for the new epoch, and the number worth
    // reporting is what the epoch just ENDED shed, not the zero that starts the next one.
    let shed = b.shed;
    apply(&mut b, Instant::now());
    (b.phase == BreakerPhase::Open && !was_open).then_some(shed)
}

// ---------------------------------------------------------------------------
// Health probe state (REQ-056)
// ---------------------------------------------------------------------------

/// How often the daemon runs the agy health probe (default 300s).
pub fn agy_health_probe_interval() -> Duration {
    std::env::var("TRIUMVIRATE_AGY_HEALTH_PROBE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(300))
}

/// Classified outcome of a health probe (REQ-056).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgyProbeOutcome {
    /// Non-empty expected output → healthy.
    Ok,
    /// Exit 0 but empty output — the silent stdout-drop regression that production
    /// traffic cannot distinguish from a legitimate empty answer.
    CaptureDegraded,
    /// Non-zero exit / classified backend error.
    BackendFailed,
}

/// Snapshot of the last agy health probe, surfaced via the daemon `/health` endpoint.
#[derive(Debug, Clone)]
pub struct AgyHealthSnapshot {
    pub capture_health: String,
    pub backend_health: String,
    pub detail: String,
    pub last_probe_unix_ms: Option<u128>,
}

impl Default for AgyHealthSnapshot {
    fn default() -> Self {
        Self {
            capture_health: "unknown".to_string(),
            backend_health: "unknown".to_string(),
            detail: "no probe run yet".to_string(),
            last_probe_unix_ms: None,
        }
    }
}

fn health_state() -> &'static Mutex<AgyHealthSnapshot> {
    static H: OnceLock<Mutex<AgyHealthSnapshot>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(AgyHealthSnapshot::default()))
}

/// Record the result of a health probe (REQ-056). A capture-degraded result leaves
/// `backend_health` ok (the process ran), and vice versa.
pub fn agy_record_health(outcome: AgyProbeOutcome, detail: impl Into<String>, now_unix_ms: u128) {
    let snapshot = {
        let mut h = health_state().lock().expect("agy health poisoned");
        h.detail = detail.into();
        h.last_probe_unix_ms = Some(now_unix_ms);
        match outcome {
            AgyProbeOutcome::Ok => {
                h.capture_health = "ok".to_string();
                h.backend_health = "ok".to_string();
            }
            AgyProbeOutcome::CaptureDegraded => {
                h.capture_health = "degraded".to_string();
                h.backend_health = "ok".to_string();
            }
            AgyProbeOutcome::BackendFailed => {
                h.backend_health = "failed".to_string();
            }
        }
        (h.capture_health.clone(), h.backend_health.clone(), h.detail.clone())
    };
    // Ship it, don't just store it. The lock is released first: capture() spawns onto the
    // runtime, and holding a std Mutex across that is how a telemetry call starts blocking
    // the thing it is supposed to be observing.
    crate::posthog::record_health_probe(&snapshot.0, &snapshot.1, &snapshot.2);
}

/// Read the latest agy health snapshot for the `/health` surface.
pub fn agy_health_snapshot() -> AgyHealthSnapshot {
    health_state().lock().expect("agy health poisoned").clone()
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
        assert!(!s.should_skip(now, HALF_OPEN_LEASE_MIN));
        s.record_quota(now, 3, base);
        s.record_quota(now, 3, base);
        assert!(!s.should_skip(now, HALF_OPEN_LEASE_MIN), "still closed below threshold");
        s.record_quota(now, 3, base);
        assert!(s.should_skip(now, HALF_OPEN_LEASE_MIN), "OPEN at threshold");
    }

    #[test]
    fn breaker_half_opens_after_cooldown_then_closes_on_success() {
        let now = Instant::now();
        let base = Duration::from_secs(120);
        let mut s = BreakerState::new();
        for _ in 0..3 {
            s.record_quota(now, 3, base);
        }
        assert!(s.should_skip(now, HALF_OPEN_LEASE_MIN), "OPEN");
        let later = now + Duration::from_secs(121);
        assert!(!s.should_skip(later, HALF_OPEN_LEASE_MIN), "half-open allows a probe after cooldown");
        s.record_success();
        assert!(!s.should_skip(later, HALF_OPEN_LEASE_MIN), "closed after a successful probe");
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
        assert!(!s.should_skip(t1, HALF_OPEN_LEASE_MIN)); // half-open
        s.record_quota(t1, 3, base); // probe fails → reopen, longer cooldown
        assert!(s.should_skip(t1 + Duration::from_secs(121), HALF_OPEN_LEASE_MIN), "still open with longer cooldown");
    }

    #[test]
    fn half_open_allows_only_one_probe_no_stampede() {
        // H1: when the cooldown elapses, only the FIRST caller probes; concurrent
        // callers keep skipping until the probe resolves — no stampede on agy.
        let now = Instant::now();
        let base = Duration::from_secs(120);
        let mut s = BreakerState::new();
        for _ in 0..3 {
            s.record_quota(now, 3, base);
        }
        let later = now + Duration::from_secs(121);
        assert!(!s.should_skip(later, HALF_OPEN_LEASE_MIN), "first caller after cooldown is the single probe");
        assert!(s.should_skip(later, HALF_OPEN_LEASE_MIN), "second concurrent caller is blocked");
        assert!(s.should_skip(later, HALF_OPEN_LEASE_MIN), "third too");
        s.record_success(); // probe succeeds → closed
        assert!(!s.should_skip(later, HALF_OPEN_LEASE_MIN), "closed after a successful probe");
    }

    #[test]
    fn half_open_lease_lets_a_new_probe_take_over_after_stuck_inflight() {
        // A probe that never records (cancelled request) must not wedge the breaker:
        // after the lease, a new probe is allowed.
        let now = Instant::now();
        let base = Duration::from_secs(120);
        let mut s = BreakerState::new();
        for _ in 0..3 {
            s.record_quota(now, 3, base);
        }
        let later = now + Duration::from_secs(121);
        assert!(!s.should_skip(later, HALF_OPEN_LEASE_MIN), "first probe");
        assert!(s.should_skip(later, HALF_OPEN_LEASE_MIN), "second blocked (probe in flight)");
        let after_lease = later + HALF_OPEN_LEASE_MIN + Duration::from_secs(1);
        assert!(!s.should_skip(after_lease, HALF_OPEN_LEASE_MIN), "new probe allowed once the lease expires");
    }

    #[test]
    fn health_capture_degraded_keeps_backend_ok() {
        // The global health state is only mutated here, so the sequence is deterministic.
        agy_record_health(AgyProbeOutcome::Ok, "probe ok", 100);
        let ok = agy_health_snapshot();
        assert_eq!(ok.capture_health, "ok");
        assert_eq!(ok.backend_health, "ok");

        agy_record_health(AgyProbeOutcome::CaptureDegraded, "empty output", 200);
        let degraded = agy_health_snapshot();
        assert_eq!(degraded.capture_health, "degraded");
        assert_eq!(degraded.backend_health, "ok", "an empty answer is not a backend failure");
        assert_eq!(degraded.last_probe_unix_ms, Some(200));

        agy_record_health(AgyProbeOutcome::BackendFailed, "exit 2", 300);
        assert_eq!(agy_health_snapshot().backend_health, "failed");
    }

    #[test]
    fn shed_counts_per_open_epoch_and_resets_on_recovery() {
        // The shed total is what makes "the breaker was open" chartable as a cost. It must
        // count per OPEN epoch and never carry across epochs, or the number is cumulative
        // nonsense that grows forever.
        let now = Instant::now();
        let base = Duration::from_secs(120);
        let mut s = BreakerState::new();
        for _ in 0..3 {
            s.record_quota(now, 3, base);
        }
        assert_eq!(s.shed, 0, "a fresh OPEN epoch starts at zero");
        for _ in 0..5 {
            assert!(s.should_skip(now, HALF_OPEN_LEASE_MIN));
            s.shed += 1; // mirrors the wrapper's increment
        }
        assert_eq!(s.shed, 5, "every refused call is counted");

        s.record_success();
        assert_eq!(s.shed, 0, "recovery closes the epoch and resets the count");

        // A NEW epoch must not inherit the previous total.
        for _ in 0..3 {
            s.record_quota(now, 3, base);
        }
        assert_eq!(s.shed, 0, "trip() starts a fresh epoch");
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
