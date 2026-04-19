use anyhow::{anyhow, Result};
use sqlx::{sqlite::SqliteQueryResult, Error, SqlitePool};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::metrics::MetricsCollector;

static OP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub async fn execute_peer_review_transaction(
    pool: &SqlitePool,
    worker_id: usize,
    op_index: u64,
    hold_ms: u64,
    metrics: &MetricsCollector,
) -> Result<()> {
    const MAX_BUSY_RETRIES: usize = 3;

    let mut attempts = 0;
    loop {
        attempts += 1;
        let operation = if hold_ms == 0 {
            write_once(pool).await
        } else {
            write_once_with_hold(pool, hold_ms).await
        };

        match operation {
            Ok(_) => return Ok(()),
            Err(e) if is_busy(&e) => {
                metrics.inc_busy_retry();
                if attempts > MAX_BUSY_RETRIES {
                    return Err(anyhow!(
                        "exceeded SQLITE_BUSY retries for worker={worker_id} op_index={op_index}: {e}"
                    ));
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

pub async fn write_once(pool: &SqlitePool) -> Result<()> {
    write_once_with_hold(pool, 0).await
}

pub async fn write_once_with_hold(pool: &SqlitePool, hold_ms: u64) -> Result<()> {
    let sequence = OP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let request_id = format!("req-{sequence}");
    let reviewer_id = format!("reviewer-{}", sequence % 512);
    let comment_id = format!("comment-{sequence}");

    let mut tx = pool.begin().await?;

    let insert_request: SqliteQueryResult = sqlx::query(
        r#"
        INSERT INTO review_request (id, reviewer_id, status, created_at)
        VALUES (?1, ?2, 'pending', datetime('now'))
        "#,
    )
    .bind(&request_id)
    .bind(&reviewer_id)
    .execute(&mut *tx)
    .await?;

    if insert_request.rows_affected() != 1 {
        return Err(anyhow!("failed to insert review request {request_id}"));
    }

    if hold_ms > 0 {
        tokio::time::sleep(Duration::from_millis(hold_ms)).await;
    }

    sqlx::query(
        r#"
        INSERT INTO review_status (request_id, status, updated_at)
        VALUES (?1, 'in_review', datetime('now'))
        ON CONFLICT(request_id)
        DO UPDATE SET status='in_review', updated_at=datetime('now')
        "#,
    )
    .bind(&request_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO review_comment (id, request_id, body, created_at)
        VALUES (?1, ?2, 'Concurrency validation comment', datetime('now'))
        "#,
    )
    .bind(&comment_id)
    .bind(&request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn read_once(pool: &SqlitePool) -> Result<u64> {
    let _total_requests: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM review_request
        "#,
    )
    .fetch_one(pool)
    .await?;

    let recent_requests: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT id
        FROM review_request
        ORDER BY created_at DESC
        LIMIT 20
        "#,
    )
    .fetch_all(pool)
    .await?;

    let _comments_aggregate: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM review_comment
        "#,
    )
    .fetch_one(pool)
    .await?;

    let rows_touched = 2_u64 + recent_requests.len() as u64;
    Ok(rows_touched)
}

fn is_busy(err: &anyhow::Error) -> bool {
    if let Some(sqlx_err) = err.downcast_ref::<Error>() {
        match sqlx_err {
            Error::Database(db_err) => db_err.message().contains("database is locked"),
            _ => false,
        }
    } else {
        false
    }
}
