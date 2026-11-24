//! The network side of the daemon: HTTP and WebSocket on one port.
//!
//! * `POST /rpc`   a JSON-RPC request (or batch) in the body, the reply in the response;
//! * `GET  /ws`    a WebSocket carrying JSON-RPC both ways, plus the events the daemon pushes;
//! * `POST /sounds?name=<file name>` the bytes of a sound file in the body (what a browser can send); it is
//!   imported like `sounds.import` and the reply is the sound's description;
//! * `POST /whip/<name>` and `POST /whep/<name>` a WebRTC offer (SDP) in the body, the answer in the reply: a browser or a
//!   program sends sound to a web input (WHIP) or listens to a web output (WHEP); `DELETE` on the address the reply gives ends it;
//! * `GET  /health` answers `ok` without credentials (is a daemon there?);
//! * everything else: the web panel's static files.
//!
//! `/rpc` and `/ws` need the token ([`super::auth`]) and, from a browser, an allowed origin. The server never
//! runs audio code on its own threads: requests that may block (opening devices) go to the blocking pool.

use super::{auth, web, Daemon};
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use omu_protocol::{Response as RpcResponse, RpcError};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

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
    /// The key of pages on other devices: it opens the web sound routes and nothing else.
    guest: Arc<String>,
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
    let guest = auth::new_token();
    daemon.set_guest_token(guest.clone());
    let state = AppState {
        daemon,
        token: Arc::new(cfg.token),
        guest: Arc::new(guest),
        origins: Arc::new(cfg.allowed_origins),
        events,
    };
    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/rpc", post(rpc))
        .route("/ws", get(ws))
        .route("/sounds", post(upload))
        .route("/whip/{name}", post(whip))
        .route("/whip/{name}/{session}", delete(end_connection))
        .route("/whep/{name}", post(whep))
        .route("/whep/{name}/{session}", delete(end_connection))
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

