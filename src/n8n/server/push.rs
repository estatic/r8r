//! The editor's push channel (`/rest/push`, spec §6.9): a WebSocket per
//! browser tab, identified by `pushRef`, carrying n8n's
//! `{ "type", "data" }` messages.

use super::auth::SessionUser;
use super::N8n;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct PushMessage {
    /// `None` = every connected editor.
    pub push_ref: Option<String>,
    pub body: Value,
}

pub async fn connect(
    State(n8n): State<Arc<N8n>>,
    _user: SessionUser,
    Query(q): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    let push_ref = q.get("pushRef").cloned().unwrap_or_default();
    let rx = n8n.push.subscribe();
    ws.on_upgrade(move |socket| forward(socket, push_ref, rx))
}

async fn forward(mut socket: WebSocket, push_ref: String, mut rx: tokio::sync::broadcast::Receiver<PushMessage>) {
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(m) => {
                    if m.push_ref.as_deref().is_some_and(|r| r != push_ref) {
                        continue;
                    }
                    if socket.send(Message::Text(m.body.to_string())).await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                _ => {}
            },
        }
    }
}
