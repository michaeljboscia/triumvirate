use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use serde::Serialize;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Error as SqlxError, SqlitePool};
use tokio::process::Command;
use tokio::task::JoinSet;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "crash-trial")]
struct Args {
    #[arg(long, default_value_t = 5)]
    trials: u32,

    #[arg(long, default_value_t = 30)]
    workers: usize,

    #[arg(long, default_value_t = 3000)]
    pre_crash_ms: u64,

    #[arg(long, default_value = "./results/crash_trial.db")]
    db_path: PathBuf,

    #[arg(long, default_value = "results/crash_trial.json")]
    out: PathBuf,

    #[arg(long, hide = true, default_value_t = false)]
    child_mode: bool,
}

#[derive(Debug, Serialize)]
struct TrialResult {
    trial_id: u32,
    pre_crash_count: i64,
    post_crash_count: i64,
    integrity_ok: bool,
    wal_replay_duration_ms: u128,
}

#[derive(Debug, Serialize)]
struct CrashTrialReport {
    trials: Vec<TrialResult>,
    all_integrity_ok: bool,
    total_trials: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if args.child_mode {
        run_child_mode(&args).await?;
        return Ok(());
    }

    run_parent_mode(&args).await
}

async fn run_parent_mode(args: &Args) -> Result<()> {
    if let Some(parent) = args.out.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if let Some(parent) = args.db_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut trials = Vec::with_capacity(args.trials as usize);

    for trial_id in 1..=args.trials {
        let trial_db_path = trial_db_path(&args.db_path, trial_id);
        cleanup_db_files(&trial_db_path).await?;

        let setup_pool = connect_pool(&trial_db_path, args.workers).await?;
        initialize_schema(&setup_pool).await?;
        drop(setup_pool);

        let exe = std::env::current_exe().context("failed to resolve current executable")?;
        let mut child = Command::new(exe)
            .arg("--child-mode")
            .arg("--workers")
            .arg(args.workers.to_string())
            .arg("--db-path")
            .arg(&trial_db_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to spawn child writer for trial {trial_id}"))?;

        tokio::time::sleep(Duration::from_millis(args.pre_crash_ms)).await;

        let pre_pool = connect_pool(&trial_db_path, args.workers).await?;
        let pre_crash_count = count_review_requests(&pre_pool).await?;

        child
            .kill()
            .await
            .with_context(|| format!("failed to SIGKILL child for trial {trial_id}"))?;
        let _ = child.wait().await?;
        drop(pre_pool);

        let replay_started = Instant::now();
        let post_pool = connect_pool(&trial_db_path, args.workers).await?;
        let post_crash_count = count_review_requests(&post_pool).await?;
        let integrity_ok = integrity_check(&post_pool).await?;
        let wal_replay_duration_ms = replay_started.elapsed().as_millis();
        drop(post_pool);

        trials.push(TrialResult {
            trial_id,
            pre_crash_count,
            post_crash_count,
            integrity_ok,
            wal_replay_duration_ms,
        });
    }

    let report = CrashTrialReport {
        all_integrity_ok: trials.iter().all(|t| t.integrity_ok),
        total_trials: args.trials,
        trials,
    };

    let json = serde_json::to_string_pretty(&report)?;
    tokio::fs::write(&args.out, json).await?;

    let md_path = args.out.with_extension("md");
    let md = render_markdown_summary(&report);
    tokio::fs::write(&md_path, md).await?;

    println!(
        "Crash trials complete. JSON: {} | Markdown: {}",
        args.out.display(),
        md_path.display()
    );

    Ok(())
}

async fn run_child_mode(args: &Args) -> Result<()> {
    if let Some(parent) = args.db_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let pool = connect_pool(&args.db_path, args.workers).await?;
    initialize_schema(&pool).await?;

    let mut writers = JoinSet::new();

    for worker_id in 0..args.workers {
        let writer_pool = pool.clone();
        writers.spawn(async move {
            let reviewer_id = format!("reviewer-{worker_id}");
            let mut op_index = 0_u64;

            loop {
                let request_id = Uuid::new_v4().to_string();
                let comment_id = format!("comment-{worker_id}-{op_index}-{}", Uuid::new_v4());

                if let Err(err) = execute_peer_review_transaction(
                    &writer_pool,
                    &request_id,
                    &reviewer_id,
                    &comment_id,
                )
                .await
                {
                    if !is_busy(&err) {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                }

                op_index = op_index.saturating_add(1);
            }
        });
    }

    while let Some(join_result) = writers.join_next().await {
        join_result?;
    }

    Ok(())
}

async fn connect_pool(db_path: &Path, workers: usize) -> Result<SqlitePool> {
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

async fn count_review_requests(pool: &SqlitePool) -> Result<i64> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM review_request")
        .fetch_one(pool)
        .await?;
    Ok(count)
}

async fn integrity_check(pool: &SqlitePool) -> Result<bool> {
    let status: String = sqlx::query_scalar("PRAGMA integrity_check;")
        .fetch_one(pool)
        .await?;
    Ok(status.trim() == "ok")
}

async fn execute_peer_review_transaction(
    pool: &SqlitePool,
    request_id: &str,
    reviewer_id: &str,
    comment_id: &str,
) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO review_request (id, reviewer_id, status, created_at)
        VALUES (?1, ?2, 'pending', datetime('now'))
        "#,
    )
    .bind(request_id)
    .bind(reviewer_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO review_status (request_id, status, updated_at)
        VALUES (?1, 'in_review', datetime('now'))
        ON CONFLICT(request_id)
        DO UPDATE SET status='in_review', updated_at=datetime('now')
        "#,
    )
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO review_comment (id, request_id, body, created_at)
        VALUES (?1, ?2, 'Crash trial concurrency validation comment', datetime('now'))
        "#,
    )
    .bind(comment_id)
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

fn is_busy(err: &anyhow::Error) -> bool {
    if let Some(sqlx_err) = err.downcast_ref::<SqlxError>() {
        match sqlx_err {
            SqlxError::Database(db_err) => db_err.message().contains("database is locked"),
            _ => false,
        }
    } else {
        false
    }
}

fn cleanup_trial_suffix(file_name: &str) -> String {
    file_name
        .trim_end_matches(".db")
        .trim_end_matches('.')
        .to_string()
}

fn trial_db_path(base: &Path, trial_id: u32) -> PathBuf {
    let parent = base.parent().map(Path::to_path_buf).unwrap_or_default();
    let file_name = base
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("crash_trial.db");
    let stem = cleanup_trial_suffix(file_name);
    let new_name = format!("{stem}-trial-{trial_id}.db");
    parent.join(new_name)
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

fn render_markdown_summary(report: &CrashTrialReport) -> String {
    let mut lines = Vec::new();

    lines.push("# SQLite Crash Trial Summary".to_string());
    lines.push(String::new());
    lines.push(format!("- Total Trials: {}", report.total_trials));
    lines.push(format!("- All Integrity OK: {}", report.all_integrity_ok));
    lines.push(String::new());
    lines.push("| Trial | Pre-Crash Count | Post-Crash Count | Integrity OK | WAL Replay (ms) |".to_string());
    lines.push("| --- | ---: | ---: | :---: | ---: |".to_string());

    for trial in &report.trials {
        lines.push(format!(
            "| {} | {} | {} | {} | {} |",
            trial.trial_id,
            trial.pre_crash_count,
            trial.post_crash_count,
            if trial.integrity_ok { "yes" } else { "no" },
            trial.wal_replay_duration_ms
        ));
    }

    lines.push(String::new());
    lines.join("\n")
}
