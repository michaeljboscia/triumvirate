mod watcher;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::{Connection, params};
use tracing::{info, warn};
use triumvirate_proto::{AgentId, FabricMessage, Payload, Topic};
use uuid::Uuid;

use self::watcher::spawn_file_watcher;
use crate::fabric::MessageBus;
use crate::memory::{LessonOutcome, LessonWrite, extract_decisions, extract_self_reported_lessons, insert_lesson};

/// Stenographer — mechanical extraction of session facts from the fabric.
///
/// Per REQ-2: NO LLM summarization. Captures raw fabric events to JSONL
/// and writes routing decisions into SQLite for traceable replay.
pub struct Stenographer {
    bus: Arc<MessageBus>,
    session_id: Uuid,
    db_path: PathBuf,
    log_path: PathBuf,
    working_dir: PathBuf,
}

impl Stenographer {
    pub fn new(
        bus: Arc<MessageBus>,
        session_id: Uuid,
        db_path: PathBuf,
        working_dir: PathBuf,
    ) -> Self {
        let session_dir = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".triumvirate")
            .join("sessions");

        let log_path = session_dir.join(format!("{session_id}.jsonl"));
        Self {
            bus,
            session_id,
            db_path,
            log_path,
            working_dir,
        }
    }

    /// Start consuming fabric messages in a background task.
    pub fn run(self) {
        tokio::spawn(async move {
            let mut rx = self.bus.subscribe_all().await;
            info!(path = %self.log_path.display(), "stenographer started — listening to all fabric topics");
            spawn_file_watcher(self.bus.clone(), self.working_dir.clone());

            if let Some(parent) = self.log_path.parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                warn!(error = %e, path = %parent.display(), "failed to create session log dir");
            }

            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)
                .ok();

            let db_conn = Connection::open(&self.db_path).ok();

            loop {
                match rx.recv().await {
                    Ok(msg) => self.handle_message(&msg, file.as_mut(), db_conn.as_ref()),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "stenographer lagged — missed messages");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("stenographer shutting down — fabric closed");
                        break;
                    }
                }
            }
        });
    }

    fn handle_message(
        &self,
        msg: &FabricMessage,
        file: Option<&mut std::fs::File>,
        db_conn: Option<&Connection>,
    ) {
        info!(
            id = %msg.id,
            source = %msg.source,
            topic = ?msg.topic,
            "steno: fabric event"
        );

        if let Some(file) = file {
            match serde_json::to_string(msg) {
                Ok(line) => {
                    if writeln!(file, "{line}").is_err() {
                        warn!("failed to append steno jsonl line");
                    }
                }
                Err(e) => warn!(error = %e, "failed to serialize steno event"),
            }
        }

        if let Some(conn) = db_conn {
            self.maybe_log_routing(conn, msg);
            self.maybe_log_decisions(conn, msg);
            self.maybe_log_lessons(conn, msg);
        }
    }

    fn maybe_log_routing(&self, conn: &Connection, msg: &FabricMessage) {
        let Topic::TaskProgress = msg.topic else {
            return;
        };

        let (target, reason, content) = match &msg.payload {
            Payload::RoutingDecision {
                target_agent,
                reason,
                content,
            } => (target_agent, reason, content),
            _ => return,
        };

        // Only router-originated trace events count as routing decisions.
        if msg.source != AgentId::System {
            return;
        }

        if let Err(e) = conn.execute(
            "INSERT INTO routing_log (session_id, source_agent, target_agent, reason, content)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                self.session_id.to_string(),
                AgentId::Human.to_string(),
                target.to_string(),
                reason,
                content,
            ],
        ) {
            warn!(error = %e, "failed to write routing_log row");
        }
    }

    fn maybe_log_decisions(&self, conn: &Connection, msg: &FabricMessage) {
        let source_agent = match msg.topic {
            Topic::AgentOutput(agent) => agent,
            _ => return,
        };

        let content = match &msg.payload {
            Payload::AgentResponse { content, .. } => content.as_str(),
            Payload::TextChunk {
                content,
                is_final: true,
            } => content.as_str(),
            _ => return,
        };

        let decisions = extract_decisions(content);
        for decision in decisions {
            if let Err(e) = conn.execute(
                "INSERT INTO decisions (session_id, decision_text, proposed_by, evidence)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    self.session_id.to_string(),
                    decision,
                    source_agent.to_string(),
                    msg.id.to_string(),
                ],
            ) {
                warn!(error = %e, "failed to write decision row");
            }
        }
    }

    fn maybe_log_lessons(&self, conn: &Connection, msg: &FabricMessage) {
        let source_agent = match msg.topic {
            Topic::AgentOutput(agent) => agent.to_string(),
            _ => return,
        };

        match &msg.payload {
            Payload::Error { message, .. } => {
                let lesson = LessonWrite {
                    decision: format!("Agent error observed: {}", truncate(message, 200)),
                    rationale: "Daemon captured agent error output during execution".to_string(),
                    outcome: LessonOutcome::Failure,
                    confidence_score: 0.9,
                    pattern: "agent_error".to_string(),
                    agent_source: source_agent,
                };
                if let Err(e) = insert_lesson(conn, &lesson) {
                    warn!(error = %e, "failed to write failure lesson");
                }
            }
            Payload::AgentResponse { content, .. }
            | Payload::TextChunk {
                content,
                is_final: true,
            } => {
                for lesson in extract_self_reported_lessons(content, &source_agent) {
                    if let Err(e) = insert_lesson(conn, &lesson) {
                        warn!(error = %e, "failed to write self-reported lesson");
                    }
                }
            }
            _ => {}
        }
    }
}

fn truncate(value: &str, max_len: usize) -> String {
    let mut out = value.chars().take(max_len).collect::<String>();
    if value.chars().count() > max_len {
        out.push_str("...");
    }
    out
}
