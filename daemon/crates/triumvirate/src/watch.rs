//! `triumvirate watch` — live streaming of agent events via WebSocket.
//!
//! Connects to the daemon's `/ws` endpoint, deserializes `AgentStreamEvent`
//! payloads, and pretty-prints them to stdout with gap detection and
//! in-place elapsed-time updates between TurnStarted/TurnCompleted.

use std::collections::HashMap;
use std::io::{Write, stdout};
use std::time::Instant;

use anyhow::{Context, Result};
use crossterm::{cursor, execute, terminal::{Clear, ClearType}};
use futures::StreamExt;
use tokio::time::{Duration, sleep};
use tokio_tungstenite::tungstenite::Message;
use tracing::instrument;

use shared_types::AgentStreamEvent;

/// Maximum backoff between reconnection attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Tracks in-progress agent turns for elapsed-time display.
struct ActiveTurn {
    agent: String,
    started_at: Instant,
}

/// Entry point for the `triumvirate watch` subcommand.
///
/// Connects to the daemon WebSocket endpoint and streams events to stdout
/// until the user presses Ctrl-C or the process is killed.
#[instrument(skip_all)]
pub async fn run_watch(all: bool, session: Option<String>) -> Result<()> {
    let bind_addr = std::env::var("TRIUMVIRATE_DAEMON_BIND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let ws_url = format!("ws://{bind_addr}/ws");

    let mut backoff = Duration::from_secs(1);

    loop {
        match connect_and_stream(&ws_url, all, session.as_deref()).await {
            Ok(()) => {
                // Clean disconnect (server shutdown). Reset backoff and retry.
                backoff = Duration::from_secs(1);
                println!("connection lost \u{2014} reconnecting...");
            }
            Err(e) => {
                // Could not connect or stream errored out.
                eprintln!("connection lost \u{2014} reconnecting... ({e:#})");
            }
        }

        sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Perform a single WebSocket connection lifecycle: connect, stream events,
/// return when the connection drops.
async fn connect_and_stream(ws_url: &str, all: bool, session: Option<&str>) -> Result<()> {
    eprintln!("connecting to daemon at {ws_url}...");

    let (ws_stream, _response) = tokio_tungstenite::connect_async(ws_url)
        .await
        .context("failed to connect to daemon WebSocket")?;

    eprintln!("connected.");

    let (_write, mut read) = ws_stream.split();

    let mut last_seq: Option<u64> = None;
    // Map from agent name -> ActiveTurn for elapsed-time tracking.
    let mut active_turns: HashMap<String, ActiveTurn> = HashMap::new();
    // Map from agent name -> session_name, learned from TurnStarted events.
    let mut agent_sessions: HashMap<String, String> = HashMap::new();

    while let Some(msg_result) = read.next().await {
        let msg = msg_result.context("WebSocket read error")?;

        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            // Ping/Pong/Binary/Frame — ignore.
            _ => continue,
        };

        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // --all=false: only show agent_stream messages.
        if !all {
            let msg_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if msg_type != "agent_stream" {
                continue;
            }
        }

        // For non-agent_stream messages (when --all), just dump the raw JSON.
        let msg_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if msg_type != "agent_stream" {
            println!("[{msg_type}] {text}");
            continue;
        }

        // Parse the payload as AgentStreamEvent.
        let payload = match value.get("payload") {
            Some(p) => p,
            None => continue,
        };

        let event: AgentStreamEvent = match serde_json::from_value(payload.clone()) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[parse error] {e}: {payload}");
                continue;
            }
        };

        // Track agent -> session_name from TurnStarted events.
        if let AgentStreamEvent::TurnStarted { ref agent, ref session_name, .. } = event {
            agent_sessions.insert(agent.clone(), session_name.clone());
        }

        // --session filter: skip events whose agent isn't in the target session.
        if let Some(target_session) = session {
            let agent_name = extract_agent(&event);
            let event_session = agent_sessions.get(agent_name).map(|s| s.as_str());
            if event_session != Some(target_session) {
                continue;
            }
        }

        // Gap detection.
        let seq = event.seq();
        if let Some(prev) = last_seq {
            if seq > prev + 1 {
                println!("[events skipped, resynced at seq {seq}]");
            }
        }
        last_seq = Some(seq);

        // Elapsed-timer tracking.
        match &event {
            AgentStreamEvent::TurnStarted { agent, .. } => {
                // Clear any prior in-place timer line for this agent.
                clear_active_timer()?;
                active_turns.insert(
                    agent.clone(),
                    ActiveTurn {
                        agent: agent.clone(),
                        started_at: Instant::now(),
                    },
                );
            }
            AgentStreamEvent::TurnCompleted { agent, .. } => {
                active_turns.remove(agent);
            }
            _ => {}
        }

        // Print the event.
        clear_active_timer()?;
        println!("{}", event.display_text());

        // If there are active turns, show the most recent one's elapsed timer.
        show_active_timer(&active_turns)?;
    }

    Ok(())
}

/// Extract the agent name from any event variant.
fn extract_agent(event: &AgentStreamEvent) -> &str {
    match event {
        AgentStreamEvent::TurnStarted { agent, .. }
        | AgentStreamEvent::ToolCall { agent, .. }
        | AgentStreamEvent::FileRead { agent, .. }
        | AgentStreamEvent::ResponseChunk { agent, .. }
        | AgentStreamEvent::TurnCompleted { agent, .. }
        | AgentStreamEvent::Error { agent, .. } => agent,
    }
}

/// Clear the in-place timer line (move to column 0, clear line).
fn clear_active_timer() -> Result<()> {
    let mut out = stdout();
    execute!(
        out,
        cursor::MoveToColumn(0),
        Clear(ClearType::CurrentLine)
    )
    .context("failed to clear terminal line")?;
    Ok(())
}

/// Display an in-place elapsed timer for the most recently started active turn.
fn show_active_timer(active_turns: &HashMap<String, ActiveTurn>) -> Result<()> {
    if let Some(turn) = active_turns.values().last() {
        let elapsed = turn.started_at.elapsed().as_secs();
        let mut out = stdout();
        write!(
            out,
            "\u{2192} {}: generating response ({elapsed}s elapsed)",
            turn.agent
        )?;
        out.flush()?;
    }
    Ok(())
}
