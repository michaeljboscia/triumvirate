//! `triumvirate watch` — live streaming of agent events via WebSocket.
//!
//! Connects to the daemon's `/ws` endpoint, deserializes `AgentStreamEvent`
//! payloads, and pretty-prints them to stdout. Uses `tokio::select!` with a
//! 1-second interval timer for smooth heartbeat updates during long generation.
//!
//! FEAT-004 (REQ-W01 through REQ-W06)
//! Origin: Codex worker T-309, instrument spans from Claude bake-off.

use anyhow::{Context, Result};
use clap::Args;
use crossterm::{
    cursor::MoveToColumn,
    execute,
    terminal::{Clear, ClearType},
};
use futures::StreamExt;
use serde_json::Value;
use shared_types::AgentStreamEvent;
use std::{
    collections::HashMap,
    io::{self, Write},
    time::Duration,
};
use tokio::time::{self, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::instrument;

#[derive(Debug, Clone, Args)]
pub struct WatchArgs {
    /// Show all WebSocket event types, not just agent_stream.
    #[arg(long)]
    pub all: bool,
    /// Filter events to a specific agent session name.
    #[arg(long)]
    pub session: Option<String>,
}

#[derive(Debug)]
struct ActiveTurn {
    agent: String,
    started: Instant,
}

#[instrument(skip_all, fields(all = args.all, session = ?args.session))]
pub async fn run_watch(args: WatchArgs) -> Result<()> {
    let mut backoff = Duration::from_millis(500);
    let max_backoff = Duration::from_secs(10);

    loop {
        let ws_url = websocket_url();
        println!("connecting to daemon...");

        match connect_async(&ws_url).await {
            Ok((stream, _)) => {
                println!("connected: {ws_url}");
                backoff = Duration::from_millis(500);
                if let Err(err) = stream_loop(stream, &args).await {
                    eprintln!("watch disconnected: {err}");
                }
            }
            Err(err) => {
                eprintln!("watch connect failed ({ws_url}): {err}");
            }
        }

        time::sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}

fn websocket_url() -> String {
    let bind = std::env::var("TRIUMVIRATE_DAEMON_BIND_ADDR")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());

    if bind.starts_with("ws://") || bind.starts_with("wss://") {
        if bind.ends_with("/ws") {
            bind
        } else {
            format!("{}/ws", bind.trim_end_matches('/'))
        }
    } else if bind.starts_with("http://") {
        let raw = bind.trim_start_matches("http://");
        format!("ws://{}/ws", raw.trim_end_matches('/'))
    } else if bind.starts_with("https://") {
        let raw = bind.trim_start_matches("https://");
        format!("wss://{}/ws", raw.trim_end_matches('/'))
    } else {
        format!("ws://{bind}/ws")
    }
}

async fn stream_loop(
    stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    args: &WatchArgs,
) -> Result<()> {
    let (_write, mut read) = stream.split();
    let mut seq_seen: Option<u64> = None;
    let mut session_by_agent: HashMap<String, String> = HashMap::new();
    let mut active_turns: HashMap<String, ActiveTurn> = HashMap::new();
    let mut timer = time::interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            _ = timer.tick() => {
                // Show timer for the most recently started turn across all agents.
                if let Some(turn) = most_recent_turn(&active_turns) {
                    redraw_timer(turn)?;
                }
            }
            msg = read.next() => {
                match msg {
                    Some(Ok(message)) => {
                        if let Some(text) = parse_message_text(message) {
                            handle_ws_text(
                                &text,
                                args,
                                &mut session_by_agent,
                                &mut seq_seen,
                                &mut active_turns,
                            )?;
                        }
                    }
                    Some(Err(err)) => {
                        clear_timer_line()?;
                        return Err(anyhow::anyhow!("websocket read error: {err}"));
                    }
                    None => {
                        clear_timer_line()?;
                        return Err(anyhow::anyhow!("websocket closed by daemon"));
                    }
                }
            }
        }
    }
}

fn parse_message_text(message: Message) -> Option<String> {
    match message {
        Message::Text(text) => Some(text.to_string()),
        Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).ok(),
        Message::Ping(_) | Message::Pong(_) | Message::Close(_) => None,
        _ => None,
    }
}

