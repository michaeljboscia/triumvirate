//! Repro for the intermittent `/session/ask` failure documented at
//! `daemon/docs/bugs/2026-05-25-daemon-session-ask-intermittent-failure.md`.
//!
//! Talks to the running daemon via the `daemon-http` client library — the SAME
//! code path the MCP client subprocess uses. After the fix in this crate, error
//! messages now include the daemon's actual response body + status, so when the
//! flake fires you see WHY instead of a generic `"daemon request failed"`.
//!
//! Usage:
//!   cargo run --release --example repro_gemini_flake -- [agent] [n_asks] [cwd]
//! Defaults:
//!   agent  = gemini
//!   n_asks = 10
//!   cwd    = current working directory
//!
//! The session name is unique per run (timestamp suffix), so we never reuse a
//! poisoned record from a prior session.

use shared_types::{AskSessionRequest, SpawnSessionRequest};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let agent = args.next().unwrap_or_else(|| "gemini".to_string());
    let n_asks: usize = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let cwd = args
        .next()
        .or_else(|| std::env::current_dir().ok().and_then(|p| p.to_str().map(String::from)));

    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let session_name = format!("repro-{agent}-{unix_ms}");

    eprintln!("== repro_gemini_flake ==");
    eprintln!("  agent        = {agent}");
    eprintln!("  n_asks       = {n_asks}");
    eprintln!("  cwd          = {cwd:?}");
    eprintln!("  session_name = {session_name}");
    eprintln!();

    let spawn_req = SpawnSessionRequest {
        agent: agent.clone(),
        name: session_name.clone(),
        cwd: cwd.clone(),
    };
    eprintln!("[spawn] requesting fresh session...");
    match daemon_http::fetch_daemon_session_spawn(&spawn_req).await {
        Ok(msg) => eprintln!("[spawn] ok: {msg}"),
        Err(err) => {
            eprintln!("[spawn] FAILED: {err:#}");
            std::process::exit(2);
        }
    }
    eprintln!();

    let mut ok_count = 0usize;
    let mut fail_count = 0usize;

    for i in 1..=n_asks {
        let prompt = format!(
            "Repro attempt {i}/{n_asks}: reply with exactly the word OK and nothing else."
        );
        let ask_req = AskSessionRequest {
            name: session_name.clone(),
            message: prompt,
            required_sources: Vec::new(),
            require_sight: None,
        };
        let t0 = SystemTime::now();
        let result = daemon_http::fetch_daemon_session_ask(&ask_req).await;
        let elapsed = t0.elapsed().unwrap_or(Duration::ZERO);
        let ts = chrono_now();
        match result {
            Ok(resp) => {
                ok_count += 1;
                let snippet: String = resp.chars().take(80).collect();
                println!(
                    "[{ts}] ask #{i:02}  OK    elapsed={:?}  resp={snippet:?}",
                    elapsed
                );
            }
            Err(err) => {
                fail_count += 1;
                // {:#} renders the anyhow chain — every wrapped cause.
                println!(
                    "[{ts}] ask #{i:02}  FAIL  elapsed={:?}  err={err:#}",
                    elapsed
                );
            }
        }
        // Tiny breath between calls so the failure pattern resembles real usage,
        // not a thundering herd.
        sleep(Duration::from_millis(250)).await;
    }

    eprintln!();
    eprintln!("== summary ==");
    eprintln!("  ok    = {ok_count}");
    eprintln!("  fail  = {fail_count}");
    eprintln!("  total = {n_asks}");
    if fail_count > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn chrono_now() -> String {
    // No chrono dep here; format from SystemTime as best-effort.
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let millis = dur.subsec_millis();
    // HH:MM:SS.mmm in local-ish UTC (good enough for ordering within a run).
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}.{millis:03}Z")
}
