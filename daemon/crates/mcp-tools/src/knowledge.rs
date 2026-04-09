use daemon_core::{
    append_memory_entry as core_append_memory_entry, list_scratchpad as core_list_scratchpad,
    read_memory_entries as core_read_memory_entries, triumvirate_home_dir as core_triumvirate_home_dir,
    unix_time_ms as core_unix_time_ms, write_scratchpad as core_write_scratchpad,
};
use daemon_http::{
    fetch_daemon_fallback_ack, fetch_daemon_fallback_gc, fetch_daemon_fallback_list,
    fetch_daemon_lesson_add, fetch_daemon_lesson_list, fetch_daemon_lesson_query,
    fetch_daemon_lesson_validate, fetch_daemon_ledger_gc, fetch_daemon_ledger_query,
    fetch_daemon_ledger_record, fetch_daemon_ledger_session, fetch_daemon_memory_read,
    fetch_daemon_memory_write, fetch_daemon_outbox_recent, fetch_daemon_scratchpad_list,
    fetch_daemon_scratchpad_write,
};
use fallback_outbox::{
    acknowledge_fallback_path, gc_fallbacks, list_pending_fallback_paths, read_outbox_events,
};
use ledger::LedgerStore;
use rmcp::Json;
use shared_types::{
    FallbackAckRequest, FallbackGcRequest, FallbackGcResponse, FallbackListRequest,
    FallbackListResponse, GcResult, HealthStatus, LedgerQueryRequest, LedgerQueryResponse,
    LedgerSessionRequest, LessonAddResponse, LessonListRequest, LessonListResponse,
    LessonQueryRequest, LessonQueryResponse, LessonValidateRequest, ManualRecord, MemoryEntry,
    MemoryReadRequest, MemoryReadResponse, MemoryWriteRequest, MemoryWriteResponse, NewLesson,
    OutboxRecentRequest, OutboxRecentResponse, ScratchpadListRequest, ScratchpadListResponse,
    ScratchpadWriteRequest, ScratchpadWriteResponse, SessionDetail,
};
use std::path::PathBuf;
use uuid::Uuid;

pub async fn memory_write(
    req: MemoryWriteRequest,
    daemon_proxy_enabled: bool,
) -> Result<Json<MemoryWriteResponse>, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_memory_write(&req)
            .await
            .map(Json)
            .map_err(|e| format!("memory_write via daemon failed: {e}"));
    }
    let id = Uuid::new_v4().to_string();
    let entry = MemoryEntry {
        id: id.clone(),
        namespace: req.namespace,
        key: req.key,
        value: req.value,
        ts_ms: core_unix_time_ms(),
    };
    append_memory_entry(&entry).map_err(|e| format!("memory_write failed: {e}"))?;
    Ok(Json(MemoryWriteResponse {
        id,
        status: "ok".to_string(),
    }))
}

pub async fn memory_read(
    req: MemoryReadRequest,
    daemon_proxy_enabled: bool,
) -> Result<Json<MemoryReadResponse>, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_memory_read(&req)
            .await
            .map(Json)
            .map_err(|e| format!("memory_read via daemon failed: {e}"));
    }
    let mut entries = read_memory_entries().map_err(|e| format!("memory_read failed: {e}"))?;
    entries.retain(|e| e.namespace == req.namespace);
    if let Some(key) = req.key {
        entries.retain(|e| e.key == key);
    }
    entries.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms));
    if let Some(limit) = req.limit {
        entries.truncate(limit);
    }
    Ok(Json(MemoryReadResponse { entries }))
}

pub async fn scratchpad_write(
    req: ScratchpadWriteRequest,
    daemon_proxy_enabled: bool,
) -> Result<Json<ScratchpadWriteResponse>, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_scratchpad_write(&req)
            .await
            .map(Json)
            .map_err(|e| format!("scratchpad_write via daemon failed: {e}"));
    }
    let path = write_scratchpad(&req.project, &req.topic, &req.content)
        .map_err(|e| format!("scratchpad_write failed: {e}"))?;
    Ok(Json(ScratchpadWriteResponse {
        path: path.display().to_string(),
    }))
}

pub async fn scratchpad_list(
    req: ScratchpadListRequest,
    daemon_proxy_enabled: bool,
) -> Result<Json<ScratchpadListResponse>, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_scratchpad_list(&req)
            .await
            .map(Json)
            .map_err(|e| format!("scratchpad_list via daemon failed: {e}"));
    }
    let files = list_scratchpad(&req.project)
        .map_err(|e| format!("scratchpad_list failed: {e}"))?
        .into_iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>();
    Ok(Json(ScratchpadListResponse { files }))
}

