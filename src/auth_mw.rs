use crate::app::{AppState, SubsonicParams};
use crate::db::{self, User};
use crate::response::{self, ResponseFormat};
use crate::subsonic::{Payload, SubsonicResponse};
use crate::tidal::SharedTidalClient;
use axum::{
    extract::FromRequestParts,
    http::request::Parts,
    response::{IntoResponse, Response},
};

tokio::task_local! {
    /// The response format/callback the current request asked for. Set by the
    /// `log_requests` middleware before a handler runs so that `ApiError`/`ApiOk`
    /// `IntoResponse` can render in the client's format even though they never
    /// see the query params directly.
    pub(crate) static RESPONSE_CTX: (ResponseFormat, Option<String>);
}

/// Read the current request's response format + jsonp callback, falling back to
/// XML / no-callback if no context was established (should not happen for
/// handlers behind `Authed`).
pub(crate) fn current_response_ctx() -> (ResponseFormat, Option<String>) {
    RESPONSE_CTX
        .try_with(|ctx| ctx.clone())
        .unwrap_or((ResponseFormat::Xml, None))
}

/// Render a Subsonic response using the current request's response context.
pub(crate) fn render_current(resp: &SubsonicResponse) -> Response {
    let (format, callback) = current_response_ctx();
    response::render(resp, format, callback.as_deref())
}

/// Render a Subsonic response in the client-requested format (explicit params).
pub(crate) fn respond(resp: &SubsonicResponse, params: &SubsonicParams) -> Response {
    response::render(resp, params.format(), params.callback.as_deref())
}

/// Thin wrapper preserved for call sites: a failed response with an error.
pub(crate) fn xml_error(code: u32, message: &str) -> SubsonicResponse {
    SubsonicResponse::error(code, message)
}

/// Thin wrapper preserved for call sites: a successful response, no payload.
pub(crate) fn xml_ok() -> SubsonicResponse {
    SubsonicResponse::ok()
}

/// Verify a Subsonic user's credentials against their stored password (token
/// `t`+`s` or legacy `p=`). Returns the authenticated `User`, or None on any
/// mismatch / unknown user.
pub(crate) async fn resolve_user(state: &AppState, params: &SubsonicParams) -> Option<User> {
    if params.u.is_empty() {
        return None;
    }
    let user = db::find_user(&state.db, &state.cipher, &params.u).await?;
    if credentials_match(&user.password, params) {
        Some(user)
    } else {
        None
    }
}

/// Check a request's credentials against the given plaintext password.
pub(crate) fn credentials_match(password: &str, params: &SubsonicParams) -> bool {
    // Token auth: t = md5(password + salt).
    if let (Some(t), Some(s)) = (&params.t, &params.s) {
        let expected = format!("{:x}", md5::compute(format!("{}{}", password, s)));
        return t == &expected;
    }
    // Legacy password auth: p = password or "enc:<hex>".
    if let Some(p) = &params.p {
        return decode_password(p) == password;
    }
    false
}

/// Decode a Subsonic `p=` password, which may be hex-encoded and prefixed with
/// "enc:".
pub(crate) fn decode_password(p: &str) -> String {
    let Some(hex) = p.strip_prefix("enc:") else {
        return p.to_string();
    };
    let bytes: Option<Vec<u8>> = (0..hex.len())
        .step_by(2)
        .map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()))
        .collect();
    bytes
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_else(|| p.to_string())
}

/// Authenticated-request extractor. Running it verifies the Subsonic
/// credentials (token `t`+`s` or legacy `p=` password) exactly as `verify_auth`
/// did per-handler, and rejects with the Subsonic `Wrong username or password`
/// error (code 40) rendered in the client-requested format. On success it
/// exposes the `AppState`, the parsed query `params`, and helpers to reach the
/// TIDAL client and render responses.
pub(crate) struct Authed {
    pub(crate) state: AppState,
    pub(crate) params: SubsonicParams,
    pub(crate) user: User,
}

impl Authed {
    /// The authenticated user's TIDAL client, or `ApiError::NotAuthedTidal` if
    /// they haven't linked a TIDAL account yet.
    pub(crate) async fn tidal(&self) -> Result<SharedTidalClient, ApiError> {
        self.state
            .registry
            .get(self.user.id)
            .await
            .ok_or(ApiError::NotAuthedTidal)
    }

    /// The authenticated TIDAL user id for the current user's account.
    pub(crate) async fn tidal_user_id(&self) -> Result<u64, ApiError> {
        let client = self.tidal().await?;
        client.user_id().await.ok_or(ApiError::NotAuthedTidal)
    }
}

impl FromRequestParts<AppState> for Authed {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Parse manually so repeated params (songId=, albumId=) collect into Vecs.
        let params = SubsonicParams::from_query(parts.uri.query().unwrap_or(""));
        let user = resolve_user(state, &params).await.ok_or(ApiError::Auth)?;
        Ok(Authed {
            state: state.clone(),
            params,
            user,
        })
    }
}

/// A handler-level error that maps to a Subsonic error response. `IntoResponse`
/// renders it in the request's current response format (set by `Authed`).
pub(crate) enum ApiError {
    /// Subsonic auth failed: code 40, "Wrong username or password".
    Auth,
    /// The proxy has no TIDAL session yet: code 0.
    NotAuthedTidal,
    /// A client/request error (e.g. missing/invalid id): the given Subsonic code.
    BadRequest(u32, String),
    /// An upstream TIDAL error: code 0.
    Tidal(String),
}

impl ApiError {
    fn to_subsonic(&self) -> SubsonicResponse {
        match self {
            ApiError::Auth => SubsonicResponse::error(40, "Wrong username or password"),
            ApiError::NotAuthedTidal => {
                SubsonicResponse::error(0, "Not authenticated with TIDAL")
            }
            ApiError::BadRequest(code, msg) => SubsonicResponse::error(*code, msg),
            ApiError::Tidal(msg) => SubsonicResponse::error(0, msg),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        render_current(&self.to_subsonic())
    }
}

/// Successful Subsonic payload returned by a handler; `IntoResponse` renders it
/// (with `status="ok"`) in the request's current response format.
pub(crate) struct ApiOk(pub(crate) SubsonicResponse);

impl IntoResponse for ApiOk {
    fn into_response(self) -> Response {
        render_current(&self.0)
    }
}

impl From<Payload> for ApiOk {
    fn from(payload: Payload) -> Self {
        ApiOk(SubsonicResponse::ok_with(payload))
    }
}

/// The common handler result: a Subsonic payload/OK response, or an `ApiError`.
/// Both arms serialize via `response::render` in the requested format.
pub(crate) type ApiResult = Result<ApiOk, ApiError>;
