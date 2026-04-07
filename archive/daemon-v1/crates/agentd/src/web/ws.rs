use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use tracing::warn;

use super::server::AppState;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_fabric(socket, state))
}

async fn stream_fabric(mut socket: WebSocket, state: AppState) {
    let mut rx = state.bus.subscribe_all().await;

    while let Ok(msg) = rx.recv().await {
        match serde_json::to_string(&msg) {
            Ok(json) => {
                if socket.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to serialize websocket event");
            }
        }
    }
}