pub async fn outbox_recent(
    req: OutboxRecentRequest,
    daemon_proxy_enabled: bool,
) -> Result<Json<OutboxRecentResponse>, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_outbox_recent(&req)
            .await
            .map(Json)
            .map_err(|e| format!("outbox_recent via daemon failed: {e}"));
    }
    let mut events = read_outbox_events().map_err(|e| format!("outbox_recent failed: {e}"))?;
    events.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms));
    events.truncate(req.limit.unwrap_or(50));
    Ok(Json(OutboxRecentResponse { events }))
}

pub async fn fallback_list(
    req: FallbackListRequest,
    daemon_proxy_enabled: bool,
) -> Result<Json<FallbackListResponse>, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_fallback_list(&req)
            .await
            .map(Json)
            .map_err(|e| format!("fallback_list via daemon failed: {e}"));
    }
    let tickets = list_pending_fallback_paths(req.limit.unwrap_or(20))
        .map_err(|e| format!("fallback_list failed: {e}"))?
        .into_iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>();
    Ok(Json(FallbackListResponse { tickets }))
}

pub async fn fallback_ack(
    req: FallbackAckRequest,
    daemon_proxy_enabled: bool,
) -> Result<String, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_fallback_ack(&req)
            .await
            .map_err(|e| format!("fallback_ack via daemon failed: {e}"));
    }
    acknowledge_fallback_path(&req.path).map_err(|e| format!("fallback_ack failed: {e}"))?;
    Ok(format!("acknowledged {}", req.path))
}

pub async fn fallback_gc(
    req: FallbackGcRequest,
    daemon_proxy_enabled: bool,
) -> Result<Json<FallbackGcResponse>, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_fallback_gc(&req)
            .await
            .map(Json)
            .map_err(|e| format!("fallback_gc via daemon failed: {e}"));
    }
    let removed = gc_fallbacks(req.max_age_days.unwrap_or(7))
        .map_err(|e| format!("fallback_gc failed: {e}"))?;
    Ok(Json(FallbackGcResponse { removed }))
}

pub async fn ledger_health() -> Result<Json<HealthStatus>, String> {
    let project_root =
        std::env::current_dir().map_err(|e| format!("failed to determine current directory: {e}"))?;
    let store = LedgerStore::open(project_root).map_err(|e| format!("failed to open ledger store: {e}"))?;
    let health = store
        .health()
        .map_err(|e| format!("failed to query ledger health: {e}"))?;
    Ok(Json(health))
}

pub async fn ledger_query(
    req: LedgerQueryRequest,
    daemon_proxy_enabled: bool,
) -> Result<Json<LedgerQueryResponse>, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_ledger_query(&req)
            .await
            .map(Json)
            .map_err(|e| format!("ledger_query via daemon failed: {e}"));
    }
    let project_root =
        std::env::current_dir().map_err(|e| format!("failed to determine current directory: {e}"))?;
    let store = LedgerStore::open(project_root).map_err(|e| format!("failed to open ledger store: {e}"))?;
    let summaries = store
        .query(&req.query, req.limit.unwrap_or(10))
        .map_err(|e| format!("ledger_query failed: {e}"))?;
    Ok(Json(LedgerQueryResponse { summaries }))
}

pub async fn ledger_session(
    req: LedgerSessionRequest,
    daemon_proxy_enabled: bool,
) -> Result<Json<SessionDetail>, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_ledger_session(&req)
            .await
            .map(Json)
            .map_err(|e| format!("ledger_session via daemon failed: {e}"));
    }
    let project_root =
        std::env::current_dir().map_err(|e| format!("failed to determine current directory: {e}"))?;
    let store = LedgerStore::open(project_root).map_err(|e| format!("failed to open ledger store: {e}"))?;
    store
        .get_session(&req.session_id)
        .map(Json)
        .map_err(|e| format!("ledger_session failed: {e}"))
}

pub async fn ledger_record(
    req: ManualRecord,
    daemon_proxy_enabled: bool,
) -> Result<String, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_ledger_record(&req)
            .await
            .map_err(|e| format!("ledger_record via daemon failed: {e}"));
    }
    let project_root =
        std::env::current_dir().map_err(|e| format!("failed to determine current directory: {e}"))?;
    let store = LedgerStore::open(project_root).map_err(|e| format!("failed to open ledger store: {e}"))?;
    store
        .record(req)
        .map_err(|e| format!("ledger_record failed: {e}"))?;
    Ok("ok".to_string())
}

