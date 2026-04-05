use std::path::PathBuf;

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use triumvirate_proto::{AgentId, FabricMessage, Payload, Topic};

use crate::fabric::MessageBus;

/// Spawn a filesystem watcher for the working directory and publish file-change
/// events onto the fabric as mechanical text chunks.
pub fn spawn_file_watcher(bus: std::sync::Arc<MessageBus>, working_dir: PathBuf) {
    tokio::spawn(async move {
        let (tx, mut rx) = mpsc::channel::<Event>(512);

        let mut watcher = match RecommendedWatcher::new(
            move |res| {
                if let Ok(event) = res {
                    let _ = tx.blocking_send(event);
                }
            },
            Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                warn!(error = %e, "failed to initialize file watcher");
                return;
            }
        };

        if let Err(e) = watcher.watch(&working_dir, RecursiveMode::Recursive) {
            warn!(error = %e, path = %working_dir.display(), "failed to watch working directory");
            return;
        }

        debug!(path = %working_dir.display(), "file watcher started");

        while let Some(event) = rx.recv().await {
            let summary = format_event(&event, &working_dir);
            bus.emit(FabricMessage::new(
                AgentId::System,
                Topic::TaskProgress,
                Payload::TextChunk {
                    content: summary,
                    is_final: true,
                },
            ))
            .await;
        }
    });
}

fn format_event(event: &Event, root: &std::path::Path) -> String {
    let mut parts = vec![format!("[FILE_CHANGE] kind={:?}", event.kind)];

    for path in &event.paths {
        let display = path
            .strip_prefix(root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string());
        parts.push(display);
    }

    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use notify::{Event, EventKind};
    use std::path::PathBuf;

    use super::format_event;

    #[test]
    fn format_event_includes_marker_and_path() {
        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("/tmp/project/src/lib.rs")],
            attrs: Default::default(),
        };

        let line = format_event(&event, std::path::Path::new("/tmp/project"));
        assert!(line.contains("[FILE_CHANGE]"));
        assert!(line.contains("src/lib.rs"));
    }
}
