mod metrics;
mod report;
mod workload;
mod writer;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use tokio::task::JoinSet;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::metrics::MetricsCollector;
use crate::report::{write_reports, RunReport};
use crate::workload::{
    assign_trace_events, load_trace_events, TraceEventIn, WorkerPacer, WorkloadProfile,
    WorkloadProfileName,
};
use crate::writer::{execute_peer_review_transaction, read_once, write_replay_once};

#[derive(Debug, Parser)]
#[command(name = "sqlite-concurrency")]
struct Args {
    #[arg(long)]
    workers: usize,

    #[arg(long, value_enum)]
    profile: WorkloadProfileName,

    #[arg(long)]
    duration: u64,

    #[arg(long, default_value_t = 0)]
    tx_hold_ms: u64,

    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=100))]
    read_pct: u8,

    #[arg(long)]
    db_path: Option<PathBuf>,

    #[arg(long)]
    run_id: Option<String>,

    #[arg(long)]
    trace_file: Option<PathBuf>,

    #[arg(long)]
    event_type_filter: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let args = Args::parse();
    let profile = WorkloadProfile::from_cli(
        args.profile,
        args.tx_hold_ms,
        args.read_pct,
        args.trace_file.clone(),
        args.event_type_filter.clone(),
    )?;
    let run_id = args.run_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let db_path = args
        .db_path
        .unwrap_or_else(|| PathBuf::from(format!("./results/{}.db", run_id)));
    let mut trace_assignments = if let WorkloadProfile::TraceReplay {
        trace_path,
        event_type_filter,
    } = &profile
    {
        let trace_events = load_trace_events(trace_path, event_type_filter.as_deref())?;
        let trace_origin = trace_events
            .first()
            .map(|event| event.emitted_at)
            .expect("trace_events should be non-empty after load");
        Some((trace_origin, assign_trace_events(args.workers, trace_events)?))
    } else {
        None
    };

    if let Some(parent) = db_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    cleanup_db_files(&db_path).await?;

    let started_at = Utc::now();
    let pool = build_pool(&db_path, args.workers).await?;
    initialize_schema(&pool).await?;

    let metrics = MetricsCollector::new()?;
    let started = Instant::now();
    let end_at = started + Duration::from_secs(args.duration);

    metrics.start_samplers(db_path.clone(), end_at);

    let barrier = match profile {
        WorkloadProfile::Herd => Some(Arc::new(tokio::sync::Barrier::new(args.workers))),
        _ => None,
    };

    let mut workers = JoinSet::new();

    for worker_id in 0..args.workers {
        let worker_pool = pool.clone();
        let worker_metrics = metrics.clone();
        let worker_barrier = barrier.clone();
        let worker_profile = profile.clone();
        let hold_ms = worker_profile.tx_hold_ms();
        let is_reader = worker_profile.is_reader_worker(worker_id, args.workers);
        let (trace_origin, trace_events_for_worker) = if let Some((origin, assignments)) =
            trace_assignments.as_mut()
        {
            (
                Some(*origin),
                Some(std::mem::take(&mut assignments[worker_id])),
            )
        } else {
            (None, None)
        };

        workers.spawn(async move {
            if let (Some(origin), Some(events)) = (trace_origin, trace_events_for_worker) {
                run_trace_replay_worker(
                    worker_id,
                    worker_pool,
                    worker_metrics,
                    events,
                    origin,
                    started,
                    end_at,
                )
                .await?;
                return Result::<(), anyhow::Error>::Ok(());
            }

            let mut pacer = WorkerPacer::new(worker_profile, worker_id, started, worker_barrier);
            let mut op_index = 0_u64;

            while Instant::now() < end_at {
                let op_started = Instant::now();
                if is_reader {
                    match read_once(&worker_pool).await {
                        Ok(_rows_touched) => {
                            worker_metrics.inc_success();
                            worker_metrics.inc_reads_completed();
                        }
                        Err(e) => {
                            worker_metrics.inc_failure();
                            error!(worker_id, op_index, error = %e, "worker read operation failed");
                        }
                    }
                } else {
                    match execute_peer_review_transaction(
                        &worker_pool,
                        worker_id,
                        op_index,
                        hold_ms,
                        &worker_metrics,
                    )
                    .await
                    {
                        Ok(_) => worker_metrics.inc_success(),
                        Err(e) => {
                            worker_metrics.inc_failure();
                            error!(worker_id, op_index, error = %e, "worker write operation failed");
                        }
                    };
                }
                worker_metrics.record_latency(op_started.elapsed()).await;

                op_index = op_index.saturating_add(1);
                pacer.tick().await;
            }

            Result::<(), anyhow::Error>::Ok(())
        });
    }

    while let Some(result) = workers.join_next().await {
        result??;
    }

    let summary = metrics.summary().await;
    let finished_at = Utc::now();

    let report = RunReport {
        run_id: run_id.clone(),
        profile,
        workers: args.workers,
        duration_secs: args.duration,
        started_at,
        finished_at,
        metrics: summary,
    };

    let results_dir = PathBuf::from("./results");
    let (json_path, md_path) = write_reports(&report, &results_dir)?;

    info!(
        run_id = %run_id,
        json = %json_path.display(),
        markdown = %md_path.display(),
        "stress test completed"
    );

    drop(pool);
    cleanup_db_files(&db_path).await?;

    Ok(())
}

