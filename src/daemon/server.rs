//! The network side of the daemon: HTTP and WebSocket on one port.
//!
//! * `POST /rpc`   a JSON-RPC request (or batch) in the body, the reply in the response;
//! * `GET  /ws`    a WebSocket carrying JSON-RPC both ways, plus the events the daemon pushes;
//! * `POST /sounds?name=<file name>` the bytes of a sound file in the body (what a browser can send); it is
//!   imported like `sounds.import` and the reply is the sound's description;
//! * `GET  /health` answers `ok` without credentials (is a daemon there?);
//! * everything else: the web panel's static files.
//!
//! `/rpc` and `/ws` need the token ([`super::auth`]) and, from a browser, an allowed origin. The server never
//! runs audio code on its own threads: requests that may block (opening devices) go to the blocking pool.

use super::{auth, web, Daemon};
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use omu_protocol::{Response as RpcResponse, RpcError};
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
        .route("/sounds", post(upload))
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
    // Stopping waits for the open connections to end; a panel left open holds a WebSocket for as long as it likes. Give
    // them a few seconds, then stop anyway.
    let (stopping_tx, stopping_rx) = tokio::sync::oneshot::channel::<()>();
    let graceful = async move {
        shutdown.await;
        let _ = stopping_tx.send(());
    };
    let serving = axum::serve(listener, router).with_graceful_shutdown(graceful);
    let result = tokio::select! {
        r = serving => r,
        () = async {
            let _ = stopping_rx.await;
            tokio::time::sleep(Duration::from_secs(3)).await;
        } => {
            tracing::warn!("some clients were still connected 3 s after the stop: closing them");
            Ok(())
        }
    };
    ticker.abort();
    result
}

/// `Some(response)` when the request must be refused.
fn refuse(state: &AppState, headers: &HeaderMap, query: Option<&str>) -> Option<Response> {
    if !auth::origin_allowed(headers, &state.origins) {
        tracing::warn!(origin = ?headers.get(header::ORIGIN), "request refused: origin not allowed");
        return Some((StatusCode::FORBIDDEN, "origin not allowed").into_response());
    }
    match auth::presented_token(headers, query) {
        Some(t) if auth::same_secret(&t, &state.token) => None,
        _ => {
            tracing::warn!("request refused: missing or wrong access token");
            Some((StatusCode::UNAUTHORIZED, unauthorized_body()).into_response())
        }
    }
}

fn unauthorized_body() -> String {
    serde_json::to_string(&RpcResponse::err(None, RpcError::unauthorized())).unwrap_or_default()
}

/// The most a sound upload may be: what a stored sound can hold (a WAV file's size is 32 bits).
const MAX_UPLOAD_BYTES: u64 = u32::MAX as u64;

/// Receives a sound file, writing it to disk as it arrives (a whole video's audio is never held in memory), then
/// imports it.
async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    Query(query): Query<HashMap<String, String>>,
    body: axum::body::Body,
) -> Response {
    use tokio::io::AsyncWriteExt;
    if let Some(r) = refuse(&state, &headers, uri.query()) {
        // Reading what the sender is still sending (a little of it) before answering: a reply sent while the body is
        // arriving makes some systems reset the connection, and the sender then sees a broken connection instead of
        // the reason it was refused.
        let mut stream = body.into_data_stream();
        let mut seen = 0usize;
        while let Some(Ok(chunk)) = stream.next().await {
            seen += chunk.len();
            if seen > 1 << 20 {
                break;
            }
        }
        return r;
    }
    let name = query.get("name").cloned().unwrap_or_else(|| "sound".into());
    let sounds = state.daemon.sounds().clone();
    let failure_with = |status: StatusCode, error: RpcError| {
        let reply = RpcResponse::err(None, error);
        (
            status,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::to_string(&reply).unwrap_or_default(),
        )
            .into_response()
    };
    let failure = |status: StatusCode, message: String| {
        failure_with(status, RpcError::invalid_params(message))
    };
    let path = match sounds.upload_slot(&name) {
        Ok(p) => p,
        Err(e) => return failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let received = async {
        let mut file = tokio::fs::File::create(&path)
            .await
            .map_err(|e| e.to_string())?;
        let mut total = 0u64;
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            total += chunk.len() as u64;
            if total > MAX_UPLOAD_BYTES {
                return Err("the file is too big (the most is 4 GB)".to_string());
            }
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
        file.flush().await.map_err(|e| e.to_string())
    };
    if let Err(e) = received.await {
        let _ = std::fs::remove_file(&path);
        return failure(StatusCode::BAD_REQUEST, e);
    }
    let imported = {
        let (sounds, path, name) = (sounds.clone(), path.clone(), name.clone());
        tokio::task::spawn_blocking(move || sounds.finish_upload(&path, &name)).await
    };
    match imported {
        Ok(Ok(info)) => (
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::to_string(&info).unwrap_or_default(),
        )
            .into_response(),
        Ok(Err(e)) => failure_with(StatusCode::UNPROCESSABLE_ENTITY, crate::sound_error(e)),
        Err(_) => failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the import was interrupted".into(),
        ),
    }
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
        Err(e) => {
            tracing::error!("the request handler failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "the request handler failed",
            )
                .into_response()
        }
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
    tracing::info!("WebSocket client connected");
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
    tracing::info!("WebSocket client disconnected");
}
