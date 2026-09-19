//! Does our telemetry actually arrive? The only honest way to know is to ask the other end.
//!
//! D-018, measured 2026-09-19 while the PostHog account was over quota:
//!
//! ```text
//! POST https://us.i.posthog.com/i/v0/e/            ->  HTTP 200 {"status":"Ok"}
//! SELECT count() ... WHERE event = 'tv_quota_probe' ->  0
//! ```
//!
//! The ingest endpoint acknowledges events it then discards. From the sending side a delivered
//! event and a dropped one are byte-identical, so no amount of checking the POST's status can
//! answer "did it land". Three days of dead telemetry looked like "nothing happened" because
//! of exactly this.
//!
//! So this module closes the loop from the OTHER side. It emits a sentinel event carrying a
//! nonce, waits out ingestion lag, then reads it back through the query API. Present means the
//! window is trustworthy. Absent means the window is UNTRUSTED, and that is said out loud.
//!
//! That one mechanism is also the answer to three older rows: D-003 wanted a marker for windows
//! where telemetry did not ship, D-005 wanted to tell an idle stream from a broken emitter, and
//! D-002's export failures are only one of the ways a window goes dark.
//!
//! TWO KEYS, AND THEY MUST NOT BE CONFUSED. Ingest uses the project key (`phc_`) in
//! `POSTHOG_API_KEY`. The query API needs a personal key (`phx_`), which this module reads from
//! `POSTHOG_PERSONAL_API_KEY`. The shared env file in `posthog-instrumentation` calls its
//! personal key `POSTHOG_API_KEY`, so sourcing that file into the daemon would overwrite the
//! ingest key with the personal one and break every event. The distinct name is the guard.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// What one round trip established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryVerdict {
    /// The sentinel was read back. Telemetry is arriving.
    Delivered,
    /// The query ran and the sentinel is not there.
    NotDelivered,
    /// The round trip could not reach a verdict (unconfigured, query failed, bad response).
    /// Deliberately distinct from `NotDelivered`: "I could not check" is not "it failed", and
    /// collapsing the two is how an instrument starts reporting its own blindness as a finding.
    Unknown(String),
}

/// The conclusion the rest of the daemon acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    Trusted,
    Untrusted,
    Unknown,
}

impl Trust {
    pub fn as_str(self) -> &'static str {
        match self {
            Trust::Trusted => "trusted",
            Trust::Untrusted => "untrusted",
            Trust::Unknown => "unknown",
        }
    }
}

/// How many consecutive `NotDelivered` verdicts before the window is declared untrusted.
///
/// Ingestion is not instant. A single miss can be lag, and one false alarm teaches the reader
/// to ignore the next true one. Two consecutive misses across two full intervals is lag the
/// system does not have.
pub const MISSES_BEFORE_UNTRUSTED: u32 = 2;

/// Fold one verdict into the running state. Pure, so the transition rules are testable.
///
/// Returns `(consecutive_misses, trust)`.
pub fn next_trust(prev_misses: u32, verdict: &DeliveryVerdict) -> (u32, Trust) {
    match verdict {
        DeliveryVerdict::Delivered => (0, Trust::Trusted),
        DeliveryVerdict::NotDelivered => {
            let misses = prev_misses.saturating_add(1);
            if misses >= MISSES_BEFORE_UNTRUSTED {
                (misses, Trust::Untrusted)
            } else {
                // One miss is not yet evidence. Keep the prior count moving and say "unknown"
                // rather than "trusted": we have stopped being able to vouch for it.
                (misses, Trust::Unknown)
            }
        }
        // Could not check. Do not reset the miss count, which would let a flaky query hide a
        // real outage, and do not add to it, which would blame delivery for the checker.
        DeliveryVerdict::Unknown(_) => (prev_misses, Trust::Unknown),
    }
}

/// Pull the count out of a HogQL query response: `{"results": [[N]], ...}`.
///
/// Shape verified against the live API on 2026-09-19, not assumed. Anything else is `None`,
/// which the caller turns into `Unknown` rather than guessing a number.
pub fn parse_count(body: &serde_json::Value) -> Option<u64> {
    body.get("results")?.get(0)?.get(0)?.as_u64()
}

/// A nonce that is safe to interpolate into HogQL: lowercase hex only.
///
/// Built from the clock and the pid rather than a UUID crate, to add no dependency. It only
/// has to be unique per sentinel, and it is validated below before it goes anywhere near a
/// query string.
fn new_nonce() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}{:x}", std::process::id())
}

