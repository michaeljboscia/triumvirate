//! T-005 (REQ-DS-007): DeepSeek HTTP client builder.
//!
//! Constructs a `reqwest::Client` configured with the timeouts and TCP keep-alive
//! from `DeepSeekConfig`. The DEEPSEEK API streams SSE chunks; the client's
//! `read_timeout` is the per-chunk rolling idle limit (each byte that arrives
//! resets the timer), while `timeout` is the absolute outer ceiling. The runner
//! (T-010) layers `tokio::time::timeout` on top of `timeout` for an extra-firm
//! request-level cap.
//!
//! This module owns ONLY the client-builder concern. It MUST NOT:
//!   - construct request bodies (T-009/T-010 territory)
//!   - parse SSE (T-006)
//!   - read or pass the API key (the runner injects `Authorization` per request)
//!
//! Keeping it builder-only means tests don't need a real DeepSeek endpoint —
//! they only need to confirm reqwest honours the rolling read_timeout, which is
//! a property of the reqwest configuration, not the wire content.

use crate::deepseek_config::DeepSeekConfig;

/// Build a `reqwest::Client` from the loaded DeepSeek config.
///
/// Returns `reqwest::Error` if reqwest's builder rejects the configuration
/// (e.g. a TLS backend failure). In practice this is infallible with our
/// default-features = false + rustls-tls workspace setup.
pub fn build_client(cfg: &DeepSeekConfig) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        // Rolling per-read idle timeout. Each byte resets it; an SSE stream that
        // dribbles chunks within `read_timeout` of each other will complete even
        // if the total wall-clock exceeds it. This is the property the spec
        // (REQ-DS-007) is leaning on for long thinking-mode responses.
        .read_timeout(cfg.read_timeout)
        // Absolute outer ceiling on a single HTTP request.
        .timeout(cfg.timeout)
        // TCP keep-alive probes detect dead peers between requests.
        .tcp_keepalive(cfg.tcp_keepalive)
        .build()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────
