use crate::app::{AppState, SubsonicParams};
use crate::auth_mw::{resolve_user, xml_error, RESPONSE_CTX};
use crate::response::{self, ResponseFormat};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

// Default handler for unimplemented endpoints
pub(crate) async fn handle_not_implemented(
    State(state): State<AppState>,
    request: axum::http::Request<axum::body::Body>,
) -> Response {
    let path = request.uri().path().to_string();

    // Only Subsonic REST calls (/rest/*) should get a Subsonic-shaped response.
    // Anything else (e.g. a client probing a server-native endpoint like
    // /auth/login) must 404 so the client knows it's absent and falls back,
    // rather than seeing a 200 and assuming the endpoint exists.
    if !path.starts_with("/rest/") {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }

    let query_str = request.uri().query().unwrap_or("");
    let parsed = url::form_urlencoded::parse(query_str.as_bytes())
        .into_owned()
        .collect::<std::collections::HashMap<_, _>>();

    let auth_params = SubsonicParams {
        u: parsed.get("u").cloned().unwrap_or_default(),
        t: parsed.get("t").cloned(),
        s: parsed.get("s").cloned(),
        p: parsed.get("p").cloned(),
        ..Default::default()
    };

    let format = ResponseFormat::from_param(parsed.get("f").map(|s| s.as_str()));
    let callback = parsed.get("callback").map(|s| s.as_str());

    if resolve_user(&state, &auth_params).await.is_none() {
        return response::render(&xml_error(40, "Wrong username or password"), format, callback);
    }
    // Unknown /rest endpoint: report a Subsonic error (code 0) so clients don't
    // silently treat a bare "ok" as a successful (but empty) result.
    response::render(
        &xml_error(0, "Endpoint not supported by tidal-subsonic"),
        format,
        callback,
    )
}

/// Redact sensitive Subsonic auth params (token, salt, password) from a query
/// string before logging it.
fn redact_query(query: &str) -> String {
    query
        .split('&')
        .map(|pair| {
            let key = pair.split('=').next().unwrap_or("");
            match key {
                "t" | "s" | "p" | "token" | "salt" | "password" | "subsonic_password"
                | "subsonic_username" => format!("{}=<redacted>", key),
                _ => pair.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Log every incoming request (method, path, redacted query) and its response
/// status + latency at INFO level, so request activity is always visible.
pub(crate) async fn log_requests(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req
        .uri()
        .query()
        .map(redact_query)
        .filter(|q| !q.is_empty());

    // Establish the response format/callback context for the whole handler
    // execution so `ApiError`/`ApiOk` can render in the client-requested format.
    let (format, callback) = req
        .uri()
        .query()
        .map(|q| {
            let parsed = url::form_urlencoded::parse(q.as_bytes())
                .into_owned()
                .collect::<std::collections::HashMap<_, _>>();
            (
                ResponseFormat::from_param(parsed.get("f").map(|s| s.as_str())),
                parsed.get("callback").cloned(),
            )
        })
        .unwrap_or((ResponseFormat::Xml, None));

    let started = std::time::Instant::now();
    let response = RESPONSE_CTX.scope((format, callback), next.run(req)).await;
    let elapsed_ms = started.elapsed().as_millis();
    let status = response.status().as_u16();

    match query {
        Some(q) => tracing::info!("{} {}?{} -> {} ({} ms)", method, path, q, status, elapsed_ms),
        None => tracing::info!("{} {} -> {} ({} ms)", method, path, status, elapsed_ms),
    }
    response
}
