//! Streamable HTTP MCP transport endpoint.
//!
//! Mounts at /mcp on the daemon's Axum server. Uses rmcp's StreamableHttpService
//! as a tower::Service that handles POST (JSON-RPC tool calls) and GET (SSE stream)
//! with session management via Mcp-Session-Id header.
//!
//! Bearer token auth is enforced via a tower middleware layer on the /mcp route,
//! matching the existing HTTP API auth pattern.
//!
//! FEAT-002 (REQ-H01, REQ-H02, REQ-H03, REQ-H04, REQ-H07, REQ-H09)
//! FEAT-011 / FEAT-014 (REQ-010, REQ-033) — Pantheon session linking via
//! X-Pantheon-Session-Id / X-Pantheon-Root-Session-Id headers.

use std::sync::Arc;

use axum::{Router, middleware};
use axum::http::{Request, StatusCode, header::AUTHORIZATION};
use daemon_core::{PANTHEON_SESSION, PantheonSessionContext};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig,
    StreamableHttpService,
    session::local::LocalSessionManager,
};
use tokio_util::sync::CancellationToken;

use crate::McpBridge;

/// Canonical header name for Pantheon session identification.
///
/// FEAT-014 (REQ-010, REQ-033): Set by Pantheon's MCP proxy when a Claude
/// Code PTY child dispatches via HTTP. Extracted by `bearer_auth_middleware`
/// and stored in the `PANTHEON_SESSION` tokio task-local in `daemon-core`.
/// ABE dispatch reads it via `daemon_core::current_pantheon_session()` BEFORE
/// spawning any monitor tasks, then passes the lineage explicitly into
/// `TaskTracker::register()`.
pub const PANTHEON_SESSION_HEADER: &str = "x-pantheon-session-id";

/// Optional second header for explicit root-session propagation in chained
/// dispatches. If absent, the middleware treats `X-Pantheon-Session-Id` as
/// both parent and root.
pub const PANTHEON_ROOT_SESSION_HEADER: &str = "x-pantheon-root-session-id";

/// Build an Axum Router for the /mcp endpoint with auth + StreamableHttpService.
///
/// The returned router includes bearer token auth middleware and the rmcp
/// StreamableHttpService as a nested service. Mount via:
/// ```rust,ignore
/// .nest("/mcp", http_mcp::build_mcp_router(bridge, token, cancel))
/// ```
pub fn build_mcp_router(
    bridge_template: McpBridge,
    bearer_token: String,
    cancellation_token: CancellationToken,
) -> Router {
    let mcp_service = build_streamable_http_service(bridge_template, cancellation_token);

    Router::new()
        .fallback_service(mcp_service)
        .layer(middleware::from_fn(move |req, next| {
            let token = bearer_token.clone();
            bearer_auth_middleware(token, req, next)
        }))
}