/// `Some(response)` when the request must be refused, whichever key it carries: the one of the program, or (for the web sound
/// routes) the guest key of a page on another device.
fn refuse_guest(state: &AppState, headers: &HeaderMap, query: Option<&str>) -> Option<Response> {
    if !auth::origin_allowed(headers, &state.origins) {
        return refuse(state, headers, query);
    }
    match auth::presented_token(headers, query) {
        Some(t) if auth::same_secret(&t, &state.guest) => None,
        _ => refuse(state, headers, query),
    }
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

/// A sender (a browser, OBS...) connects to a web input.
async fn whip(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    Path(name): Path<String>,
    offer: String,
) -> Response {
    signal(state, headers, uri, name, offer, true).await
}

/// A listener connects to a web output.
async fn whep(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    Path(name): Path<String>,
    offer: String,
) -> Response {
    signal(state, headers, uri, name, offer, false).await
}

/// Hands a WebRTC offer to the hub and returns its answer: `201 Created`, the answer as `application/sdp`, and the address
/// that ends the connection in `Location` (what WHIP and WHEP ask for).
async fn signal(
    state: AppState,
    headers: HeaderMap,
    uri: Uri,
    name: String,
    offer: String,
    sending: bool,
) -> Response {
    if let Some(r) = refuse_guest(&state, &headers, uri.query()) {
        return r;
    }
    let hub = state.daemon.web().clone();
    let kind = if sending { "whip" } else { "whep" };
    let wanted = name.clone();
    // opening the sockets of a connection takes a moment: not on the thread that serves the others
    let answered = tokio::task::spawn_blocking(move || {
        if sending {
            hub.whip(&wanted, &offer)
        } else {
            hub.whep(&wanted, &offer)
        }
    })
    .await;
    use omu_net::web::WebError;
    match answered {
        Ok(Ok(answer)) => (
            StatusCode::CREATED,
            [
                (header::CONTENT_TYPE, "application/sdp".to_owned()),
                (
                    header::LOCATION,
                    format!("/{kind}/{name}/{}", answer.session),
                ),
                (header::ACCESS_CONTROL_EXPOSE_HEADERS, "Location".to_owned()),
            ],
            answer.sdp,
        )
            .into_response(),
        Ok(Err(e @ WebError::NoEndpoint(_))) => {
            (StatusCode::NOT_FOUND, e.to_string()).into_response()
        }
        Ok(Err(e @ WebError::BadOffer(_))) => {
            (StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
        Ok(Err(e @ WebError::Internal(_))) => {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "the connection could not be made",
        )
            .into_response(),
    }
}

/// Ends a connection (the address a WHIP or WHEP reply gave).
async fn end_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    Path((_, session)): Path<(String, String)>,
) -> Response {
    if let Some(r) = refuse_guest(&state, &headers, uri.query()) {
        return r;
    }
    state.daemon.web().close_session(&session);
    StatusCode::NO_CONTENT.into_response()
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
    // the page follows an upload by the id it gave it; the length it announces is what the progress is counted against
    let job = query
        .get("job")
        .cloned()
        .filter(|j| !j.is_empty() && j.len() <= 64);
    let announced = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
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
        if let Some(job) = &job {
            sounds.note_import(job, &name, omu_types::ImportStep::Receiving, 0, announced);
        }
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            total += chunk.len() as u64;
            if let Some(job) = &job {
                sounds.note_import(
                    job,
                    &name,
                    omu_types::ImportStep::Receiving,
                    total,
                    announced,
                );
            }
            if total > MAX_UPLOAD_BYTES {
                return Err("the file is too big (the most is 4 GB)".to_string());
            }
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
        file.flush().await.map_err(|e| e.to_string())
    };
    if let Err(e) = received.await {
        let _ = std::fs::remove_file(&path);
        if let Some(job) = &job {
            sounds.end_import(job);
        }
        return failure(StatusCode::BAD_REQUEST, e);
    }
    let imported = {
        let (sounds, path, name, job) = (sounds.clone(), path.clone(), name.clone(), job.clone());
        tokio::task::spawn_blocking(move || sounds.finish_upload(&path, &name, job.as_deref()))
            .await
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

/// The methods that can take very long (a person's answer to a prompt, a download, a conversion): they run apart, so that the
/// requests that follow on the same socket, and the events, do not wait behind them.
fn is_slow(request: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(request)
        .ok()
        .and_then(|v| v.get("method").and_then(|m| m.as_str()).map(str::to_owned))
        .is_some_and(|m| {
            m == "devices.clean" || m.starts_with("modules.") || m.starts_with("sounds.")
        })
}

/// One WebSocket. Requests are answered in the order they came (a client sets a graph, then starts the engine), except the slow ones
/// (see [`is_slow`]), which are answered when they are done; meanwhile the events keep flowing. A client that was too slow to read the
/// events and missed some is told to read the graph again, since a "graph changed" may have been among them.
async fn socket_loop(state: AppState, socket: WebSocket) {
    tracing::info!("WebSocket client connected");
    let (mut tx, mut rx) = socket.split();
    let mut events = state.events.subscribe();
    let (jobs, mut queue) = mpsc::channel::<String>(256);
    let (replies, mut answers) = mpsc::unbounded_channel::<String>();
    let worker = {
        let daemon = state.daemon.clone();
        tokio::spawn(async move {
            while let Some(text) = queue.recv().await {
                let slow = is_slow(&text);
                let (daemon, replies) = (daemon.clone(), replies.clone());
                let run = async move {
                    let reply = tokio::task::spawn_blocking(move || daemon.handle_text(&text))
                        .await
                        .ok()
                        .flatten();
                    if let Some(reply) = reply {
                        let _ = replies.send(reply);
                    }
                };
                if slow {
                    tokio::spawn(run);
                } else {
                    run.await;
                }
            }
        })
    };
    loop {
        tokio::select! {
            incoming = rx.next() => {
                let Some(Ok(msg)) = incoming else { break };
                match msg {
                    Message::Text(text) => {
                        if jobs.send(text.to_string()).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            Some(reply) = answers.recv() => {
                if tx.send(Message::Text(reply.into())).await.is_err() {
                    break;
                }
            }
            event = events.recv() => {
                match event {
                    Ok(text) => {
                        if tx.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    // some events were missed: the panel reads the graph again on this one (the status events are fresh anyway)
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let resync = r#"{"jsonrpc":"2.0","method":"graph.changed","params":null}"#;
                        if tx.send(Message::Text(resync.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    worker.abort();
    tracing::info!("WebSocket client disconnected");
}

/// The port of the secure server (HTTPS), when other devices are allowed in.
pub const HTTPS_PORT: u16 = 7651;

/// Runs the secure server for as long as the setting says: started when it is switched on, stopped when it is switched off. What
/// it does (or why it could not start) is recorded in the daemon for the panel to show.
pub async fn serve_https(
    daemon: Arc<Daemon>,
    router: Router,
    mut wanted: tokio::sync::watch::Receiver<bool>,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) {
    loop {
        let on = *wanted.borrow_and_update();
        let mut running: Option<axum_server::Handle<SocketAddr>> = None;
        if on {
            match start_https(&daemon, router.clone()).await {
                Ok(handle) => running = Some(handle),
                Err(message) => {
                    tracing::warn!("the secure server could not start: {message}");
                    daemon.set_remote(omu_types::RemoteInfo {
                        port: HTTPS_PORT,
                        error: Some(message),
                        ..Default::default()
                    });
                }
            }
        } else {
            daemon.set_remote(omu_types::RemoteInfo::default());
        }
        // the setting changed, or the daemon is ending (a signal of its own: the setting's sender is also held by the settings listener,
        // which lives as long as the daemon, so it never closes)
        let ended = tokio::select! {
            changed = wanted.changed() => changed.is_err(),
            _ = &mut stop => true,
        };
        if let Some(handle) = running {
            handle.shutdown();
        }
        if ended {
            return;
        }
    }
}

async fn start_https(
    daemon: &Arc<Daemon>,
    router: Router,
) -> Result<axum_server::Handle<SocketAddr>, String> {
    let trust = omu_tls::Trust::from_settings(&daemon.sounds().settings(), daemon.data_dir());
    let data_dir = daemon.data_dir().to_owned();
    let identity =
        tokio::task::spawn_blocking(move || omu_tls::server_identity(&trust, &data_dir, &[]))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
    // the port is taken first, so that a port in use is said instead of failing later
    let listener = std::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], HTTPS_PORT)))
        .map_err(|e| format!("cannot listen on port {HTTPS_PORT}: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let handle = axum_server::Handle::new();
    let config = axum_server::tls_rustls::RustlsConfig::from_config(identity.config.clone());
    let server = axum_server::from_tcp_rustls(listener, config).map_err(|e| e.to_string())?;
    let serving = server
        .handle(handle.clone())
        .serve(router.into_make_service());
    tokio::spawn(async move {
        if let Err(e) = serving.await {
            tracing::warn!("the secure server stopped: {e}");
        }
    });
    tracing::info!(
        port = HTTPS_PORT,
        generated = identity.generated,
        "secure server open"
    );
    daemon.set_remote(omu_types::RemoteInfo {
        enabled: true,
        port: HTTPS_PORT,
        urls: vec![],
        fingerprint: Some(identity.fingerprint),
        certificate_generated: identity.generated,
        error: None,
        names: identity.names.clone(),
    });
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::is_slow;

    #[test]
    fn only_the_methods_that_wait_for_a_person_or_the_network_are_slow() {
        for slow in [
            "devices.clean",
            "modules.install",
            "sounds.import",
            "sounds.compress",
        ] {
            assert!(
                is_slow(&format!(
                    r#"{{"jsonrpc":"2.0","id":1,"method":"{slow}","params":null}}"#
                )),
                "{slow}"
            );
        }
        for fast in [
            "graph.set",
            "engine.status",
            "graph.validate",
            "apps.list",
            "devices.list",
            "settings.get",
        ] {
            assert!(
                !is_slow(&format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{fast}"}}"#)),
                "{fast}"
            );
        }
        assert!(!is_slow("not json"));
    }
}
