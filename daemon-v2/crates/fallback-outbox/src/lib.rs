use daemon_core::{
    acknowledge_dead_drop_ticket, append_outbox_event as core_append_outbox_event,
    count_dead_drop_tickets, create_dead_drop_ticket, gc_dead_drop_tickets, list_dead_drop_tickets,
    read_outbox_events as core_read_outbox_events, triumvirate_home_dir as core_triumvirate_home_dir,
};
use shared_types::OutboxEvent;
use std::path::PathBuf;

pub fn append_outbox_event(event: &OutboxEvent) -> anyhow::Result<()> {
    core_append_outbox_event(&core_triumvirate_home_dir()?, event)
}

pub fn read_outbox_events() -> anyhow::Result<Vec<OutboxEvent>> {
    core_read_outbox_events(&core_triumvirate_home_dir()?)
}

pub fn spawn_dead_drop(
    agent: &str,
    message: &str,
    reason: &str,
    cwd: &Option<String>,
    repo: &Option<String>,
    branch: &Option<String>,
    ticket_id: &str,
) -> anyhow::Result<PathBuf> {
    create_dead_drop_ticket(
        &core_triumvirate_home_dir()?,
        agent,
        message,
        reason,
        cwd,
        repo,
        branch,
        ticket_id,
    )
}

pub fn count_pending_fallbacks() -> anyhow::Result<usize> {
    count_dead_drop_tickets(&core_triumvirate_home_dir()?)
}

pub fn list_pending_fallback_paths(limit: usize) -> anyhow::Result<Vec<PathBuf>> {
    list_dead_drop_tickets(&core_triumvirate_home_dir()?, limit)
}

pub fn acknowledge_fallback_path(path: &str) -> anyhow::Result<()> {
    acknowledge_dead_drop_ticket(&core_triumvirate_home_dir()?, path)
}

pub fn gc_fallbacks(max_age_days: u64) -> anyhow::Result<usize> {
    gc_dead_drop_tickets(&core_triumvirate_home_dir()?, max_age_days)
}
