//! The web panel: static files built from `ui/` (see `just ui-build`) and embedded in the daemon, so that one
//! executable serves its own control page, locally or remotely.
//!
//! In a debug build the files are read from `ui/dist` at request time (rebuild the panel, reload the page); in a
//! release build they are inside the executable. Without a built panel the daemon still runs and serves a page that
//! says how to build it.

use axum::http::Uri;
#[cfg(feature = "web-panel")]
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
#[cfg(feature = "web-panel")]
use rust_embed::RustEmbed;

#[cfg(feature = "web-panel")]
#[derive(RustEmbed)]
#[folder = "ui/dist/"]
struct Assets;

const NOT_BUILT: &str = "<!doctype html><meta charset=utf-8><title>Panel not built</title>\
<body style=\"font:16px system-ui;max-width:42rem;margin:4rem auto;padding:0 1rem\">\
<h1>The web panel is not built</h1>\
<p>The daemon is running and answers the control protocol, but this build does not contain the panel.</p>\
<p>Build it with <code>just ui-build</code> (needs Node.js), then rebuild or restart the daemon.</p></body>";

/// Without the panel in the executable there is nothing to find: only the page that says so.
#[cfg(not(feature = "web-panel"))]
pub fn serve(_uri: &Uri) -> Response {
    Html(NOT_BUILT).into_response()
}

/// Serves a file of the panel; unknown paths without an extension get the application page (client-side routing).
#[cfg(feature = "web-panel")]
pub fn serve(uri: &Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let asset = Assets::get(path).or_else(|| {
        (!path.contains('.'))
            .then(|| Assets::get("index.html"))
            .flatten()
    });
    match asset {
        Some(file) => {
            let mime = file.metadata.mimetype().to_owned();
            let cache = if path.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, cache.to_owned()),
                ],
                file.data.into_owned(),
            )
                .into_response()
        }
        None if path == "index.html" => Html(NOT_BUILT).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
