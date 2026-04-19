use anyhow::{anyhow, Result};
use sqlx::{sqlite::SqliteQueryResult, Error, SqlitePool};

use crate::metrics::MetricsCollector;

pub async fn execute_peer_review_transaction(
    pool: &SqlitePool,
    worker_id: usize,
    op_index: u64,
    metrics: &MetricsCollector,
) -> Result<()> {
    const MAX_BUSY_RETRIES: usize = 3;

    let request_id = format!("req-{}-{}", worker_id, op_index);
    let reviewer_id = format!("reviewer-{}", worker_id);
    let comment_id = format!("comment-{}-{}", worker_id, op_index);

    let mut attempts = 0;
    loop {
        attempts += 1;
        match execute_once(pool, &request_id, &reviewer_id, &comment_id).await {
            Ok(_) => return Ok(()),
            Err(e) if is_busy(&e) => {
                metrics.inc_busy_retry();
                if attempts > MAX_BUSY_RETRIES {
                    return Err(anyhow!("exceeded SQLITE_BUSY retries for {}: {e}", request_id));
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn execute_once(
    pool: &SqlitePool,
    request_id: &str,
    reviewer_id: &str,
    comment_id: &str,
) -> Result<()> {
    let mut tx = pool.begin().await?;

    let insert_request: SqliteQueryResult = sqlx::query(
        r#"
        INSERT INTO review_request (id, reviewer_id, status, created_at)
        VALUES (?1, ?2, 'pending', datetime('now'))
        "#,
    )
    .bind(request_id)
    .bind(reviewer_id)
    .execute(&mut *tx)
    .await?;

    if insert_request.rows_affected() != 1 {
        return Err(anyhow!("failed to insert review request {request_id}"));
    }

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
        VALUES (?1, ?2, 'Concurrency validation comment', datetime('now'))
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
    if let Some(sqlx_err) = err.downcast_ref::<Error>() {
        match sqlx_err {
            Error::Database(db_err) => db_err.message().contains("database is locked"),
            _ => false,
        }
    } else {
        false
    }
}
