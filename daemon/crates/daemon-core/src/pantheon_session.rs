//! Pantheon session lineage propagation.
//!
//! FEAT-011 / FEAT-014 (REQ-010, REQ-033).
//!
//! Pantheon's TUI/GUI is a parent session that dispatches workers (Codex,
//! Claude) into the daemon via MCP. For the sidebar worker hierarchy to
//! render correctly, each dispatched worker must carry a `parent_session_id`
//! and `root_session_id` that identify the Pantheon session that launched it.
//!
//! The ID enters the daemon via one of two MCP transports:
//!
//! - **HTTP Streamable**: Pantheon's MCP client sets the `X-Pantheon-Session-Id`
//!   header on every request. The bearer-auth middleware extracts it and wraps
//!   downstream handlers in `PANTHEON_SESSION_ID.scope(...)`.
//! - **stdio**: Pantheon's MCP client writes the session ID into the JSON-RPC
//!   `_meta.pantheon.session_id` field of every tool call. The stdio handler
//!   reads it and does the same scope wrap. (Wired separately; this module
//!   only provides the task-local substrate.)
//!
//! Read path:
//!
//! - `current_pantheon_session_id()` returns the scoped value if the current
//!   async task is running inside a `PANTHEON_SESSION_ID.scope(...)` closure.
//!   Returns `None` for non-Pantheon callers (legacy CLI, tests, manual
//!   `curl` against the MCP endpoint).
//!
//! CRITICAL: task-locals do NOT propagate across `tokio::spawn`. Code that
//! spawns background monitor tasks (e.g. ABE dispatch) MUST read the ID in
//! the parent task and capture it into an owned `Option<String>` before
//! moving it into the spawned closure.

use std::sync::Arc;

tokio::task_local! {
    /// The Pantheon session ID + root session ID for the current MCP request.
    ///
    /// Set by the HTTP MCP middleware (triumvirate::http_mcp::bearer_auth_middleware)
    /// or the stdio MCP dispatch hook when the caller has identified itself
    /// as a Pantheon session via `X-Pantheon-Session-Id` / `_meta.pantheon.session_id`.
    ///
    /// None means no Pantheon context — the request came from a legacy CLI,
    /// a test, or a manual caller. WorkerLifecycle events will be emitted
    /// without lineage in that case, and Pantheon's sidebar will show the
    /// worker as a top-level root.
    pub static PANTHEON_SESSION: Option<Arc<PantheonSessionContext>>;
}

/// Lineage context captured from an inbound MCP request.
///
/// `parent_session_id` identifies the immediate caller (the Pantheon session
/// that issued this dispatch). `root_session_id` identifies the top of the
/// dispatch chain — for a direct Pantheon dispatch these are equal, but if
/// a worker itself dispatches a sub-worker, the sub-worker's parent is the
/// intermediate worker while root remains the original Pantheon session.
///
/// For v3.9.0 the two are almost always equal; the distinction exists so
/// future chained dispatches (v4.0+) can slot in without re-plumbing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PantheonSessionContext {
    pub parent_session_id: String,
    pub root_session_id: String,
}

impl PantheonSessionContext {
    /// Construct from a parent ID, defaulting root to parent (direct dispatch).
    pub fn new(parent_session_id: impl Into<String>) -> Self {
        let parent = parent_session_id.into();
        Self {
            root_session_id: parent.clone(),
            parent_session_id: parent,
        }
    }

    /// Construct with an explicit root (chained dispatch).
    pub fn with_root(
        parent_session_id: impl Into<String>,
        root_session_id: impl Into<String>,
    ) -> Self {
        Self {
            parent_session_id: parent_session_id.into(),
            root_session_id: root_session_id.into(),
        }
    }
}

/// Read the current Pantheon session context from task-local state.
///
/// Returns `None` if called outside a `PANTHEON_SESSION.scope(...)` closure
/// or if the scope was entered with `None`.
///
/// This is the primary read API for dispatch code. Call this ONCE in the
/// request task and move the result into any spawned monitor tasks — do not
/// call it from inside a `tokio::spawn` closure, because task-locals do not
/// inherit.
pub fn current_pantheon_session() -> Option<Arc<PantheonSessionContext>> {
    PANTHEON_SESSION.try_with(|v| v.clone()).ok().flatten()
}

/// Convenience: read just the parent session ID.
pub fn current_parent_session_id() -> Option<String> {
    current_pantheon_session().map(|ctx| ctx.parent_session_id.clone())
}

/// Convenience: read just the root session ID.
pub fn current_root_session_id() -> Option<String> {
    current_pantheon_session().map(|ctx| ctx.root_session_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn current_returns_none_outside_scope() {
        assert!(current_pantheon_session().is_none());
        assert!(current_parent_session_id().is_none());
        assert!(current_root_session_id().is_none());
    }

    #[tokio::test]
    async fn scope_propagates_context_to_awaited_future() {
        let ctx = Arc::new(PantheonSessionContext::new("pantheon-session-abc"));
        let result = PANTHEON_SESSION
            .scope(Some(ctx.clone()), async {
                let read = current_pantheon_session().expect("scoped");
                (
                    read.parent_session_id.clone(),
                    read.root_session_id.clone(),
                )
            })
            .await;
        assert_eq!(result.0, "pantheon-session-abc");
        assert_eq!(result.1, "pantheon-session-abc");
    }

    #[tokio::test]
    async fn scope_with_none_still_returns_none() {
        let got = PANTHEON_SESSION
            .scope(None, async { current_pantheon_session() })
            .await;
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn with_root_preserves_distinct_root() {
        let ctx = Arc::new(PantheonSessionContext::with_root(
            "worker-intermediate",
            "pantheon-root",
        ));
        let (parent, root) = PANTHEON_SESSION
            .scope(Some(ctx), async {
                (
                    current_parent_session_id().unwrap(),
                    current_root_session_id().unwrap(),
                )
            })
            .await;
        assert_eq!(parent, "worker-intermediate");
        assert_eq!(root, "pantheon-root");
    }

    /// Critical regression test: task-locals do NOT propagate across
    /// `tokio::spawn`. This test documents and enforces that behavior so
    /// nobody is surprised when they refactor dispatch code and lineage
    /// silently becomes None.
    #[tokio::test]
    async fn tokio_spawn_does_not_inherit_task_local() {
        let ctx = Arc::new(PantheonSessionContext::new("pantheon-123"));
        let handle = PANTHEON_SESSION
            .scope(Some(ctx), async {
                // Inside the scope we see it...
                assert!(current_pantheon_session().is_some());
                // ...but a spawned task does NOT.
                tokio::spawn(async { current_pantheon_session() })
            })
            .await;
        let spawned_result = handle.await.unwrap();
        assert!(
            spawned_result.is_none(),
            "tokio::spawn MUST NOT inherit PANTHEON_SESSION task-local; \
             dispatch code must capture the value before spawning"
        );
    }
}
