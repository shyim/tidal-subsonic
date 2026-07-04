//! Library surface for `tidal-subsonic`: the module tree plus the router
//! assembly, factored out of `main.rs` so integration tests can build the same
//! app and drive it in-process.

pub mod app;
pub mod auth;
pub mod auth_mw;
pub mod crypto;
pub mod db;
pub mod item_id;
pub mod mapping;
pub mod response;
pub mod routes;
pub mod subsonic;
pub mod tidal;

use app::AppState;
use axum::routing::get;
use axum::Router;
use db::SharedDb;
use routes::{
    browsing, discovery, fallback, lists, lyrics, media, playlists, queue, search, system, users,
};
use std::collections::HashMap;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

/// Assemble the shared `AppState` from an already-open database and its config.
/// Used by `main()` and by integration tests (which pass a temp DB).
pub async fn build_state(db: SharedDb, cipher: crypto::Cipher, cfg: &db::DbConfig) -> AppState {
    let registry = tidal::ClientRegistry::new(
        db.clone(),
        cipher.clone(),
        cfg.tidal_client_id.clone(),
        cfg.tidal_client_secret.clone(),
    );
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("build http client");

    AppState {
        registry,
        db,
        cipher,
        http_client,
        pkce_sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        max_quality: cfg.tidal_max_quality.clone(),
        media_cache: routes::media_cache::MediaCache::open(),
        metadata_cache: routes::metadata_cache::MetadataCache::new(),
        web_sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
    }
}

/// Register a Subsonic endpoint under both the bare `/rest/<name>` path and the
/// legacy `/rest/<name>.view` alias (older clients append `.view`), so each
/// endpoint only has to be listed once.
fn rest(
    router: Router<AppState>,
    name: &str,
    handler: axum::routing::MethodRouter<AppState>,
) -> Router<AppState> {
    router
        .route(&format!("/rest/{}", name), handler.clone())
        .route(&format!("/rest/{}.view", name), handler)
}

/// Build the full application router (portal `/api/*` + Subsonic `/rest/*` + SPA
/// fallback + middleware), ready to serve. Shared by `main()` and the tests.
pub fn build_router(state: AppState) -> Router {
    // Web portal: the /api/* JSON surface. The embedded SPA is served by the
    // final fallback for any non-/api, non-/rest route.
    let mut app = routes::portal::api_routes();
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
        ("getTopSongs", get(discovery::handle_get_top_songs)),
        ("getSimilarSongs", get(discovery::handle_get_similar_songs)),
        ("getSimilarSongs2", get(discovery::handle_get_similar_songs2)),
        ("getArtistInfo", get(discovery::handle_get_artist_info)),
        ("getArtistInfo2", get(discovery::handle_get_artist_info2)),
        ("getAlbumInfo", get(discovery::handle_get_album_info)),
        ("getAlbumInfo2", get(discovery::handle_get_album_info)),
        ("getLyrics", get(lyrics::handle_get_lyrics)),
        ("getLyricsBySongId", get(lyrics::handle_get_lyrics_by_song_id)),
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
        ("download", get(media::handle_download)),
        ("scrobble", get(system::handle_scrobble)),
        ("getUser", get(system::handle_get_user)),
        ("getUsers", get(users::handle_get_users)),
        ("createUser", get(users::handle_create_user)),
        ("updateUser", get(users::handle_update_user)),
        ("deleteUser", get(users::handle_delete_user)),
        ("changePassword", get(users::handle_change_password)),
        ("getScanStatus", get(system::handle_get_scan_status)),
        ("startScan", get(system::handle_start_scan)),
        ("getGenres", get(system::handle_get_genres)),
        ("getNowPlaying", get(system::handle_get_now_playing)),
        ("savePlayQueue", get(queue::handle_save_play_queue)),
        ("getPlayQueue", get(queue::handle_get_play_queue)),
        ("createBookmark", get(queue::handle_create_bookmark)),
        ("getBookmarks", get(queue::handle_get_bookmarks)),
        ("deleteBookmark", get(queue::handle_delete_bookmark)),
        (
            "getOpenSubsonicExtensions",
            get(system::handle_get_open_subsonic_extensions),
        ),
        ("getAvatar", get(system::handle_get_avatar)),
    ] {
        app = rest(app, name, handler);
    }

    app.fallback(fallback::handle_not_implemented)
        .layer(axum::middleware::from_fn(fallback::log_requests))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
