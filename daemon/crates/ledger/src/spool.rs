use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use shared_types::{DrainResult, RawEvent};
use tracing::instrument;

use crate::LedgerStore;

const MAX_JSON_FIELD_BYTES: usize = 64 * 1024;

fn file_sort_key(path: &Path) -> SystemTime {
    fs::metadata(path)
        .ok()
        .and_then(|meta| meta.created().ok().or_else(|| meta.modified().ok()))
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn truncate_large_strings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            if s.len() > MAX_JSON_FIELD_BYTES {
                *s = "[...truncated]".to_string();
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                truncate_large_strings(child);
            }
        }
        serde_json::Value::Object(map) => {
            for child in map.values_mut() {
                truncate_large_strings(child);
            }
        }
        _ => {}
    }
}

fn sanitize_payload_json(raw: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(mut value) => {
            truncate_large_strings(&mut value);
            serde_json::to_string(&value).unwrap_or_else(|_| raw.to_string())
        }
        Err(_) => raw.to_string(),
    }
}

fn parse_event_from_file(path: &Path) -> anyhow::Result<RawEvent> {
    let body = fs::read_to_string(path)?;
    let mut event: RawEvent = serde_json::from_str(&body)?;
    event.payload_json = sanitize_payload_json(&event.payload_json);
    Ok(event)
}

#[instrument(
    skip_all,
    fields(
        event_type = "spool_drain",
        spool_size = tracing::field::Empty,
        operation = "drain_spool"
    )
)]
pub(crate) fn drain_spool(store: &LedgerStore, spool_dir: &Path) -> anyhow::Result<DrainResult> {
    if !spool_dir.exists() {
        return Ok(DrainResult {
            ingested_count: 0,
            skipped_count: 0,
            failed_count: 0,
        });
    }

    let mut files: Vec<PathBuf> = fs::read_dir(spool_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort_by_key(|path| file_sort_key(path));

    let mut result = DrainResult {
        ingested_count: 0,
        skipped_count: 0,
        failed_count: 0,
    };

    for path in files {
        match parse_event_from_file(&path).and_then(|event| store.ingest_event(event)) {
            Ok(()) => {
                fs::remove_file(&path)?;
                result.ingested_count += 1;
            }
            Err(_) => {
                result.failed_count += 1;
            }
        }
    }

    Ok(result)
}
