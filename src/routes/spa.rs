//! Serves the embedded single-page web portal. The Vite build (see `web/`)
//! produces a single self-contained `web/dist/index.html` (all JS/CSS inlined),
//! which is baked into the binary at compile time.

use axum::{
    http::header,
    response::{IntoResponse, Response},
};

/// The built SPA, inlined into the binary. `build.rs` runs the frontend build
/// before compilation so this file is always current.
static INDEX_HTML: &str = include_str!("../../web/dist/index.html");

/// Serve the SPA shell for any portal route (client-side routing takes over).
pub(crate) async fn serve_spa() -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        INDEX_HTML,
    )
        .into_response()
}
