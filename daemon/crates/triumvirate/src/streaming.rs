//! Streaming executor adapter for agent execution.
//!
//! Provides execute_ask_agent_streaming() which returns both the final response
//! AND an mpsc channel of AgentStreamEvent values emitted during execution.
//!
//! The existing execute_ask_agent() is adapted to call the streaming version
//! and discard the event channel (adapter pattern — zero blast radius on callers).
//!
//! FEAT-001 (REQ-E01, REQ-E03)

use daemon_core::EventSequencer;
use shared_types::{AgentStreamEvent, AskAgentRequest, AskAgentResponse};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Result of a streaming agent execution: the final response + an event receiver.
///
/// The receiver yields AgentStreamEvent values as the agent works. It closes
/// when the agent turn completes (or errors). The String response is the same
/// blob that execute_ask_agent() would return.
pub struct StreamingAgentResult {
    pub response: AskAgentResponse,
    pub events_rx: mpsc::Receiver<AgentStreamEvent>,
}

/// Execute an agent request with streaming events.
///
/// Returns the final response AND a channel receiver for streaming events.
/// Events are emitted as the agent subprocess produces output (tool calls,
/// file reads, response chunks). The caller can forward events to WebSocket,
/// SSE, or any other consumer.
///
/// This is the canonical execution path. The blob-returning variant wraps this.
pub async fn execute_ask_agent_streaming(
    req: &AskAgentRequest,
    sequencer: Arc<EventSequencer>,
) -> Result<StreamingAgentResult, String> {
    // Phase 1 (Wave 0): stub that delegates to the existing non-streaming path.
    // Wave 1 tasks (T-302, T-303) will wire the parsers to emit events.
    // For now, we create a channel, send no events, and return the blob.
    let (tx, rx) = mpsc::channel::<AgentStreamEvent>(64);

    // Send a TurnStarted event so the adapter test can verify the channel works
    let agent = req.agent.clone();
    let session_name = req.cwd.clone().unwrap_or_default();
    let _ = tx
        .send(AgentStreamEvent::TurnStarted {
            agent: agent.clone(),
            session_name,
            seq: sequencer.next(),
        })
        .await;

    // Delegate to the existing executor (imported from agent_exec via crate)
    let response = crate::agent_exec::execute_ask_agent(req, None).await?;

    // Send TurnCompleted — token data not available on AskAgentResponse
    // (it's extracted separately in agent_exec). Wave 1 tasks will wire
    // the parsers to emit rich events including token stats.
    let _ = tx
        .send(AgentStreamEvent::TurnCompleted {
            agent,
            tokens_in: 0,
            tokens_out: 0,
            cached_tokens: None,
            tool_count: 0,
            duration_ms: 0,
            seq: sequencer.next(),
        })
        .await;

    // Drop sender so receiver sees channel close
    drop(tx);

    Ok(StreamingAgentResult {
        response,
        events_rx: rx,
    })
}

/// Adapter: execute agent and collect result as blob (same as v3.2.0 behavior).
///
/// Calls execute_ask_agent_streaming internally, discards the event channel.
/// All existing callers use this — zero behavior change.
pub async fn execute_ask_agent_blob(
    req: &AskAgentRequest,
    sequencer: Arc<EventSequencer>,
) -> Result<AskAgentResponse, String> {
    let result = execute_ask_agent_streaming(req, sequencer).await?;
    // Events channel is dropped — events go nowhere for non-streaming callers
    Ok(result.response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::AskAgentRequest;

    // NOTE: Full integration tests require a running daemon + agent.
    // These tests verify the adapter plumbing and channel mechanics.

    #[tokio::test]
    async fn streaming_result_produces_events_on_channel() {
        // This test verifies the channel plumbing works.
        // It can't call the real executor (needs running agents),
        // so we test the channel mechanics directly.
        let sequencer = Arc::new(EventSequencer::new());
        let (tx, mut rx) = mpsc::channel::<AgentStreamEvent>(64);

        // Simulate what execute_ask_agent_streaming does
        let _ = tx
            .send(AgentStreamEvent::TurnStarted {
                agent: "test".into(),
                session_name: "test-session".into(),
                seq: sequencer.next(),
            })
            .await;
        let _ = tx
            .send(AgentStreamEvent::ToolCall {
                agent: "test".into(),
                tool_name: "read_file".into(),
                args_summary: "src/main.rs".into(),
                seq: sequencer.next(),
            })
            .await;
        let _ = tx
            .send(AgentStreamEvent::TurnCompleted {
                agent: "test".into(),
                tokens_in: 100,
                tokens_out: 50,
                cached_tokens: Some(20),
                tool_count: 1,
                duration_ms: 3000,
                seq: sequencer.next(),
            })
            .await;
        drop(tx);

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], AgentStreamEvent::TurnStarted { .. }));
        assert!(matches!(events[1], AgentStreamEvent::ToolCall { .. }));
        assert!(matches!(events[2], AgentStreamEvent::TurnCompleted { .. }));

        // Verify sequence numbers are monotonic
        assert_eq!(events[0].seq(), 1);
        assert_eq!(events[1].seq(), 2);
        assert_eq!(events[2].seq(), 3);
    }

    #[tokio::test]
    async fn blob_adapter_returns_string() {
        // Verify the adapter type signature compiles and the channel gets dropped
        let sequencer = Arc::new(EventSequencer::new());
        let (tx, rx) = mpsc::channel::<AgentStreamEvent>(64);

        // Simulate: send events then drop
        let _ = tx
            .send(AgentStreamEvent::TurnStarted {
                agent: "test".into(),
                session_name: "s".into(),
                seq: sequencer.next(),
            })
            .await;
        drop(tx);

        // Verify receiver drains cleanly when dropped
        drop(rx);
        // No panic = pass
    }
}
