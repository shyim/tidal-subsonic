//! Web portal: a JSON `/api/*` surface + browser sessions backing the embedded
//! SPA. Sessions are cookie tokens mapped to an in-memory table (no new deps);
//! the cookie is HttpOnly + SameSite=Strict. Admin routes re-check `is_admin`
//! server-side on every call.

use crate::app::{AppState, WebSession};
use crate::{auth, db};
use axum::{
    extract::{FromRequestParts, Path, State},
    http::{header, request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;

const SESSION_COOKIE: &str = "tsub_session";

pub(crate) fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/link/start", post(link_start))
        .route("/api/link/complete", post(link_complete))
        .route("/api/unlink", post(unlink))
        .route("/api/password", post(change_password))
        .route("/api/users", get(list_users).post(create_user))
        .route("/api/users/:name", post(update_user).delete(delete_user))
}

// ---- session plumbing ----

fn random_token() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 24] = rng.gen();
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn session_token_from_cookies(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie.split(';').find_map(|part| {
        let (k, v) = part.trim().split_once('=')?;
        (k == SESSION_COOKIE).then(|| v.to_string())
    })
}

/// A `Set-Cookie` header value for a fresh session (HttpOnly, SameSite=Strict).
/// `Secure` is included when the request came in over HTTPS.
fn set_cookie(token: &str, secure: bool) -> String {
    let mut c = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=604800",
        SESSION_COOKIE, token
    );
    if secure {
        c.push_str("; Secure");
    }
    c
}

fn clear_cookie() -> String {
    format!(
        "{}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0",
        SESSION_COOKIE
    )
}

fn is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        == Some("https")
}

/// Extractor: the authenticated portal session, or 401.
struct Session(WebSession);

#[axum::async_trait]
impl FromRequestParts<AppState> for Session {
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        let token = session_token_from_cookies(&parts.headers)
            .ok_or_else(|| json_err(StatusCode::UNAUTHORIZED, "Not signed in"))?;
        let sess = state.web_sessions.lock().await.get(&token).cloned();
        sess.map(Session)
            .ok_or_else(|| json_err(StatusCode::UNAUTHORIZED, "Session expired"))
    }
}

fn json_err(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({ "error": msg }))).into_response()
}

// ---- auth ----

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Response {
    let user = db::find_user(&state.db, &state.cipher, &body.username).await;
    let user = match user {
        Some(u) if !body.password.is_empty() && u.password == body.password => u,
        _ => return json_err(StatusCode::UNAUTHORIZED, "Wrong username or password"),
    };

    let token = random_token();
    state.web_sessions.lock().await.insert(
        token.clone(),
        WebSession {
            user_id: user.id,
            username: user.username.clone(),
            is_admin: user.is_admin,
        },
    );

    (
        [(header::SET_COOKIE, set_cookie(&token, is_https(&headers)))],
        Json(json!({ "username": user.username, "isAdmin": user.is_admin })),
    )
        .into_response()
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = session_token_from_cookies(&headers) {
        state.web_sessions.lock().await.remove(&token);
    }
    ([(header::SET_COOKIE, clear_cookie())], Json(json!({ "ok": true }))).into_response()
}

async fn me(State(state): State<AppState>, headers: HeaderMap, Session(s): Session) -> Response {
    let linked = db::load_tidal_account(&state.db, &state.cipher, s.user_id)
        .await
        .map(|a| !a.access_token.is_empty())
        .unwrap_or(false);
    let base_url = crate::app::base_url_from_headers(&headers);
    Json(json!({
        "username": s.username,
        "isAdmin": s.is_admin,
        "tidalLinked": linked,
        "serverUrl": base_url,
    }))
    .into_response()
}

// ---- TIDAL linking ----

