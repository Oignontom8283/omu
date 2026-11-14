//! The network side of the daemon: HTTP and WebSocket on one port.
//!
//! * `POST /rpc`   a JSON-RPC request (or batch) in the body, the reply in the response;
//! * `GET  /ws`    a WebSocket carrying JSON-RPC both ways, plus the events the daemon pushes;
//! * `GET  /health` answers `ok` without credentials (is a daemon there?);
//! * everything else: the web panel's static files.
//!
//! `/rpc` and `/ws` need the token ([`super::auth`]) and, from a browser, an allowed origin. The server never
//! runs audio code on its own threads: requests that may block (opening devices) go to the blocking pool.

use super::{auth, web, Daemon};
use crate::protocol::{Response as RpcResponse, RpcError};
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

pub struct ServerConfig {
    pub listen: SocketAddr,
    pub token: String,
    /// Extra browser origins allowed besides the daemon's own (a development server of the panel, a proxy...).
    pub allowed_origins: Vec<String>,
}

#[derive(Clone)]
struct AppState {
    daemon: Arc<Daemon>,
    token: Arc<String>,
    origins: Arc<Vec<String>>,
    events: broadcast::Sender<String>,
}

/// Binds the address and returns the router with the bound address (the port may have been chosen by the system).
pub async fn bind(
    daemon: Arc<Daemon>,
    cfg: ServerConfig,
) -> std::io::Result<(tokio::net::TcpListener, Router)> {
    let listener = tokio::net::TcpListener::bind(cfg.listen).await?;
    let (events, _) = broadcast::channel::<String>(64);
    let tx = events.clone();
    daemon.on_event(Box::new(move |note| {
        if let Ok(text) = serde_json::to_string(note) {
            let _ = tx.send(text); // no listener is not an error
        }
    }));
    let state = AppState {
        daemon,
        token: Arc::new(cfg.token),
        origins: Arc::new(cfg.allowed_origins),
        events,
    };
    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/rpc", post(rpc))
        .route("/ws", get(ws))
        .fallback(|uri: Uri| async move { web::serve(&uri) })
        .with_state(state);
    Ok((listener, router))
}

/// Runs the server until `shutdown` completes; also drives the status timer.
pub async fn run(
    daemon: Arc<Daemon>,
    listener: tokio::net::TcpListener,
    router: Router,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let ticker = {
        let daemon = daemon.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(250));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let d = daemon.clone();
                let _ = tokio::task::spawn_blocking(move || d.tick()).await;
            }
        })
    };
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await;
    ticker.abort();
    result
}

/// `Some(response)` when the request must be refused.
fn refuse(state: &AppState, headers: &HeaderMap, query: Option<&str>) -> Option<Response> {
    if !auth::origin_allowed(headers, &state.origins) {
        return Some((StatusCode::FORBIDDEN, "origin not allowed").into_response());
    }
    match auth::presented_token(headers, query) {
        Some(t) if auth::same_secret(&t, &state.token) => None,
        _ => Some((StatusCode::UNAUTHORIZED, unauthorized_body()).into_response()),
    }
}

fn unauthorized_body() -> String {
    serde_json::to_string(&RpcResponse::err(None, RpcError::unauthorized())).unwrap_or_default()
}

async fn rpc(State(state): State<AppState>, headers: HeaderMap, uri: Uri, body: Bytes) -> Response {
    if let Some(r) = refuse(&state, &headers, uri.query()) {
        return r;
    }
    let Ok(text) = String::from_utf8(body.to_vec()) else {
        return (StatusCode::BAD_REQUEST, "the body is not UTF-8").into_response();
    };
    let daemon = state.daemon.clone();
    match tokio::task::spawn_blocking(move || daemon.handle_text(&text)).await {
        Ok(Some(reply)) => ([(header::CONTENT_TYPE, "application/json")], reply).into_response(),
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "the request handler failed",
        )
            .into_response(),
    }
}

async fn ws(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let q = query.get("token").map(|t| format!("token={t}"));
    if let Some(r) = refuse(&state, &headers, q.as_deref()) {
        return r;
    }
    upgrade.on_upgrade(move |socket| socket_loop(state, socket))
}

async fn socket_loop(state: AppState, socket: WebSocket) {
    let (mut tx, mut rx) = socket.split();
    let mut events = state.events.subscribe();
    loop {
        tokio::select! {
            incoming = rx.next() => {
                let Some(Ok(msg)) = incoming else { break };
                match msg {
                    Message::Text(text) => {
                        let daemon = state.daemon.clone();
                        let text = text.to_string();
                        let reply = tokio::task::spawn_blocking(move || daemon.handle_text(&text)).await.ok().flatten();
                        if let Some(reply) = reply {
                            if tx.send(Message::Text(reply.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            event = events.recv() => {
                match event {
                    Ok(text) => {
                        if tx.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    // A slow client missed some status events: not an error, the next one is fresh.
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}
