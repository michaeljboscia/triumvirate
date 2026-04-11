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
    let mut active_turn: Option<ActiveTurn> = None;
    let mut timer = time::interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            _ = timer.tick() => {
                if let Some(turn) = &active_turn {
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
                                &mut active_turn,
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
    active_turn: &mut Option<ActiveTurn>,
) -> Result<()> {
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            if args.all {
                print_line_with_timer(active_turn, "[invalid_json] failed to parse websocket message")?;
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
            print_line_with_timer(active_turn, &format!("[{event_type}] {payload_json}"))?;
        }
        return Ok(());
    }

    let payload = value
        .get("payload")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("agent_stream missing payload"))?;

    let event: AgentStreamEvent = serde_json::from_value(payload)
        .context("failed to deserialize AgentStreamEvent payload")?;

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
    };

    if let Some(expected) = args.session.as_deref() {
        if event_session.as_deref() != Some(expected) {
            return Ok(());
        }
    }

    let seq = event.seq();
    if let Some(prev) = *seq_seen {
        if seq != prev.saturating_add(1) {
            print_line_with_timer(active_turn, &format!("[events skipped, resynced at seq {seq}]"))?;
        }
    }
    *seq_seen = Some(seq);

    match &event {
        AgentStreamEvent::TurnStarted { agent, .. } => {
            *active_turn = Some(ActiveTurn {
                agent: agent.clone(),
                started: Instant::now(),
            });
            print_line_with_timer(active_turn, &event.display_text())?;
            if let Some(turn) = active_turn.as_ref() {
                redraw_timer(turn)?;
            }
        }
        AgentStreamEvent::TurnCompleted { agent, .. } => {
            print_line_with_timer(active_turn, &event.display_text())?;
            if active_turn.as_ref().map(|t| t.agent.as_str()) == Some(agent.as_str()) {
                *active_turn = None;
                clear_timer_line()?;
            }
        }
        _ => {
            print_line_with_timer(active_turn, &event.display_text())?;
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

fn print_line_with_timer(active_turn: &Option<ActiveTurn>, line: &str) -> Result<()> {
    if active_turn.is_some() {
        clear_timer_line()?;
    }
    println!("{line}");
    if let Some(turn) = active_turn {
        redraw_timer(turn)?;
    }
    Ok(())
}
