use shared_types::{Lesson, NewLesson};
use tracing::instrument;

use crate::LedgerStore;

#[derive(Debug, Clone)]
struct LessonRow {
    lesson: Lesson,
    days_since_validation: f64,
}

#[instrument(
    skip_all,
    fields(
        event_type = "lesson_write",
        spool_size = tracing::field::Empty,
        operation = "add_lesson"
    )
)]
pub(crate) fn add_lesson(store: &LedgerStore, lesson: NewLesson) -> anyhow::Result<i64> {
    store.with_conn(|conn| {
        conn.execute(
            "INSERT INTO lessons (title, body, source_session_id, initial_confidence, tags_json, req_ids_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                lesson.title,
                lesson.body,
                lesson.source_session_id,
                lesson.initial_confidence,
                lesson.tags_json,
                lesson.req_ids_json
            ],
        )?;
        Ok(conn.last_insert_rowid())
    })
}

#[instrument(
    skip_all,
    fields(
        event_type = "lesson_query",
        spool_size = tracing::field::Empty,
        operation = "query_lessons"
    )
)]
pub(crate) fn query_lessons(
    store: &LedgerStore,
    query: &str,
    min_confidence: f64,
) -> anyhow::Result<Vec<Lesson>> {
    let rows = store.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT l.lesson_id, l.title, l.body, l.source_session_id, l.created_at, l.last_validated_at,
                    l.initial_confidence, l.tags_json, l.req_ids_json,
                    COALESCE(julianday('now') - julianday(l.last_validated_at), 0.0) AS days_since_validation
             FROM lessons l
             JOIN lessons_fts ON lessons_fts.rowid = l.lesson_id
             WHERE lessons_fts MATCH ?1
             ORDER BY l.lesson_id DESC",
        )?;
        let mapped = stmt.query_map([query], lesson_row_mapper)?;
        let mut out = Vec::new();
        for row in mapped {
            out.push(row?);
        }
        Ok(out)
    })?;

    Ok(apply_confidence_decay(rows, min_confidence))
}

#[instrument(
    skip_all,
    fields(
        event_type = "lesson_validate",
        spool_size = tracing::field::Empty,
        operation = "validate_lesson"
    )
)]
pub(crate) fn validate_lesson(store: &LedgerStore, lesson_id: i64) -> anyhow::Result<()> {
    store.with_conn(|conn| {
        let updated = conn.execute(
            "UPDATE lessons
             SET last_validated_at = datetime('now')
             WHERE lesson_id = ?1",
            [lesson_id],
        )?;
        if updated == 0 {
            anyhow::bail!("lesson not found: {lesson_id}");
        }
        Ok(())
    })
}

#[instrument(
    skip_all,
    fields(
        event_type = "lesson_list",
        spool_size = tracing::field::Empty,
        operation = "list_lessons"
    )
)]
pub(crate) fn list_lessons(
    store: &LedgerStore,
    tags: Option<&[String]>,
    stale_days: Option<f64>,
) -> anyhow::Result<Vec<Lesson>> {
    let rows = store.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT lesson_id, title, body, source_session_id, created_at, last_validated_at,
                    initial_confidence, tags_json, req_ids_json,
                    COALESCE(julianday('now') - julianday(last_validated_at), 0.0) AS days_since_validation
             FROM lessons
             ORDER BY lesson_id DESC",
        )?;
        let mapped = stmt.query_map([], lesson_row_mapper)?;
        let mut out = Vec::new();
        for row in mapped {
            out.push(row?);
        }
        Ok(out)
    })?;

    let required_tags = tags.unwrap_or(&[]);
    let filtered = rows
        .into_iter()
        .filter(|row| match stale_days {
            Some(days) => row.days_since_validation >= days,
            None => true,
        })
        .filter(|row| {
            if required_tags.is_empty() {
                return true;
            }
            let parsed_tags = row
                .lesson
                .tags_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
                .unwrap_or_default();
            required_tags.iter().all(|tag| parsed_tags.iter().any(|t| t == tag))
        })
        .collect::<Vec<_>>();

    Ok(apply_confidence_decay(filtered, 0.0))
}

fn lesson_row_mapper(row: &rusqlite::Row<'_>) -> rusqlite::Result<LessonRow> {
    Ok(LessonRow {
        lesson: Lesson {
            lesson_id: row.get(0)?,
            title: row.get(1)?,
            body: row.get(2)?,
            source_session_id: row.get(3)?,
            created_at: row.get(4)?,
            last_validated_at: row.get(5)?,
            initial_confidence: row.get(6)?,
            tags_json: row.get(7)?,
            req_ids_json: row.get(8)?,
        },
        days_since_validation: row.get(9)?,
    })
}

fn apply_confidence_decay(rows: Vec<LessonRow>, min_confidence: f64) -> Vec<Lesson> {
    rows.into_iter()
        .filter_map(|row| {
            let days = row.days_since_validation.max(0.0);
            let effective_confidence = row.lesson.initial_confidence * f64::exp(-0.01 * days);
            if effective_confidence < min_confidence {
                return None;
            }

            let mut lesson = row.lesson;
            lesson.initial_confidence = effective_confidence;
            Some(lesson)
        })
        .collect()
}