fn handle_ws_text(
    text: &str,
    args: &WatchArgs,
    session_by_agent: &mut HashMap<String, String>,
    seq_seen: &mut Option<u64>,
    active_turns: &mut HashMap<String, ActiveTurn>,
) -> Result<()> {
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            if args.all {
                print_line_with_timer(active_turns, "[invalid_json] failed to parse websocket message")?;
            }
            return Ok(());
        }
    };

    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    if event_type != "agent_stream" {
        if args.all {
            let payload = value.get("payload").cloned().unwrap_or(Value::Null);
            let payload_json = serde_json::to_string(&payload)
                .unwrap_or_else(|_| "<unserializable payload>".to_string());
            print_line_with_timer(active_turns, &format!("[{event_type}] {payload_json}"))?;
        }
        return Ok(());
    }

    let payload = value
        .get("payload")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("agent_stream missing payload"))?;

    let event: AgentStreamEvent = serde_json::from_value(payload)
        .context("failed to deserialize AgentStreamEvent payload")?;

    // BUG-1 FIX: Track sequence number for ALL events BEFORE session filtering.
    // Without this, filtered events leave gaps in seq_seen causing false
    // "[events skipped]" spam when the next matching event arrives.
    let seq = event.seq();
    if let Some(prev) = *seq_seen {
        if seq != prev.saturating_add(1) {
            print_line_with_timer(active_turns, &format!("[events skipped, resynced at seq {seq}]"))?;
        }
    }
    *seq_seen = Some(seq);

    // BUG-3 NOTE: session_by_agent uses agent name as key, so concurrent
    // sessions from the same agent (e.g. two gemini sessions) overwrite each
    // other. The real fix requires session_name on all event variants, which
    // is a spec change deferred past v3.3.0.
    let event_session = match &event {
        AgentStreamEvent::TurnStarted { agent, session_name, .. } => {
            session_by_agent.insert(agent.clone(), session_name.clone());
            Some(session_name.clone())
        }
        AgentStreamEvent::ToolCall { agent, .. }
        | AgentStreamEvent::FileRead { agent, .. }
        | AgentStreamEvent::ResponseChunk { agent, .. }
        | AgentStreamEvent::TurnCompleted { agent, .. }
        | AgentStreamEvent::Error { agent, .. } => session_by_agent.get(agent).cloned(),
        // FEAT-014 (REQ-010): WorkerLifecycle events use session_name directly,
        // not session_by_agent lookup. These events are emitted by the daemon
        // with full lineage context, so session_name is authoritative.
        AgentStreamEvent::WorkerLifecycle { session_name, .. } => Some(session_name.clone()),
    };

    // BUG-2 FIX: Only filter when we positively know the session doesn't match.
    // If session_by_agent hasn't been populated yet (watch connected after
    // TurnStarted), event_session is None — show the event rather than
    // silently dropping it.
    if let Some(expected) = args.session.as_deref() {
        match event_session.as_deref() {
            Some(actual) if actual != expected => return Ok(()),
            _ => {} // None (unknown) or matching — show it
        }
    }

    match &event {
        AgentStreamEvent::TurnStarted { agent, .. } => {
            // BUG-4 FIX: Track per-agent active turns instead of a single global.
            active_turns.insert(
                agent.clone(),
                ActiveTurn {
                    agent: agent.clone(),
                    started: Instant::now(),
                },
            );
            print_line_with_timer(active_turns, &event.display_text())?;
            if let Some(turn) = active_turns.get(agent) {
                redraw_timer(turn)?;
            }
        }
        AgentStreamEvent::TurnCompleted { agent, .. } => {
            print_line_with_timer(active_turns, &event.display_text())?;
            // BUG-4 FIX: Remove only this agent's turn.
            active_turns.remove(agent.as_str());
            // BUG-5 FIX: Clean up session_by_agent on turn completion to
            // prevent unbounded memory growth. The map reflects only the
            // current active session per agent.
            session_by_agent.remove(agent.as_str());
            if active_turns.is_empty() {
                clear_timer_line()?;
            }
        }
        _ => {
            print_line_with_timer(active_turns, &event.display_text())?;
        }
    }

    Ok(())
}

fn redraw_timer(turn: &ActiveTurn) -> Result<()> {
    let elapsed = turn.started.elapsed().as_secs();
    let mut stdout = io::stdout();
    execute!(stdout, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
    write!(
        stdout,
        "→ {}: generating response ({}s elapsed)",
        turn.agent, elapsed
    )?;
    stdout.flush()?;
    Ok(())
}

fn clear_timer_line() -> Result<()> {
    let mut stdout = io::stdout();
    execute!(stdout, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
    stdout.flush()?;
    Ok(())
}

/// Return the most recently started turn across all agents (for timer display).
fn most_recent_turn(active_turns: &HashMap<String, ActiveTurn>) -> Option<&ActiveTurn> {
    // "Most recently started" = smallest elapsed = largest `started` Instant.
    active_turns.values().max_by_key(|t| t.started)
}

fn print_line_with_timer(active_turns: &HashMap<String, ActiveTurn>, line: &str) -> Result<()> {
    if !active_turns.is_empty() {
        clear_timer_line()?;
    }
    println!("{line}");
    if let Some(turn) = most_recent_turn(active_turns) {
        redraw_timer(turn)?;
    }
    Ok(())
}
