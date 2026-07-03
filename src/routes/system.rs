use crate::app::{AppState, SubsonicParams};
use crate::auth_mw::{respond, verify_auth, xml_ok, ApiOk, ApiResult, Authed};
use crate::subsonic::*;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub(crate) async fn handle_ping(_authed: Authed) -> ApiResult {
    Ok(ApiOk(xml_ok()))
}

pub(crate) async fn handle_get_license(_authed: Authed) -> ApiResult {
    Ok(Payload::License(License {
        valid: true,
        email: None,
        license_expires: None,
        trial_expires: None,
    })
    .into())
}

pub(crate) async fn handle_get_music_folders(_authed: Authed) -> ApiResult {
    Ok(Payload::MusicFolders(MusicFolders {
        music_folder: vec![MusicFolder {
            id: 1,
            name: "TIDAL".to_string(),
        }],
    })
    .into())
}

pub(crate) async fn handle_scrobble(_authed: Authed) -> ApiResult {
    // Scrobbling is not implemented - just acknowledge
    Ok(ApiOk(xml_ok()))
}

pub(crate) async fn handle_get_user(authed: Authed) -> ApiResult {
    Ok(Payload::User(SubsonicUser {
        username: authed.state.subsonic_username.clone(),
        email: None,
        scrobbling_enabled: Some(false),
        admin_role: Some(true),
        settings_role: Some(true),
        download_role: Some(true),
        upload_role: Some(false),
        playlist_role: Some(true),
        cover_art_role: Some(false),
        comment_role: Some(false),
        podcast_role: Some(false),
        stream_role: Some(true),
        jukebox_role: Some(true),
        share_role: Some(false),
        video_conversion_role: Some(false),
        avatar_last_changed: None,
        folder: None,
    })
    .into())
}

pub(crate) async fn handle_get_scan_status(_authed: Authed) -> ApiResult {
    // There is no local library to scan — the proxy is always "done scanning".
    Ok(Payload::ScanStatus(ScanStatus {
        scanning: false,
        count: Some(0),
    })
    .into())
}

/// `startScan` on a live proxy has nothing to scan, so we immediately report a
/// finished scan. Clients (e.g. Submariner) poll getScanStatus afterwards and
/// need to see `scanning=false` to consider the scan complete.
pub(crate) async fn handle_start_scan(_authed: Authed) -> ApiResult {
    Ok(Payload::ScanStatus(ScanStatus {
        scanning: false,
        count: Some(0),
    })
    .into())
}

/// We don't track cross-client playback, so report an empty now-playing list.
pub(crate) async fn handle_get_now_playing(_authed: Authed) -> ApiResult {
    Ok(Payload::NowPlaying(NowPlaying { entry: vec![] }).into())
}

pub(crate) async fn handle_get_genres(_authed: Authed) -> ApiResult {
    Ok(Payload::Genres(GenresWrapper { genre: vec![] }).into())
}

pub(crate) async fn handle_get_open_subsonic_extensions(
    Query(params): Query<SubsonicParams>,
) -> Response {
    // Per the OpenSubsonic spec this discovery endpoint does not require auth.
    // We advertise no optional extensions (empty list) — enough for clients to
    // recognise us as OpenSubsonic-aware without promising unimplemented ones.
    let resp = SubsonicResponse::ok_with(Payload::OpenSubsonicExtensions(vec![]));
    respond(&resp, &params)
}

pub(crate) async fn handle_get_avatar(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    // Avatar keeps its own auth check: on failure it returns a plain HTTP 401
    // (not a Subsonic-shaped body), so it must not use the `Authed` extractor.
    if !verify_auth(&state, &params) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    // Return a 1x1 transparent PNG as default avatar
    let png: [u8; 67] = [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
        0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
        0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
        0x42, 0x60, 0x82,
    ];
    (StatusCode::OK, [("content-type", "image/png")], png.to_vec()).into_response()
}
