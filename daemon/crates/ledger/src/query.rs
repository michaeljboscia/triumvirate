use shared_types::Summary;

use crate::LedgerStore;

pub(crate) fn query_summaries(
    store: &LedgerStore,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<Summary>> {
    store.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT s.id, s.event_id, s.title, s.narrative, s.facts_json, s.concepts_json,
                    s.affected_files_json, s.summary_type, s.created_at
             FROM summaries_fts f
             JOIN summaries s ON s.id = f.rowid
             WHERE summaries_fts MATCH ?1
             ORDER BY bm25(summaries_fts)
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![query, limit as i64], |row| {
            Ok(Summary {
                id: row.get(0)?,
                event_id: row.get(1)?,
                title: row.get(2)?,
                narrative: row.get(3)?,
                facts_json: row.get(4)?,
                concepts_json: row.get(5)?,
                affected_files_json: row.get(6)?,
                summary_type: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })
}
