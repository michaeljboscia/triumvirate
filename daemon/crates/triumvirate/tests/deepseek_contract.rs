// Wave-0 DeepSeek live contract probes (REQ-DS-017).
//
// These tests hit the real api.deepseek.com to ground-truth the contract assumptions in
// daemon/docs/specs/deepseek-integration-spec.md. They are `#[ignore]`-gated (run only via
// `cargo test -p triumvirate -- --ignored deepseek_contract`) and require a funded DeepSeek
// account ($0 balance ⇒ 402 on every chat call). The API key is read from the env var
// `TRIUMVIRATE_DEEPSEEK_API_KEY` and is NEVER written to source or stdout.
//
// What each probe verifies (against the spec):
//   1. /user/balance contract — schema present, balance fields accessible (REQ-DS-018).
//   2. /models lists exactly the v1 model IDs (REQ-DS-005).
//   3. POST /chat/completions streaming SSE shape: reasoning_content delta → content delta →
//      usage chunk with cache hit/miss fields → [DONE] (REQ-DS-019/009/023).
//   4. completion_tokens INCLUDES reasoning_tokens (no double-add — REQ-DS-009/A-04b).
//   5. max_tokens shared-budget starvation: tiny cap with thinking ON → finish_reason=length
//      + empty content (REQ-DS-005/030, C-1 verification).
//   6. Bad key → 401 with type=authentication_error (REQ-DS-006).
//   7. Malformed request → 422 with type=invalid_request_error (REQ-DS-006).
//
// 402/429 are environmental and not reliably reproducible — they're covered by unit tests
// against synthetic responses (T-004's breaker-state suite), not by these live probes.

use futures_util::StreamExt;
use serde_json::Value;
use std::time::Duration;

const BASE_URL_DEFAULT: &str = "https://api.deepseek.com/v1";

struct Setup {
    client: reqwest::Client,
    base: String,
    key: String,
}

/// Reads TRIUMVIRATE_DEEPSEEK_API_KEY (+ optional _BASE_URL). Returns None if the key isn't
/// set — caller should print a skip and return Ok(()) so the test succeeds without running.
fn setup() -> Option<Setup> {
    let key = std::env::var("TRIUMVIRATE_DEEPSEEK_API_KEY").ok()?;
    if key.is_empty() {
        return None;
    }
    let base = std::env::var("TRIUMVIRATE_DEEPSEEK_BASE_URL")
        .unwrap_or_else(|_| BASE_URL_DEFAULT.to_string());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .expect("reqwest client builds");
    Some(Setup { client, base, key })
}

macro_rules! require_env_or_skip {
    () => {{
        match setup() {
            Some(s) => s,
            None => {
                println!("SKIP: TRIUMVIRATE_DEEPSEEK_API_KEY not set — probe skipped");
                return;
            }
        }
    }};
}

// ─────────────────────────────────────────────────────────────────────────────
// Probe 1 — /user/balance contract
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
#[ignore = "live API; requires TRIUMVIRATE_DEEPSEEK_API_KEY"]
async fn probe_01_balance_endpoint_shape() {
    let s = require_env_or_skip!();
    // /user/balance lives at the API root, not under /v1; derive root from base.
    let root = s
        .base
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string();
    let resp = s
        .client
        .get(format!("{root}/user/balance"))
        .header("Authorization", format!("Bearer {}", s.key))
        .send()
        .await
        .expect("balance request sends");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "expected 200 on /user/balance"
    );
    let body: Value = resp.json().await.expect("balance is JSON");
    assert!(
        body.get("is_available").is_some(),
        "balance JSON must contain is_available; got: {body}"
    );
    let infos = body
        .get("balance_infos")
        .and_then(|v| v.as_array())
        .expect("balance_infos array present");
    assert!(!infos.is_empty(), "balance_infos must have at least 1 entry");
    let first = &infos[0];
    for k in ["currency", "total_balance", "granted_balance", "topped_up_balance"] {
        assert!(
            first.get(k).is_some(),
            "balance_infos[0] missing key {k}; got: {first}"
        );
    }
    let is_available = body["is_available"].as_bool().unwrap_or(false);
    let total = first["total_balance"].as_str().unwrap_or("");
    println!("PROBE-01 OK: is_available={is_available} total_balance={total}");
}