async fn link_start(State(state): State<AppState>, Session(s): Session) -> Response {
    let start = auth::start_link(&state, s.user_id).await;
    Json(json!({
        "authorizeUrl": start.authorize_url,
        "key": start.client_unique_key,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct LinkCompleteBody {
    code: String,
    key: String,
}

async fn link_complete(
    State(state): State<AppState>,
    Session(s): Session,
    Json(body): Json<LinkCompleteBody>,
) -> Response {
    // Bind completion to the logged-in user — you can only finish your own link.
    match auth::complete_link(&state, &body.code, &body.key, s.user_id).await {
        Ok(_) => Json(json!({ "tidalLinked": true })).into_response(),
        Err(e) => json_err(StatusCode::BAD_REQUEST, &e),
    }
}

async fn unlink(State(state): State<AppState>, Session(s): Session) -> Response {
    if let Err(e) = db::delete_tidal_account(&state.db, s.user_id).await {
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    state.registry.invalidate(s.user_id).await;
    Json(json!({ "tidalLinked": false })).into_response()
}

// ---- self-service password ----

#[derive(Deserialize)]
struct PasswordBody {
    #[serde(rename = "newPassword")]
    new_password: String,
}

async fn change_password(
    State(state): State<AppState>,
    Session(s): Session,
    Json(body): Json<PasswordBody>,
) -> Response {
    if body.new_password.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "Password cannot be empty");
    }
    match db::update_user(
        &state.db,
        &state.cipher,
        &s.username,
        Some(&body.new_password),
        None,
    )
    .await
    {
        Some(_) => Json(json!({ "ok": true })).into_response(),
        None => json_err(StatusCode::NOT_FOUND, "User not found"),
    }
}

// ---- admin: users ----

#[derive(Serialize)]
struct UserView {
    username: String,
    #[serde(rename = "isAdmin")]
    is_admin: bool,
    #[serde(rename = "tidalLinked")]
    tidal_linked: bool,
}

fn require_admin(s: &WebSession) -> Result<(), Response> {
    if s.is_admin {
        Ok(())
    } else {
        Err(json_err(StatusCode::FORBIDDEN, "Admin only"))
    }
}

/// Drop all portal sessions belonging to `username`, so a password change,
/// privilege change, or deletion takes effect immediately (existing sessions
/// carry a cached `is_admin`, so they must be re-established).
async fn invalidate_user_sessions(state: &AppState, username: &str) {
    state
        .web_sessions
        .lock()
        .await
        .retain(|_, sess| sess.username != username);
}

async fn list_users(State(state): State<AppState>, Session(s): Session) -> Response {
    if let Err(r) = require_admin(&s) {
        return r;
    }
    let users = db::list_users(&state.db, &state.cipher).await;
    let mut out = Vec::with_capacity(users.len());
    for u in &users {
        let linked = db::load_tidal_account(&state.db, &state.cipher, u.id)
            .await
            .map(|a| !a.access_token.is_empty())
            .unwrap_or(false);
        out.push(UserView {
            username: u.username.clone(),
            is_admin: u.is_admin,
            tidal_linked: linked,
        });
    }
    Json(out).into_response()
}

#[derive(Deserialize)]
struct CreateUserBody {
    username: String,
    password: String,
    #[serde(default, rename = "isAdmin")]
    is_admin: bool,
}

async fn create_user(
    State(state): State<AppState>,
    Session(s): Session,
    Json(body): Json<CreateUserBody>,
) -> Response {
    if let Err(r) = require_admin(&s) {
        return r;
    }
    if body.username.is_empty() || body.password.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "Username and password required");
    }
    if db::find_user(&state.db, &state.cipher, &body.username)
        .await
        .is_some()
    {
        return json_err(StatusCode::CONFLICT, "User already exists");
    }
    match db::create_user(
        &state.db,
        &state.cipher,
        &body.username,
        &body.password,
        body.is_admin,
    )
    .await
    {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => json_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize)]
struct UpdateUserBody {
    #[serde(default)]
    password: Option<String>,
    #[serde(default, rename = "isAdmin")]
    is_admin: Option<bool>,
}

async fn update_user(
    State(state): State<AppState>,
    Session(s): Session,
    Path(name): Path<String>,
    Json(body): Json<UpdateUserBody>,
) -> Response {
    if let Err(r) = require_admin(&s) {
        return r;
    }
    let pw = body.password.as_deref().filter(|p| !p.is_empty());
    match db::update_user(&state.db, &state.cipher, &name, pw, body.is_admin).await {
        Some(_) => {
            // A password or privilege change must not leave stale sessions with
            // the old credentials/role active.
            invalidate_user_sessions(&state, &name).await;
            Json(json!({ "ok": true })).into_response()
        }
        None => json_err(StatusCode::NOT_FOUND, "User not found"),
    }
}

async fn delete_user(
    State(state): State<AppState>,
    Session(s): Session,
    Path(name): Path<String>,
) -> Response {
    if let Err(r) = require_admin(&s) {
        return r;
    }
    if name == s.username {
        return json_err(StatusCode::BAD_REQUEST, "You can't delete your own account");
    }
    match db::delete_user(&state.db, &name).await {
        Some(id) => {
            state.registry.invalidate(id).await;
            invalidate_user_sessions(&state, &name).await;
            Json(json!({ "ok": true })).into_response()
        }
        None => json_err(StatusCode::NOT_FOUND, "User not found"),
    }
}