async fn build_pool(db_path: &Path, workers: usize) -> Result<SqlitePool> {
    let connect_options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_millis(5_000))
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections((workers + 4) as u32)
        .connect_with(connect_options)
        .await?;

    sqlx::query("PRAGMA busy_timeout = 5000;")
        .execute(&pool)
        .await?;

    Ok(pool)
}

async fn run_trace_replay_worker(
    worker_id: usize,
    worker_pool: SqlitePool,
    worker_metrics: MetricsCollector,
    events: Vec<TraceEventIn>,
    trace_origin: chrono::DateTime<Utc>,
    replay_started: Instant,
    end_at: Instant,
) -> Result<()> {
    for (op_index, event) in events.into_iter().enumerate() {
        if Instant::now() >= end_at {
            break;
        }

        let target_delay = event
            .emitted_at
            .signed_duration_since(trace_origin)
            .to_std()
            .unwrap_or(Duration::ZERO);
        let target_instant = replay_started + target_delay;
        let sleep_for = target_instant.saturating_duration_since(Instant::now());
        if !sleep_for.is_zero() {
            tokio::time::sleep(sleep_for.min(Duration::from_secs(30))).await;
        }

        if Instant::now() >= end_at {
            break;
        }

        let op_started = Instant::now();
        match write_replay_once(&worker_pool, &event.event_id).await {
            Ok(_) => worker_metrics.inc_success(),
            Err(e) => {
                worker_metrics.inc_failure();
                error!(
                    worker_id,
                    op_index,
                    event_id = %event.event_id,
                    event_type = %event.event_type,
                    error = %e,
                    "worker replay operation failed"
                );
            }
        }
        worker_metrics.record_latency(op_started.elapsed()).await;
    }

    Ok(())
}

async fn initialize_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS review_request (
            id TEXT PRIMARY KEY,
            reviewer_id TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS review_status (
            request_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY(request_id) REFERENCES review_request(id)
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS review_comment (
            id TEXT PRIMARY KEY,
            request_id TEXT NOT NULL,
            body TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY(request_id) REFERENCES review_request(id)
        );
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn cleanup_db_files(db_path: &Path) -> Result<()> {
    let wal = PathBuf::from(format!("{}-wal", db_path.display()));
    let shm = PathBuf::from(format!("{}-shm", db_path.display()));

    for path in [db_path.to_path_buf(), wal, shm] {
        match tokio::fs::remove_file(&path).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
