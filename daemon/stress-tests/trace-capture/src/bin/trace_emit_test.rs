use std::path::PathBuf;

use anyhow::Result;
use chrono::{Duration as ChronoDuration, Utc};
use clap::Parser;
use rand::seq::SliceRandom;
use rand::Rng;
use serde_json::json;
use trace_capture::{
    JsonlSink, TraceEvent, COST_API_CALL, COST_TOKEN_USAGE, LESSON_CANDIDATE, PEER_REVIEW_DECIDED,
    PEER_REVIEW_REQUESTED, TOOL_CALL_COMPLETED, TOOL_CALL_STARTED, WORKER_COMPLETED,
    WORKER_SPAWNED, WORKER_STATE_CHANGED,
};
use uuid::Uuid;

const AGENTS: [&str; 3] = ["claude", "codex", "gemini"];
const BATCH_SIZE: usize = 50;
const EVENT_WEIGHTS: [(&str, u16); 10] = [
    (TOOL_CALL_COMPLETED, 60),
    (COST_TOKEN_USAGE, 15),
    (WORKER_STATE_CHANGED, 10),
    (TOOL_CALL_STARTED, 5),
    (WORKER_SPAWNED, 5),
    (WORKER_COMPLETED, 5),
    (PEER_REVIEW_DECIDED, 5),
    (PEER_REVIEW_REQUESTED, 2),
    (COST_API_CALL, 2),
    (LESSON_CANDIDATE, 2),
];

#[derive(Parser, Debug)]
#[command(name = "trace-emit-test")]
#[command(about = "Emit synthetic Pantheon trace events to JSONL")]
struct Args {
    #[arg(long)]
    out_dir: PathBuf,

    #[arg(long)]
    count: usize,

    #[arg(long)]
    duration_secs: u64,
}

#[derive(Clone, Copy)]
enum WorkerState {
    Spawned,
    Working,
    Done,
}

impl WorkerState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Spawned => "spawned",
            Self::Working => "working",
            Self::Done => "done",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Spawned => Self::Working,
            Self::Working => Self::Done,
            Self::Done => Self::Spawned,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let sink = JsonlSink::new(args.out_dir)?;

    let mut rng = rand::thread_rng();
    let mut batch = Vec::with_capacity(BATCH_SIZE);
    let mut tool_call_ids: Vec<Uuid> = Vec::new();
    let mut worker_state = WorkerState::Spawned;
    let worker_id = Uuid::now_v7();
    let start = Utc::now();
    let total_span_ms = (args.duration_secs.saturating_mul(1000)) as i64;

    for idx in 0..args.count {
        let event_kind = choose_event_type(&mut rng);
        let mut event = build_event(
            event_kind,
            &mut rng,
            &mut tool_call_ids,
            worker_id,
            &mut worker_state,
        );

        let offset_ms = if args.count <= 1 {
            0
        } else {
            ((idx as i64) * total_span_ms) / ((args.count - 1) as i64)
        };
        event.emitted_at = start + ChronoDuration::milliseconds(offset_ms);

        batch.push(event);
        if batch.len() >= BATCH_SIZE {
            sink.append_many(&batch)?;
            batch.clear();
        }
    }

    if !batch.is_empty() {
        sink.append_many(&batch)?;
    }

    Ok(())
}

fn choose_event_type<R: Rng + ?Sized>(rng: &mut R) -> &'static str {
    let total_weight: u16 = EVENT_WEIGHTS.iter().map(|(_, w)| *w).sum();
    let mut threshold = rng.gen_range(0..total_weight);

    for (event_type, weight) in EVENT_WEIGHTS {
        if threshold < weight {
            return event_type;
        }
        threshold -= weight;
    }

    TOOL_CALL_COMPLETED
}