/// Bearer token auth middleware for the /mcp endpoint.
///
/// - REQ-H09: Returns 401 Unauthorized if the token is missing or wrong.
/// - REQ-010 / REQ-033 (T-004): On authorized requests, extracts the
///   `X-Pantheon-Session-Id` (and optional `X-Pantheon-Root-Session-Id`)
///   headers and scopes the downstream handler chain in a
///   `PANTHEON_SESSION.scope(...)` so that ABE dispatch code can read the
///   lineage via `daemon_core::current_pantheon_session()`.
///
/// Non-Pantheon callers (the legacy CLI, `curl`, tests) simply omit the
/// header and the scope holds `None`, which propagates as "no lineage" into
/// WorkerLifecycle events.
async fn bearer_auth_middleware(
    expected_token: String,
    req: Request<axum::body::Body>,
    next: middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let auth_header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let is_authorized = auth_header
        .as_deref()
        .and_then(|h: &str| h.strip_prefix("Bearer "))
        .is_some_and(|token| token == expected_token);

    if !is_authorized {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // FEAT-014 (REQ-010) T-004: extract Pantheon lineage headers (if any).
    // Missing / empty / non-ASCII values all collapse to `None`.
    let pantheon_parent = req
        .headers()
        .get(PANTHEON_SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);

    let pantheon_root = req
        .headers()
        .get(PANTHEON_ROOT_SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);

    let scope_value: Option<Arc<PantheonSessionContext>> = pantheon_parent.map(|parent| {
        let ctx = match pantheon_root {
            Some(root) => PantheonSessionContext::with_root(parent, root),
            None => PantheonSessionContext::new(parent),
        };
        Arc::new(ctx)
    });

    // Wrap the downstream handler in PANTHEON_SESSION.scope so that every
    // `.await` inside the MCP tool call chain (including `tracker.register()`)
    // can observe the lineage via task-local reads. tokio::spawn boundaries
    // inside dispatch code are handled by explicit capture there — see
    // mcp_tools::abe::dispatch_codex.
    let response = PANTHEON_SESSION
        .scope(scope_value, async move { next.run(req).await })
        .await;

    Ok(response)
}

fn build_streamable_http_service(
    bridge_template: McpBridge,
    cancellation_token: CancellationToken,
) -> StreamableHttpService<McpBridge, LocalSessionManager> {
    let config = StreamableHttpServerConfig::default()
        .with_sse_keep_alive(Some(std::time::Duration::from_secs(15)))
        .with_sse_retry(Some(std::time::Duration::from_secs(3)))
        .with_stateful_mode(true)
        .with_json_response(false)
        .with_cancellation_token(cancellation_token);

    let session_manager = Arc::new(LocalSessionManager::default());

    StreamableHttpService::new(
        move || Ok(bridge_template.clone()),
        session_manager,
        config,
    )
}

#[cfg(test)]
mod tests {
    //! Reality tests for T-004 (REQ-010, REQ-033).
    //!
    //! These drive the `bearer_auth_middleware` directly with synthetic
    //! `Request<Body>` objects and inspect what `current_pantheon_session()`
    //! returns from inside the downstream handler. No real HTTP server,
    //! no real MCP — the middleware is the entire unit under test.

    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use daemon_core::current_pantheon_session;
    use std::sync::{Mutex as StdMutex, OnceLock};
    use tower::ServiceExt;

    /// Guards single-threaded use of the shared captured-state cell.
    /// Axum's `middleware::from_fn` test harness will run closures in
    /// parallel otherwise, and we want deterministic reads.
    fn capture_cell() -> &'static StdMutex<Option<Option<Arc<PantheonSessionContext>>>> {
        static CELL: OnceLock<StdMutex<Option<Option<Arc<PantheonSessionContext>>>>> =
            OnceLock::new();
        CELL.get_or_init(|| StdMutex::new(None))
    }

    /// Build a tiny Axum router with `bearer_auth_middleware` layered on top
    /// of a handler that captures the current PANTHEON_SESSION into the test
    /// cell and returns 200 OK.
    fn make_test_router(token: &str) -> axum::Router {
        let capture_handler = |_req: Request<Body>| async move {
            let got = current_pantheon_session();
            *capture_cell().lock().unwrap() = Some(got);
            axum::response::Response::builder()
                .status(StatusCode::OK)
                .body(Body::empty())
                .unwrap()
        };

        axum::Router::new()
            .fallback(capture_handler)
            .layer(axum::middleware::from_fn({
                let token = token.to_string();
                move |req, next| {
                    let token = token.clone();
                    bearer_auth_middleware(token, req, next)
                }
            }))
    }

    fn reset_capture() {
        *capture_cell().lock().unwrap() = None;
    }

    fn taken_capture() -> Option<Arc<PantheonSessionContext>> {
        capture_cell()
            .lock()
            .unwrap()
            .take()
            .expect("handler should have captured")
    }

    #[tokio::test]
    async fn middleware_propagates_pantheon_session_header_to_task_local() {
        reset_capture();
        let router = make_test_router("secret");

        let req = Request::builder()
            .uri("/mcp")
            .method("POST")
            .header("Authorization", "Bearer secret")
            .header(PANTHEON_SESSION_HEADER, "pantheon-sess-123")
            .body(Body::from("{}"))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let captured = taken_capture().expect("scope should have been populated");
        assert_eq!(captured.parent_session_id, "pantheon-sess-123");
        // No root header supplied → root defaults to parent.
        assert_eq!(captured.root_session_id, "pantheon-sess-123");
    }

    #[tokio::test]
    async fn middleware_honors_explicit_root_session_header() {
        reset_capture();
        let router = make_test_router("secret");

        let req = Request::builder()
            .uri("/mcp")
            .method("POST")
            .header("Authorization", "Bearer secret")
            .header(PANTHEON_SESSION_HEADER, "intermediate-worker")
            .header(PANTHEON_ROOT_SESSION_HEADER, "pantheon-root")
            .body(Body::from("{}"))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let captured = taken_capture().expect("scope should have been populated");
        assert_eq!(captured.parent_session_id, "intermediate-worker");
        assert_eq!(captured.root_session_id, "pantheon-root");
    }

    #[tokio::test]
    async fn middleware_missing_pantheon_header_leaves_scope_none() {
        reset_capture();
        let router = make_test_router("secret");

        let req = Request::builder()
            .uri("/mcp")
            .method("POST")
            .header("Authorization", "Bearer secret")
            .body(Body::from("{}"))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Handler ran inside PANTHEON_SESSION.scope(None, ...) → None.
        assert!(taken_capture().is_none());
    }

    #[tokio::test]
    async fn middleware_rejects_missing_bearer_before_scope_runs() {
        reset_capture();
        let router = make_test_router("secret");

        let req = Request::builder()
            .uri("/mcp")
            .method("POST")
            .header(PANTHEON_SESSION_HEADER, "should-never-be-read")
            .body(Body::from("{}"))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // Handler should never have run, so the cell is still None.
        assert!(capture_cell().lock().unwrap().is_none());
        // Drain the body to keep tower happy.
        let _ = to_bytes(resp.into_body(), usize::MAX).await;
    }

    #[tokio::test]
    async fn middleware_rejects_wrong_bearer() {
        reset_capture();
        let router = make_test_router("secret");

        let req = Request::builder()
            .uri("/mcp")
            .method("POST")
            .header("Authorization", "Bearer wrong")
            .header(PANTHEON_SESSION_HEADER, "x")
            .body(Body::from("{}"))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert!(capture_cell().lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn middleware_treats_empty_header_as_none() {
        reset_capture();
        let router = make_test_router("secret");

        let req = Request::builder()
            .uri("/mcp")
            .method("POST")
            .header("Authorization", "Bearer secret")
            .header(PANTHEON_SESSION_HEADER, "   ")
            .body(Body::from("{}"))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(taken_capture().is_none());
    }
}
