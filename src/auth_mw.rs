use crate::app::{AppState, SubsonicParams};
use crate::response::{self, ResponseFormat};
use crate::subsonic::{Payload, SubsonicResponse};
use crate::tidal::SharedTidalClient;
use axum::{
    extract::{FromRequestParts, Query},
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

pub(crate) fn verify_auth(state: &AppState, params: &SubsonicParams) -> bool {
    if params.u != state.subsonic_username {
        return false;
    }
    // Token auth: t = md5(password + salt).
    if let (Some(t), Some(s)) = (&params.t, &params.s) {
        let expected = format!("{:x}", md5::compute(format!("{}{}", state.subsonic_password, s)));
        return t == &expected;
    }
    // Legacy password auth: p = password or "enc:<hex>".
    if let Some(p) = &params.p {
        let plaintext = decode_password(p);
        return plaintext == state.subsonic_password;
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
}

impl Authed {
    /// The shared TIDAL client for this request.
    pub(crate) fn tidal(&self) -> &SharedTidalClient {
        &self.state.tidal
    }

    /// The authenticated TIDAL user id, or `ApiError::NotAuthedTidal` if the
    /// proxy has no TIDAL session yet.
    pub(crate) async fn tidal_user_id(&self) -> Result<u64, ApiError> {
        self.state
            .tidal
            .user_id()
            .await
            .ok_or(ApiError::NotAuthedTidal)
    }
}

#[axum::async_trait]
impl FromRequestParts<AppState> for Authed {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Query(params) = Query::<SubsonicParams>::from_request_parts(parts, state)
            .await
            .unwrap_or_else(|_| Query(SubsonicParams::default()));
        if !verify_auth(state, &params) {
            return Err(ApiError::Auth);
        }
        Ok(Authed {
            state: state.clone(),
            params,
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