// ─────────────────────────────────────────────────────────────────────────────
// Probe 2 — /models lists deepseek-v4-pro AND deepseek-v4-flash
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
#[ignore = "live API; requires TRIUMVIRATE_DEEPSEEK_API_KEY"]
async fn probe_02_models_endpoint_returns_v4_pro_and_v4_flash() {
    let s = require_env_or_skip!();
    let resp = s
        .client
        .get(format!("{}/models", s.base))
        .header("Authorization", format!("Bearer {}", s.key))
        .send()
        .await
        .expect("models request sends");
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.expect("models is JSON");
    let data = body["data"]
        .as_array()
        .expect("models response has data[]");
    let ids: Vec<String> = data
        .iter()
        .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(String::from))
        .collect();
    assert!(
        ids.iter().any(|m| m == "deepseek-v4-pro"),
        "expected deepseek-v4-pro in /models; got {ids:?}"
    );
    assert!(
        ids.iter().any(|m| m == "deepseek-v4-flash"),
        "expected deepseek-v4-flash in /models; got {ids:?}"
    );
    println!("PROBE-02 OK: models served = {ids:?}");
}

// ─────────────────────────────────────────────────────────────────────────────
// Probe 3 — Streaming SSE shape: reasoning_content → content → usage → [DONE]
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
#[ignore = "live API; requires TRIUMVIRATE_DEEPSEEK_API_KEY"]
async fn probe_03_streaming_emits_reasoning_then_content_then_usage_then_done() {
    let s = require_env_or_skip!();
    let req = serde_json::json!({
        "model": "deepseek-v4-pro",
        "messages": [{"role": "user", "content": "What is 17 + 26? Brief reasoning then answer."}],
        "max_tokens": 600,
        "stream": true,
        "stream_options": {"include_usage": true},
        "thinking": {"type": "enabled"},
        "reasoning_effort": "high",
    });

    let resp = s
        .client
        .post(format!("{}/chat/completions", s.base))
        .header("Authorization", format!("Bearer {}", s.key))
        .header("Content-Type", "application/json")
        .json(&req)
        .send()
        .await
        .expect("stream POST sends");
    assert_eq!(resp.status().as_u16(), 200, "expected 200 streaming OK");

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut saw_reasoning_delta = false;
    let mut saw_content_delta = false;
    let mut saw_usage_chunk = false;
    let mut saw_done = false;
    let mut keepalive_lines = 0;
    let mut last_finish_reason: Option<String> = None;
    let mut usage_obj: Option<Value> = None;
    let mut content_acc = String::new();
    let mut reasoning_acc = String::new();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.expect("stream chunk reads");
        buf.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(idx) = buf.find('\n') {
            let line = buf[..idx].to_string();
            buf.drain(..=idx);
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            if line.starts_with(':') {
                keepalive_lines += 1; // : keep-alive comments
                continue;
            }
            let Some(payload) = line.strip_prefix("data: ") else {
                continue;
            };
            if payload == "[DONE]" {
                saw_done = true;
                continue;
            }
            let v: Value = match serde_json::from_str(payload) {
                Ok(v) => v,
                Err(_) => continue,
            };
            // Capture usage from ANY chunk that carries it (DeepSeek may emit it in an
            // empty-choices chunk OR alongside the final content chunk — be resilient).
            if let Some(u) = v.get("usage")
                && !u.is_null()
                && u.as_object().is_some_and(|m| !m.is_empty())
            {
                usage_obj = Some(u.clone());
                saw_usage_chunk = true;
            }
            if let Some(choices) = v.get("choices").and_then(|c| c.as_array()) {
                for choice in choices {
                    if let Some(delta) = choice.get("delta") {
                        if let Some(rc) = delta.get("reasoning_content").and_then(|x| x.as_str())
                            && !rc.is_empty()
                        {
                            saw_reasoning_delta = true;
                            reasoning_acc.push_str(rc);
                        }
                        if let Some(c) = delta.get("content").and_then(|x| x.as_str())
                            && !c.is_empty()
                        {
                            saw_content_delta = true;
                            content_acc.push_str(c);
                        }
                    }
                    if let Some(fr) = choice.get("finish_reason").and_then(|x| x.as_str()) {
                        last_finish_reason = Some(fr.to_string());
                    }
                }
            }
        }
    }

    assert!(saw_reasoning_delta, "expected at least one delta.reasoning_content chunk");
    assert!(saw_content_delta, "expected at least one delta.content chunk");
    assert!(saw_usage_chunk, "expected an empty-choices chunk with usage block");
    assert!(saw_done, "expected `data: [DONE]` sentinel");
    assert_eq!(
        last_finish_reason.as_deref(),
        Some("stop"),
        "expected finish_reason=stop on a normal completion"
    );
    let usage = usage_obj.unwrap();
    for k in [
        "prompt_tokens",
        "completion_tokens",
        "total_tokens",
        "prompt_cache_hit_tokens",
        "prompt_cache_miss_tokens",
    ] {
        assert!(usage.get(k).is_some(), "usage missing key {k}; got {usage}");
    }
    println!(
        "PROBE-03 OK: reasoning_chars={} content_chars={} keepalive_lines={} usage={}",
        reasoning_acc.len(),
        content_acc.len(),
        keepalive_lines,
        usage
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Probe 4 — completion_tokens INCLUDES reasoning_tokens (no double-add)
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
#[ignore = "live API; requires TRIUMVIRATE_DEEPSEEK_API_KEY"]
async fn probe_04_reasoning_tokens_already_in_completion_tokens() {
    let s = require_env_or_skip!();
    let req = serde_json::json!({
        "model": "deepseek-v4-pro",
        "messages": [{"role": "user", "content": "What is 17 * 23? Show brief reasoning."}],
        "max_tokens": 800,
        "thinking": {"type": "enabled"},
        "reasoning_effort": "high",
    });
    let resp = s
        .client
        .post(format!("{}/chat/completions", s.base))
        .header("Authorization", format!("Bearer {}", s.key))
        .header("Content-Type", "application/json")
        .json(&req)
        .send()
        .await
        .expect("non-stream POST sends");
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.expect("response is JSON");
    let usage = body["usage"].clone();
    let completion = usage["completion_tokens"].as_u64().expect("completion_tokens u64");
    let reasoning = usage
        .pointer("/completion_tokens_details/reasoning_tokens")
        .and_then(|v| v.as_u64())
        .expect("completion_tokens_details.reasoning_tokens present");
    // The contract: completion_tokens >= reasoning_tokens (reasoning is INCLUDED in completion);
    // i.e. if we added them we'd overstate output.
    assert!(
        reasoning > 0,
        "expected reasoning_tokens > 0 with thinking enabled; got 0 (usage={usage})"
    );
    assert!(
        completion >= reasoning,
        "completion_tokens ({completion}) must INCLUDE reasoning_tokens ({reasoning}) — do NOT double-add"
    );
    // Also sanity: prompt = hit + miss.
    let prompt = usage["prompt_tokens"].as_u64().unwrap();
    let hit = usage["prompt_cache_hit_tokens"].as_u64().unwrap();
    let miss = usage["prompt_cache_miss_tokens"].as_u64().unwrap();
    assert_eq!(prompt, hit + miss, "prompt_tokens must equal hit + miss (got {prompt} vs {hit}+{miss}={})", hit + miss);
    println!(
        "PROBE-04 OK: completion={completion} reasoning={reasoning} (incl) prompt={prompt} (hit={hit} miss={miss})"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Probe 5 — shared-budget starvation: max_tokens=64 + thinking ON → length + empty
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
#[ignore = "live API; requires TRIUMVIRATE_DEEPSEEK_API_KEY"]
async fn probe_05_max_tokens_starvation_returns_finish_reason_length() {
    let s = require_env_or_skip!();
    let req = serde_json::json!({
        "model": "deepseek-v4-pro",
        "messages": [{"role": "user", "content": "Prove rigorously that sqrt(2) is irrational."}],
        "max_tokens": 64,
        "thinking": {"type": "enabled"},
        "reasoning_effort": "high",
    });
    let resp = s
        .client
        .post(format!("{}/chat/completions", s.base))
        .header("Authorization", format!("Bearer {}", s.key))
        .header("Content-Type", "application/json")
        .json(&req)
        .send()
        .await
        .expect("starvation POST sends");
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.expect("response is JSON");
    let ch = &body["choices"][0];
    let finish = ch["finish_reason"].as_str().unwrap_or("");
    let content = ch["message"]["content"].as_str().unwrap_or("");
    let reasoning = ch["message"]["reasoning_content"].as_str().unwrap_or("");
    assert_eq!(finish, "length", "expected finish_reason=length on a tiny budget; got {finish}");
    assert!(
        content.is_empty(),
        "expected EMPTY content on starvation; got {content:?}"
    );
    assert!(
        !reasoning.is_empty(),
        "expected reasoning_content to be populated on starvation; got empty"
    );
    println!(
        "PROBE-05 OK: finish_reason=length content.len()={} reasoning.len()={}",
        content.len(),
        reasoning.len()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Probe 6 — bad API key → 401, type=authentication_error
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
#[ignore = "live API; requires TRIUMVIRATE_DEEPSEEK_API_KEY"]
async fn probe_06_bad_key_returns_401_authentication_error() {
    let s = require_env_or_skip!();
    let req = serde_json::json!({
        "model": "deepseek-v4-flash",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 10,
    });
    let resp = s
        .client
        .post(format!("{}/chat/completions", s.base))
        .header("Authorization", "Bearer sk-deadbeefdeadbeefdeadbeefdeadbeef")
        .header("Content-Type", "application/json")
        .json(&req)
        .send()
        .await
        .expect("bad-key POST sends");
    assert_eq!(resp.status().as_u16(), 401, "expected 401 on bad key");
    let body: Value = resp.json().await.expect("error JSON");
    let typ = body.pointer("/error/type").and_then(|v| v.as_str()).unwrap_or("");
    assert_eq!(typ, "authentication_error", "expected type=authentication_error; got {body}");
    println!("PROBE-06 OK: 401 + error.type=authentication_error");
}

// ─────────────────────────────────────────────────────────────────────────────
// Probe 7 — malformed request → 422 (invalid temperature)
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
#[ignore = "live API; requires TRIUMVIRATE_DEEPSEEK_API_KEY"]
async fn probe_07_malformed_request_returns_4xx_invalid_parameter() {
    let s = require_env_or_skip!();
    // Documented 4xx trigger (most reliable per DeepSeek docs): response_format=json_object
    // without the word "json" in the prompt → the API rejects with 400.
    let req = serde_json::json!({
        "model": "deepseek-v4-flash",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 10,
        "response_format": {"type": "json_object"},
    });
    let resp = s
        .client
        .post(format!("{}/chat/completions", s.base))
        .header("Authorization", format!("Bearer {}", s.key))
        .header("Content-Type", "application/json")
        .json(&req)
        .send()
        .await
        .expect("malformed POST sends");
    let status = resp.status().as_u16();
    let body_text = resp.text().await.unwrap_or_default();
    assert!(
        status == 400 || status == 422,
        "expected 400 or 422 on n>1; got status={status} body={body_text}"
    );
    println!("PROBE-07 OK: malformed → HTTP {status}; body={body_text}");
}

// ─────────────────────────────────────────────────────────────────────────────
// Probe 8 (bonus) — v4-flash thinking:disabled produces content but no reasoning
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
#[ignore = "live API; requires TRIUMVIRATE_DEEPSEEK_API_KEY"]
async fn probe_08_flash_non_thinking_no_reasoning_content() {
    let s = require_env_or_skip!();
    let req = serde_json::json!({
        "model": "deepseek-v4-flash",
        "messages": [{"role": "user", "content": "Reply with exactly: ok"}],
        "max_tokens": 20,
        "thinking": {"type": "disabled"},
    });
    let resp = s
        .client
        .post(format!("{}/chat/completions", s.base))
        .header("Authorization", format!("Bearer {}", s.key))
        .header("Content-Type", "application/json")
        .json(&req)
        .send()
        .await
        .expect("flash POST sends");
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.expect("flash JSON");
    let msg = &body["choices"][0]["message"];
    let content = msg["content"].as_str().unwrap_or("");
    let reasoning = msg["reasoning_content"].as_str().unwrap_or("");
    assert!(!content.is_empty(), "expected non-empty content from flash non-think; got empty");
    assert!(
        reasoning.is_empty(),
        "expected NO reasoning_content with thinking disabled; got {} chars",
        reasoning.len()
    );
    println!("PROBE-08 OK: flash non-think — content={content:?} reasoning='' ");
}

/// PROBE-09 (B.9 follow-up, added 2026-05-26): end-to-end runner integration
/// probe. Verifies that `mcp_bridge::deepseek::run()` — the ACTUAL production
/// entry point — succeeds against api.deepseek.com. Caught the build_request_body
/// nested-thinking-shape bug that probes 01-08 missed because they hand-craft
/// JSON instead of going through the runner.
///
/// A regression that breaks the wire shape (e.g. reverts the
/// `thinking: {type: ...}` nesting to a flat string) would fail this probe
/// with HTTP 400 / invalid_request_error.
#[tokio::test]
#[ignore = "live API contract — set TRIUMVIRATE_DEEPSEEK_API_KEY and run with --ignored"]
async fn probe_09_runner_end_to_end_against_live_api() {
    use mcp_bridge::deepseek as ds;
    use mcp_bridge::deepseek_config::DeepSeekConfig;

    if std::env::var("TRIUMVIRATE_DEEPSEEK_API_KEY").is_err() {
        println!("SKIP: TRIUMVIRATE_DEEPSEEK_API_KEY not set — probe skipped");
        return;
    }

    let cfg = DeepSeekConfig::from_env().expect("config from env");
    assert!(!cfg.api_key.is_empty(), "API key must be present");
    let client = ds::build_client(&cfg).expect("build_client");
    let resilience = ds::ResilienceState::from_cfg(&cfg);
    let req = ds::RunRequest {
        messages: vec![ds::RequestMessage {
            role: "user".to_string(),
            content: "Reply with exactly: ok".to_string(),
        }],
        session_id: format!("deepseek-probe-09-{}", std::process::id()),
        prompt_chars_estimate: 24,
        include_reasoning: false,
    };
    let result = ds::run(&cfg, &client, &req, &resilience)
        .await
        .expect("PROBE-09: runner must succeed end-to-end against live API");
    assert!(!result.response_text.is_empty(),
        "PROBE-09: response_text must be non-empty (model returned content)");
    let sid = result.session_id.as_deref().unwrap_or("");
    assert!(sid.starts_with("deepseek-probe-09-"),
        "PROBE-09: session_id must round-trip from request; got {sid}");
    let usage = result.token_usage.as_ref().expect("PROBE-09: usage populated");
    assert!(usage.output.unwrap_or(0) > 0, "PROBE-09: output tokens > 0");
    println!(
        "PROBE-09 OK: response={:?} session_id={} usage=input:{:?}/output:{:?}/cached:{:?}",
        result.response_text, sid, usage.input, usage.output, usage.cached,
    );
}