fn is_safe_nonce(nonce: &str) -> bool {
    !nonce.is_empty() && nonce.len() <= 64 && nonce.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A snapshot for `/health`.
#[derive(Debug, Clone)]
pub struct DeliverySnapshot {
    pub trust: Trust,
    pub consecutive_misses: u32,
    pub detail: String,
    pub last_check_unix_ms: Option<u128>,
}

impl Default for DeliverySnapshot {
    fn default() -> Self {
        Self {
            trust: Trust::Unknown,
            consecutive_misses: 0,
            detail: "no delivery round trip has run yet".to_string(),
            last_check_unix_ms: None,
        }
    }
}

fn state() -> &'static Mutex<DeliverySnapshot> {
    static S: OnceLock<Mutex<DeliverySnapshot>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(DeliverySnapshot::default()))
}

pub fn delivery_snapshot() -> DeliverySnapshot {
    state().lock().expect("telemetry delivery state poisoned").clone()
}

/// How often the daemon runs a round trip. Default ten minutes.
pub fn sentinel_interval() -> Duration {
    std::env::var("TRIUMVIRATE_TELEMETRY_SENTINEL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(600))
}

/// How long to wait for ingestion before reading the sentinel back. Default ninety seconds.
pub fn ingestion_wait() -> Duration {
    std::env::var("TRIUMVIRATE_TELEMETRY_INGEST_WAIT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(90))
}

struct Config {
    ingest_host: String,
    ingest_key: String,
    query_host: String,
    personal_key: String,
    project_id: String,
}

fn config() -> Result<Config, String> {
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let ingest_host = get("POSTHOG_HOST").ok_or("POSTHOG_HOST unset")?;
    let ingest_key = get("POSTHOG_API_KEY").ok_or("POSTHOG_API_KEY unset")?;
    let personal_key = get("POSTHOG_PERSONAL_API_KEY")
        .ok_or("POSTHOG_PERSONAL_API_KEY unset, so delivery cannot be verified")?;
    let project_id = get("POSTHOG_PROJECT_ID").ok_or("POSTHOG_PROJECT_ID unset")?;
    // Catch the collision described at the top of this file BEFORE it corrupts anything.
    if ingest_key.starts_with("phx_") {
        return Err(
            "POSTHOG_API_KEY holds a personal (phx_) key; ingest needs the project (phc_) key. \
             The personal key belongs in POSTHOG_PERSONAL_API_KEY."
                .to_string(),
        );
    }
    let query_host = get("POSTHOG_QUERY_HOST").unwrap_or_else(|| "https://us.posthog.com".to_string());
    Ok(Config { ingest_host, ingest_key, query_host, personal_key, project_id })
}

/// Send one sentinel, wait, read it back. Never panics and never fails a caller.
pub async fn round_trip(wait: Duration) -> DeliveryVerdict {
    let cfg = match config() {
        Ok(c) => c,
        Err(e) => return DeliveryVerdict::Unknown(e),
    };
    let nonce = new_nonce();
    if !is_safe_nonce(&nonce) {
        return DeliveryVerdict::Unknown(format!("generated nonce is not hex: {nonce}"));
    }
    let client = match reqwest::Client::builder().timeout(Duration::from_secs(20)).build() {
        Ok(c) => c,
        Err(e) => return DeliveryVerdict::Unknown(format!("http client: {e}")),
    };

    let send = serde_json::json!({
        "api_key": cfg.ingest_key,
        "event": "tv_telemetry_sentinel",
        "distinct_id": "triumvirate-daemon",
        "properties": { "tv_nonce": nonce },
    });
    let ingest_url = format!("{}/i/v0/e/", cfg.ingest_host.trim_end_matches('/'));
    match client.post(&ingest_url).json(&send).send().await {
        // A 200 here proves nothing about delivery; that is the whole point of the module. It
        // only proves the request was accepted for processing, so it is logged and moved past.
        Ok(r) if r.status().is_success() => {}
        Ok(r) => return DeliveryVerdict::Unknown(format!("sentinel POST returned {}", r.status())),
        Err(e) => return DeliveryVerdict::Unknown(format!("sentinel POST failed: {e}")),
    }

    tokio::time::sleep(wait).await;

    let query = format!(
        "SELECT count() FROM events WHERE event = 'tv_telemetry_sentinel' \
         AND properties.tv_nonce = '{nonce}' AND timestamp > now() - INTERVAL 1 HOUR"
    );
    let query_url = format!(
        "{}/api/projects/{}/query/",
        cfg.query_host.trim_end_matches('/'),
        cfg.project_id
    );
    let resp = match client
        .post(&query_url)
        .bearer_auth(&cfg.personal_key)
        .json(&serde_json::json!({ "query": { "kind": "HogQLQuery", "query": query } }))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return DeliveryVerdict::Unknown(format!("query failed: {e}")),
    };
    if !resp.status().is_success() {
        return DeliveryVerdict::Unknown(format!("query returned {}", resp.status()));
    }
    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(e) => return DeliveryVerdict::Unknown(format!("query body: {e}")),
    };
    match parse_count(&body) {
        Some(0) => DeliveryVerdict::NotDelivered,
        Some(_) => DeliveryVerdict::Delivered,
        None => DeliveryVerdict::Unknown(format!(
            "unexpected query response shape: {}",
            body.to_string().chars().take(200).collect::<String>()
        )),
    }
}