pub async fn ledger_gc(daemon_proxy_enabled: bool) -> Result<Json<GcResult>, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_ledger_gc()
            .await
            .map(Json)
            .map_err(|e| format!("ledger_gc via daemon failed: {e}"));
    }
    let project_root =
        std::env::current_dir().map_err(|e| format!("failed to determine current directory: {e}"))?;
    let store = LedgerStore::open(project_root).map_err(|e| format!("failed to open ledger store: {e}"))?;
    let result = store.gc().map_err(|e| format!("ledger_gc failed: {e}"))?;
    Ok(Json(result))
}

pub async fn lesson_add(
    req: NewLesson,
    daemon_proxy_enabled: bool,
) -> Result<Json<LessonAddResponse>, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_lesson_add(&req)
            .await
            .map(Json)
            .map_err(|e| format!("lesson_add via daemon failed: {e}"));
    }
    let project_root =
        std::env::current_dir().map_err(|e| format!("failed to determine current directory: {e}"))?;
    let store = LedgerStore::open(project_root).map_err(|e| format!("failed to open ledger store: {e}"))?;
    let lesson_id = store
        .add_lesson(req)
        .map_err(|e| format!("lesson_add failed: {e}"))?;
    Ok(Json(LessonAddResponse { lesson_id }))
}

pub async fn lesson_query(
    req: LessonQueryRequest,
    daemon_proxy_enabled: bool,
) -> Result<Json<LessonQueryResponse>, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_lesson_query(&req)
            .await
            .map(Json)
            .map_err(|e| format!("lesson_query via daemon failed: {e}"));
    }
    let project_root =
        std::env::current_dir().map_err(|e| format!("failed to determine current directory: {e}"))?;
    let store = LedgerStore::open(project_root).map_err(|e| format!("failed to open ledger store: {e}"))?;
    let lessons = store
        .query_lessons(&req.query, req.min_confidence.unwrap_or(0.0))
        .map_err(|e| format!("lesson_query failed: {e}"))?;
    Ok(Json(LessonQueryResponse { lessons }))
}

pub async fn lesson_validate(
    req: LessonValidateRequest,
    daemon_proxy_enabled: bool,
) -> Result<String, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_lesson_validate(&req)
            .await
            .map_err(|e| format!("lesson_validate via daemon failed: {e}"));
    }
    let project_root =
        std::env::current_dir().map_err(|e| format!("failed to determine current directory: {e}"))?;
    let store = LedgerStore::open(project_root).map_err(|e| format!("failed to open ledger store: {e}"))?;
    store
        .validate_lesson(req.lesson_id)
        .map_err(|e| format!("lesson_validate failed: {e}"))?;
    Ok("ok".to_string())
}

pub async fn lesson_list(
    req: LessonListRequest,
    daemon_proxy_enabled: bool,
) -> Result<Json<LessonListResponse>, String> {
    if daemon_proxy_enabled {
        return fetch_daemon_lesson_list(&req)
            .await
            .map(Json)
            .map_err(|e| format!("lesson_list via daemon failed: {e}"));
    }
    let project_root =
        std::env::current_dir().map_err(|e| format!("failed to determine current directory: {e}"))?;
    let store = LedgerStore::open(project_root).map_err(|e| format!("failed to open ledger store: {e}"))?;
    let tags_ref = req.tags.as_deref();
    let lessons = store
        .list_lessons(tags_ref, req.stale_days)
        .map_err(|e| format!("lesson_list failed: {e}"))?;
    Ok(Json(LessonListResponse { lessons }))
}

fn append_memory_entry(entry: &MemoryEntry) -> anyhow::Result<()> {
    core_append_memory_entry(&core_triumvirate_home_dir()?, entry)
}

fn read_memory_entries() -> anyhow::Result<Vec<MemoryEntry>> {
    core_read_memory_entries(&core_triumvirate_home_dir()?)
}

fn write_scratchpad(project: &str, topic: &str, content: &str) -> anyhow::Result<PathBuf> {
    core_write_scratchpad(
        &core_triumvirate_home_dir()?,
        project,
        topic,
        content,
        core_unix_time_ms(),
    )
}

fn list_scratchpad(project: &str) -> anyhow::Result<Vec<PathBuf>> {
    core_list_scratchpad(&core_triumvirate_home_dir()?, project)
}