fn build_event<R: Rng + ?Sized>(
    event_type: &str,
    rng: &mut R,
    tool_call_ids: &mut Vec<Uuid>,
    worker_id: Uuid,
    worker_state: &mut WorkerState,
) -> TraceEvent {
    match event_type {
        TOOL_CALL_COMPLETED => {
            let event = TraceEvent::new(
                TOOL_CALL_COMPLETED,
                subject("tool_call", "completed"),
                None,
                json!({
                    "agent": AGENTS.choose(rng).copied().unwrap_or("codex"),
                    "duration_ms": rng.gen_range(50..=2000),
                    "success": rng.gen_bool(0.95),
                    "tool": format!("tool_{:02}", rng.gen_range(1..=8)),
                }),
            );
            tool_call_ids.push(event.event_id);
            event
        }
        COST_TOKEN_USAGE => TraceEvent::new(
            COST_TOKEN_USAGE,
            subject("cost", "token_usage"),
            tool_call_ids.choose(rng).copied(),
            json!({
                "input_tokens": rng.gen_range(200..=5000),
                "output_tokens": rng.gen_range(50..=800),
                "agent": AGENTS.choose(rng).copied().unwrap_or("codex"),
            }),
        ),
        WORKER_STATE_CHANGED => {
            let from = *worker_state;
            let to = worker_state.next();
            *worker_state = to;
            TraceEvent::new(
                WORKER_STATE_CHANGED,
                subject("worker", "state_changed"),
                Some(worker_id),
                json!({
                    "worker_id": worker_id,
                    "from": from.as_str(),
                    "to": to.as_str(),
                }),
            )
        }
        TOOL_CALL_STARTED => TraceEvent::new(
            TOOL_CALL_STARTED,
            subject("tool_call", "started"),
            None,
            json!({
                "agent": AGENTS.choose(rng).copied().unwrap_or("codex"),
                "tool": format!("tool_{:02}", rng.gen_range(1..=8)),
            }),
        ),
        WORKER_SPAWNED => TraceEvent::new(
            WORKER_SPAWNED,
            subject("worker", "spawned"),
            Some(worker_id),
            json!({
                "worker_id": worker_id,
                "worker_kind": "trace_replay",
            }),
        ),
        WORKER_COMPLETED => TraceEvent::new(
            WORKER_COMPLETED,
            subject("worker", "completed"),
            Some(worker_id),
            json!({
                "worker_id": worker_id,
                "status": "ok",
            }),
        ),
        PEER_REVIEW_DECIDED => TraceEvent::new(
            PEER_REVIEW_DECIDED,
            subject("peer_review", "decided"),
            None,
            json!({
                "decision": if rng.gen_bool(0.8) { "approved" } else { "changes_requested" },
                "reviewer": AGENTS.choose(rng).copied().unwrap_or("codex"),
            }),
        ),
        PEER_REVIEW_REQUESTED => TraceEvent::new(
            PEER_REVIEW_REQUESTED,
            subject("peer_review", "requested"),
            None,
            json!({
                "reviewer": AGENTS.choose(rng).copied().unwrap_or("codex"),
                "request_id": Uuid::now_v7(),
            }),
        ),
        COST_API_CALL => TraceEvent::new(
            COST_API_CALL,
            subject("cost", "api_call"),
            tool_call_ids.choose(rng).copied(),
            json!({
                "provider": "openai",
                "model": "gpt-5.3-codex",
                "usd": rng.gen_range(0.0001_f64..0.025_f64),
            }),
        ),
        LESSON_CANDIDATE => TraceEvent::new(
            LESSON_CANDIDATE,
            subject("lesson", "candidate"),
            None,
            {
                let severity = ["low", "medium", "high"]
                    .choose(rng)
                    .copied()
                    .unwrap_or("low");
                json!({
                    "severity": severity,
                    "title": "Synthetic lesson candidate",
                })
            },
        ),
        _ => TraceEvent::new(
            TOOL_CALL_COMPLETED,
            subject("tool_call", "completed"),
            None,
            json!({ "agent": "codex", "duration_ms": 100, "success": true }),
        ),
    }
}

fn subject(domain: &str, event: &str) -> String {
    format!("pantheon.local.synthetic.{domain}.{event}")
}