/// Run one round trip and fold it into the shared state. Loud on the transition that matters.
pub async fn check_and_record(wait: Duration) -> DeliverySnapshot {
    let verdict = round_trip(wait).await;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let (was, snapshot) = {
        let mut s = state().lock().expect("telemetry delivery state poisoned");
        let was = s.trust;
        let (misses, trust) = next_trust(s.consecutive_misses, &verdict);
        s.consecutive_misses = misses;
        s.trust = trust;
        s.last_check_unix_ms = Some(now_ms);
        s.detail = match &verdict {
            DeliveryVerdict::Delivered => "sentinel read back: telemetry is arriving".to_string(),
            DeliveryVerdict::NotDelivered => format!(
                "sentinel accepted by ingest but absent from the data ({misses} consecutive)"
            ),
            DeliveryVerdict::Unknown(why) => format!("could not verify delivery: {why}"),
        };
        (was, s.clone())
    };
    match snapshot.trust {
        // The line D-018 said was missing. Ingest answers 200 OK while discarding, so this
        // warning is the ONLY place the daemon can learn its telemetry is going nowhere.
        Trust::Untrusted => tracing::warn!(
            consecutive_misses = snapshot.consecutive_misses,
            detail = %snapshot.detail,
            "telemetry UNTRUSTED: events are acknowledged by PostHog and not arriving. Every \
             tv_* absence in this window means nothing."
        ),
        Trust::Trusted if was == Trust::Untrusted => {
            tracing::warn!("telemetry trusted again: sentinel round trip succeeded")
        }
        Trust::Trusted => tracing::debug!("telemetry sentinel round trip succeeded"),
        Trust::Unknown => tracing::info!(detail = %snapshot.detail, "telemetry delivery unknown"),
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RED IF: a single miss is declared an outage. Ingestion lag would then raise false alarms,
    /// and one false alarm is enough to train the reader to skip the next true one.
    #[test]
    fn one_miss_is_not_an_outage_and_two_are() {
        let (m1, t1) = next_trust(0, &DeliveryVerdict::NotDelivered);
        assert_eq!((m1, t1), (1, Trust::Unknown));
        let (m2, t2) = next_trust(m1, &DeliveryVerdict::NotDelivered);
        assert_eq!((m2, t2), (2, Trust::Untrusted));
    }

    /// RED IF: a delivery stops resetting the count, so a recovered system stays marked broken.
    #[test]
    fn a_delivery_resets_to_trusted() {
        assert_eq!(next_trust(7, &DeliveryVerdict::Delivered), (0, Trust::Trusted));
    }

    /// RED IF: "could not check" is scored as a delivery failure OR as a success. A flaky query
    /// must neither manufacture an outage nor erase a real one.
    #[test]
    fn an_unknown_verdict_moves_the_count_in_neither_direction() {
        let unknown = DeliveryVerdict::Unknown("query timed out".to_string());
        assert_eq!(next_trust(1, &unknown), (1, Trust::Unknown), "a real miss is not erased");
        assert_eq!(next_trust(0, &unknown), (0, Trust::Unknown), "no miss is invented");
    }

    /// The shape verified against the live API on 2026-09-19.
    #[test]
    fn parse_count_reads_the_real_response_shape_and_nothing_else() {
        let live = serde_json::json!({"columns": ["count()"], "results": [[0]], "error": null});
        assert_eq!(parse_count(&live), Some(0));
        assert_eq!(parse_count(&serde_json::json!({"results": [[3]]})), Some(3));
        assert_eq!(parse_count(&serde_json::json!({"results": []})), None, "no row is not zero");
        assert_eq!(parse_count(&serde_json::json!({"detail": "Unauthorized"})), None);
    }

    /// The nonce is interpolated into HogQL. RED IF: anything but hex can reach the query.
    #[test]
    fn only_a_hex_nonce_may_reach_the_query() {
        assert!(is_safe_nonce(&new_nonce()));
        for bad in ["", "abc' OR 1=1 --", "abc def", "zz", &"a".repeat(65)] {
            assert!(!is_safe_nonce(bad), "must be refused: {bad:?}");
        }
    }
}