//
// Reality test (REQ-DS-007): a stub that uses only the absolute `timeout` would
// fail the rolling-read-timeout property. We prove the property by spinning up
// a raw TCP server that:
//   1. writes the HTTP/1.1 response head + Content-Length: 15
//   2. then writes 15 bytes of body, one every ~200ms (total ~3s)
//
// Client config: read_timeout = 500ms (shorter than the total wall-clock so a
// non-rolling timeout would fail; longer than the inter-byte gap so the
// rolling one succeeds).
//
// No hyper dependency — `tokio::net::TcpListener` + hand-written HTTP/1.1 is
// enough because we control both sides.

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    /// Spawns a one-shot HTTP/1.1 server that responds to ONE request by
    /// dribbling `body_byte_count` bytes spaced `inter_byte_delay` apart.
    /// Returns the bound `127.0.0.1:PORT` URL string. The handler task exits
    /// after serving the single connection.
    async fn spawn_dribbler(
        body_byte_count: usize,
        inter_byte_delay: Duration,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let url = format!("http://{}", addr);

        tokio::spawn(async move {
            let (mut sock, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => return,
            };

            // We don't bother parsing the request — read until \r\n\r\n then write.
            // Reading isn't strictly necessary for the dribble; the test calls .send()
            // which writes the request, but reqwest may not block on response start
            // until it has flushed. To keep things simple we just discard whatever
            // shows up and proceed to write.
            let mut buf = [0u8; 4096];
            // Best-effort: peek once, ignore result.
            let _ = tokio::time::timeout(
                Duration::from_millis(100),
                tokio::io::AsyncReadExt::read(&mut sock, &mut buf),
            )
            .await;

            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
                body_byte_count
            );
            if sock.write_all(head.as_bytes()).await.is_err() {
                return;
            }

            for i in 0..body_byte_count {
                // One byte. Use ASCII printable so any debugging dump is readable.
                let byte = [b'a' + (i as u8 % 26)];
                if sock.write_all(&byte).await.is_err() {
                    return;
                }
                if sock.flush().await.is_err() {
                    return;
                }
                tokio::time::sleep(inter_byte_delay).await;
            }
            let _ = sock.shutdown().await;
        });

        url
    }

    /// Build a config with the timing knobs we care about; everything else
    /// at its `from_env` default-equivalent. We don't go through env loading
    /// here because we want to vary timings independently.
    fn cfg_with_timings(
        read_timeout: Duration,
        absolute_timeout: Duration,
    ) -> DeepSeekConfig {
        // Construct with literal defaults (the public surface). The point of
        // this test is the reqwest config plumbing, not env parsing.
        DeepSeekConfig {
            base_url: "http://localhost".to_string(),
            api_key: crate::deepseek_config::ApiKey::new("unused-for-this-test"),
            model: "deepseek-v4-pro".to_string(),
            max_tokens: 32768,
            thinking: crate::deepseek_config::ThinkingMode::Enabled,
            reasoning_effort: crate::deepseek_config::ReasoningEffort::High,
            read_timeout,
            timeout: absolute_timeout,
            tcp_keepalive: Duration::from_secs(30),
            max_concurrent: 8,
            max_rpm: 60,
            reasoning_cap_tokens: 0,
            log_dir: std::path::PathBuf::from("/tmp/deepseek-logs-test"),
            log_reasoning_cap_bytes: 262_144,
            bulk_bytes: 16_384,
        }
    }

    // REQ-DS-007 reality check: a rolling read_timeout allows total wall-clock
    // to exceed `read_timeout` as long as each individual gap is shorter.
    //
    // Stub guard: a client built with only an absolute `timeout` ≈ 500ms would
    // be cut off at 500ms — well before the 15 bytes finish at ~3s total.
    #[tokio::test]
    async fn deepseek_client_timeouts_rolling_read_succeeds_even_when_total_exceeds_read_timeout() {
        // 15 bytes × 200ms each → ~3s total. read_timeout=500ms, absolute=10s.
        // Inter-byte gap (200ms) < read_timeout (500ms) → rolling timer never trips.
        // Total wall-clock (3s) > read_timeout (500ms) → non-rolling stub would fail.
        let url = spawn_dribbler(15, Duration::from_millis(200)).await;

        let cfg = cfg_with_timings(Duration::from_millis(500), Duration::from_secs(10));
        let client = build_client(&cfg).expect("build_client");

        let started = Instant::now();
        let resp = client
            .get(&url)
            .send()
            .await
            .expect("request should complete despite total > read_timeout");
        assert!(resp.status().is_success(), "HTTP status: {}", resp.status());
        let body = resp.bytes().await.expect("body").to_vec();
        let elapsed = started.elapsed();

        assert_eq!(body.len(), 15, "should receive all 15 dribbled bytes");
        assert!(
            elapsed >= Duration::from_millis(15 * 200 - 200),
            "elapsed should reflect dribble timing (got {elapsed:?})"
        );
        assert!(
            elapsed > Duration::from_millis(500),
            "elapsed must exceed read_timeout to prove the rolling property (got {elapsed:?})"
        );
    }

    // Stub guard: confirms read_timeout DOES bite when an inter-byte gap exceeds it.
    // If reqwest silently ignored read_timeout, this test would hang or pass with
    // the full body — both outcomes are caught.
    #[tokio::test]
    async fn deepseek_client_timeouts_rolling_read_times_out_when_gap_exceeds_read_timeout() {
        // 5 bytes × 800ms gap → first byte arrives ~800ms after request, by which
        // point the 200ms read_timeout has long since fired.
        let url = spawn_dribbler(5, Duration::from_millis(800)).await;

        let cfg = cfg_with_timings(Duration::from_millis(200), Duration::from_secs(10));
        let client = build_client(&cfg).expect("build_client");

        let result = client.get(&url).send().await;
        // reqwest returns Ok(Response) on headers received, then errors on body.
        // We must consume the body to actually exercise the read_timeout.
        let body_result = match result {
            Ok(resp) => resp.bytes().await,
            Err(e) => return assert!(e.is_timeout() || e.is_connect() || e.is_request(),
                "expected timeout-class error on send(), got: {e}"),
        };
        let err = body_result.expect_err(
            "body read should fail because inter-byte gap (800ms) >> read_timeout (200ms)",
        );
        assert!(
            err.is_timeout() || err.is_body(),
            "expected timeout/body error from rolling read_timeout; got: {err}"
        );
    }

    // build_client itself is infallible with the default workspace TLS feature
    // set — confirm we don't accidentally introduce a builder that fails on
    // ordinary configs. (A future change that adds a required builder field
    // without a default would trip this.)
    #[test]
    fn build_client_succeeds_with_defaultish_config() {
        let cfg = cfg_with_timings(Duration::from_secs(60), Duration::from_secs(1800));
        build_client(&cfg).expect("build_client should not fail on default-shaped config");
    }
}
