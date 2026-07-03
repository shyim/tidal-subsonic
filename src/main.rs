mod app;
mod auth;
mod auth_mw;
mod db;
mod item_id;
mod mapping;
mod response;
mod routes;
mod subsonic;
mod tidal;

use app::AppState;
use axum::routing::get;
use reqwest::Client as ReqwestClient;
use routes::{browsing, fallback, lists, media, playlists, search, system};
use std::collections::HashMap;
use std::sync::Arc;
use tidal::{SharedTidalClient, TidalClient};
use tower_http::trace::TraceLayer;

/// Register a Subsonic endpoint under both the bare `/rest/<name>` path and the
/// legacy `/rest/<name>.view` alias (older clients append `.view`), so each
/// endpoint only has to be listed once.
fn rest(
    router: axum::Router<AppState>,
    name: &str,
    handler: axum::routing::MethodRouter<AppState>,
) -> axum::Router<AppState> {
    router
        .route(&format!("/rest/{}", name), handler.clone())
        .route(&format!("/rest/{}.view", name), handler)
}

#[tokio::main]
async fn main() {
    // Default to INFO for this crate (so request logs always show) plus WARN
    // elsewhere; RUST_LOG overrides everything when set.
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tidal_subsonic=info,tower_http=warn"));
    fmt().with_env_filter(filter).init();

    // Open or create SQLite database
    let db_path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("tidal-subsonic")
        .join("tidal-subsonic.db");

    let db = db::open_db(&db_path).expect("Failed to open database");
    let cfg = db::load_config(&db).await;

    // Create TidalClient with DB-backed tokens
    let tidal: SharedTidalClient = Arc::new(TidalClient::from_db_config(&cfg, db.clone()));

    // If we have tokens but no user_id, try to get session info
    if tidal.is_authenticated().await && tidal.user_id().await.is_none() {
        tracing::info!("Authenticated, fetching session info...");
        // get_session_info persists the discovered user_id / country_code itself.
        match tidal.get_session_info().await {
            Ok(uid) => {
                tracing::info!("Logged in as user {}", uid);
            }
            Err(e) => {
                tracing::warn!("Failed to get session info: {} - will try again on first API call", e);
            }
        }
    }

    let http_client = ReqwestClient::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();

    let subsonic_username = cfg.subsonic_username.clone();
    let subsonic_password = cfg.subsonic_password.clone();
    let max_quality = cfg.tidal_max_quality.clone();
    let host = cfg.server_host.clone();
    let port = cfg.server_port;

    let state = AppState {
        tidal: tidal.clone(),
        db: db.clone(),
        http_client,
        pkce_sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        subsonic_username: subsonic_username.clone(),
        subsonic_password: subsonic_password.clone(),
        max_quality: max_quality.clone(),
    };

    let mut app = auth::auth_routes();
    // Each endpoint is registered under both `/rest/<name>` and its `.view`
    // alias via `rest`, so the list below stays one line per endpoint.
    for (name, handler) in [
        ("ping", get(system::handle_ping)),
        ("getLicense", get(system::handle_get_license)),
        ("getMusicFolders", get(system::handle_get_music_folders)),
        ("getIndexes", get(browsing::handle_get_indexes)),
        ("getArtists", get(browsing::handle_get_artists)),
        ("getArtist", get(browsing::handle_get_artist)),
        ("getMusicDirectory", get(browsing::handle_get_music_directory)),
        ("getAlbum", get(browsing::handle_get_album)),
        ("getSong", get(browsing::handle_get_song)),
        ("getRandomSongs", get(lists::handle_get_random_songs)),
        ("getAlbumList", get(lists::handle_get_album_list)),
        ("getAlbumList2", get(lists::handle_get_album_list2)),
        ("getStarred", get(lists::handle_get_starred)),
        ("getStarred2", get(lists::handle_get_starred)),
        ("star", get(lists::handle_star)),
        ("unstar", get(lists::handle_unstar)),
        ("search2", get(search::handle_search2)),
        ("search3", get(search::handle_search3)),
        ("getPlaylists", get(playlists::handle_get_playlists)),
        ("getPlaylist", get(playlists::handle_get_playlist)),
        ("createPlaylist", get(playlists::handle_create_playlist)),
        ("updatePlaylist", get(playlists::handle_update_playlist)),
        ("deletePlaylist", get(playlists::handle_delete_playlist)),
        ("getCoverArt", get(media::handle_get_cover_art)),
        ("stream", get(media::handle_stream)),
        ("scrobble", get(system::handle_scrobble)),
        ("getUser", get(system::handle_get_user)),
        ("getScanStatus", get(system::handle_get_scan_status)),
        ("startScan", get(system::handle_start_scan)),
        ("getGenres", get(system::handle_get_genres)),
        ("getNowPlaying", get(system::handle_get_now_playing)),
        (
            "getOpenSubsonicExtensions",
            get(system::handle_get_open_subsonic_extensions),
        ),
        ("getAvatar", get(system::handle_get_avatar)),
    ] {
        app = rest(app, name, handler);
    }

    let app = app
        .fallback(fallback::handle_not_implemented)
        .layer(axum::middleware::from_fn(fallback::log_requests))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = format!("{}:{}", host, port);
    tracing::info!("Starting tidal-subsonic server on {}", addr);
    tracing::info!("Username: '{}', Password: '{}'", subsonic_username, subsonic_password);

    let auth_check = db::is_authenticated(&db).await;
    if !auth_check {
        tracing::warn!("Not authenticated with TIDAL. Open http://{}:{}/ to set up.", host, port);
    } else {
        tracing::info!("TIDAL authenticated. Subsonic API ready.");
    }

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
