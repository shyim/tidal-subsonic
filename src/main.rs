mod auth;
mod db;
mod mapping;
mod response;
mod subsonic;
mod tidal_client;

use auth::PkceSession;
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use db::SharedDb;
use reqwest::Client as ReqwestClient;
use response::ResponseFormat;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use subsonic::*;
use tidal_client::*;
use tower_http::trace::TraceLayer;

const API_VERSION: &str = "1.16.1";
const SERVER_NAME: &str = "tidal-subsonic";

#[derive(Clone)]
struct AppState {
    tidal: SharedTidalClient,
    db: SharedDb,
    http_client: ReqwestClient,
    pkce_sessions: Arc<tokio::sync::Mutex<HashMap<String, PkceSession>>>,
    subsonic_password: String,
    subsonic_username: String,
    max_quality: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct SubsonicParams {
    #[serde(default)]
    u: String,          // username
    #[serde(default)]
    t: Option<String>,  // token = md5(password + salt)
    #[serde(default)]
    s: Option<String>,  // salt (random)
    #[serde(default)]
    p: Option<String>,  // plaintext or "enc:"-hex password (legacy clients)
    #[serde(default)]
    v: Option<String>,  // API version
    #[serde(default)]
    c: Option<String>,  // client name
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    #[serde(rename = "type")]
    list_type: Option<String>,
    #[serde(default)]
    size: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    #[serde(rename = "artistCount")]
    artist_count: Option<u32>,
    #[serde(default)]
    #[serde(rename = "artistOffset")]
    artist_offset: Option<u32>,
    #[serde(default)]
    #[serde(rename = "albumCount")]
    album_count: Option<u32>,
    #[serde(default)]
    #[serde(rename = "albumOffset")]
    album_offset: Option<u32>,
    #[serde(default)]
    #[serde(rename = "songCount")]
    song_count: Option<u32>,
    #[serde(default)]
    #[serde(rename = "songOffset")]
    song_offset: Option<u32>,
    #[serde(default)]
    #[serde(rename = "musicFolderId")]
    music_folder_id: Option<String>,
    #[serde(default)]
    #[serde(rename = "fromYear")]
    from_year: Option<String>,
    #[serde(default)]
    #[serde(rename = "toYear")]
    to_year: Option<String>,
    #[serde(default)]
    genre: Option<String>,
    #[serde(default)]
    #[serde(rename = "minBitRate")]
    min_bit_rate: Option<u32>,
    #[serde(default)]
    #[serde(rename = "maxBitRate")]
    max_bit_rate: Option<u32>,
    #[serde(default)]
    #[serde(rename = "ifModifiedSince")]
    if_modified_since: Option<u64>,
    #[serde(default)]
    #[serde(rename = "newestMethod")]
    newest_method: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    #[serde(rename = "songId")]
    song_id: Option<Vec<String>>,
    #[serde(default)]
    #[serde(rename = "albumId")]
    album_id: Option<Vec<String>>,
    #[serde(default)]
    #[serde(rename = "artistId")]
    artist_id_str: Option<String>,
    #[serde(default)]
    submission: Option<bool>,
    #[serde(default)]
    time: Option<u64>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    f: Option<String>,
    #[serde(default)]
    callback: Option<String>,
    #[serde(default)]
    format: Option<String>,
}

impl SubsonicParams {
    fn format(&self) -> ResponseFormat {
        ResponseFormat::from_param(self.f.as_deref())
    }
}

fn verify_auth(state: &AppState, params: &SubsonicParams) -> bool {
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
fn decode_password(p: &str) -> String {
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

/// Render a Subsonic response in the client-requested format.
fn respond(resp: &SubsonicResponse, params: &SubsonicParams) -> Response {
    response::render(resp, params.format(), params.callback.as_deref())
}

fn xml_error(code: u32, message: &str) -> SubsonicResponse {
    SubsonicResponse {
        xmlns: "http://subsonic.org/restapi".to_string(),
        status: "failed".to_string(),
        version: API_VERSION.to_string(),
        server_type: SERVER_NAME.to_string(),
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        open_subsonic: true,
        error: Some(SubsonicError {
            code,
            message: message.to_string(),
        }),
        license: None,
        music_folders: None,
        indexes: None,
        artists: None,
        artist: None,
        album: None,
        song: None,
        album_list: None,
        album_list2: None,
        random_songs: None,
        songs_by_genre: None,
        now_playing: None,
        starred: None,
        starred2: None,
        search_result: None,
        search_result2: None,
        search_result3: None,
        playlists: None,
        playlist: None,
        user: None,
        scan_status: None,
        genres: None,
        open_subsonic_extensions: None,
        directory: None,
    }
}

fn xml_ok() -> SubsonicResponse {
    SubsonicResponse {
        xmlns: "http://subsonic.org/restapi".to_string(),
        status: "ok".to_string(),
        version: API_VERSION.to_string(),
        server_type: SERVER_NAME.to_string(),
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        open_subsonic: true,
        error: None,
        license: None,
        music_folders: None,
        indexes: None,
        artists: None,
        artist: None,
        album: None,
        song: None,
        album_list: None,
        album_list2: None,
        random_songs: None,
        songs_by_genre: None,
        now_playing: None,
        starred: None,
        starred2: None,
        search_result: None,
        search_result2: None,
        search_result3: None,
        playlists: None,
        playlist: None,
        user: None,
        scan_status: None,
        genres: None,
        open_subsonic_extensions: None,
        directory: None,
    }
}

fn base_url_from_headers(headers: &HeaderMap) -> String {
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:4533");
    let proto = if headers.get("x-forwarded-proto").and_then(|h| h.to_str().ok()) == Some("https") {
        "https"
    } else {
        "http"
    };
    format!("{}://{}", proto, host)
}

async fn handle_ping(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    _headers: HeaderMap,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let resp = xml_ok();
    respond(&resp, &params)
}

async fn handle_get_license(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    resp.license = Some(License {
        valid: true,
        email: None,
        license_expires: None,
        trial_expires: None,
    });
    respond(&resp, &params)
}

async fn handle_get_music_folders(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    resp.music_folders = Some(MusicFolders {
        music_folder: vec![MusicFolder {
            id: 1,
            name: "TIDAL".to_string(),
        }],
    });
    respond(&resp, &params)
}

async fn handle_get_indexes(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let mut client = state.tidal.lock().await;
    let user_id = match client.user_id() {
        Some(id) => id,
        None => {
            return respond(&xml_error(0, "Not authenticated with TIDAL"), &params);
        }
    };
    match client.get_favorite_artists(user_id, 0, 500).await {
        Ok(artists) => {
            resp.indexes = Some(mapping::build_indexes(&artists.items));
        }
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    respond(&resp, &params)
}

async fn handle_get_artists(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let mut client = state.tidal.lock().await;
    let user_id = match client.user_id() {
        Some(id) => id,
        None => {
            return respond(&xml_error(0, "Not authenticated with TIDAL"), &params);
        }
    };
    match client.get_favorite_artists(user_id, 0, 500).await {
        Ok(artists) => {
            let indexes = mapping::build_indexes(&artists.items);
            resp.artists = Some(ArtistsList {
                ignored_articles: indexes.ignored_articles,
                index: indexes.index,
            });
        }
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    respond(&resp, &params)
}

async fn handle_get_artist(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let artist_id_str = match &params.id {
        Some(id) => id.clone(),
        None => return respond(&xml_error(10, "Missing artist id"), &params),
    };
    let artist_id: u64 = match artist_id_str.strip_prefix("ar-").and_then(|s| s.parse().ok()) {
        Some(id) => id,
        None => return respond(&xml_error(0, "Invalid artist id"), &params),
    };

    let mut client = state.tidal.lock().await;
    match client.get_artist_detail(artist_id).await {
        Ok(artist_detail) => {
            let sub_artist = mapping::artist_to_subsonic(&artist_detail);
            let mut sub_albums = Vec::new();
            if let Ok(albums) = client.get_artist_albums(artist_id, 0, 100).await {
                sub_albums = albums.items.iter().map(|a| mapping::album_to_subsonic(a)).collect();
            }
            resp.artist = Some(ArtistWithAlbums {
                id: sub_artist.id,
                name: sub_artist.name,
                cover_art: sub_artist.cover_art,
                album_count: Some(sub_albums.len() as u32),
                artist_image_url: sub_artist.artist_image_url,
                starred: None,
                album: sub_albums,
            });
        }
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    respond(&resp, &params)
}

async fn handle_get_album(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let album_id_str = match &params.id {
        Some(id) => id.clone(),
        None => return respond(&xml_error(10, "Missing album id"), &params),
    };
    let album_id: u64 = match album_id_str.strip_prefix("al-").and_then(|s| s.parse().ok()) {
        Some(id) => id,
        None => return respond(&xml_error(0, "Invalid album id"), &params),
    };

    let mut client = state.tidal.lock().await;
    let base_url = base_url_from_headers(&headers);
    match client.get_album_detail(album_id).await {
        Ok(album) => {
            match client.get_album_tracks(album_id, 0, 200).await {
                Ok(tracks) => {
                    resp.album = Some(mapping::album_detail_to_album_with_songs(
                        &album, &tracks.items, &base_url,
                    ));
                }
                Err(_e) => {
                    // Still return album without tracks
                    resp.album = Some(mapping::album_detail_to_album_with_songs(
                        &album, &[], &base_url,
                    ));
                }
            }
        }
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    respond(&resp, &params)
}

/// Classic browsing endpoint used by older clients (e.g. Submariner). An
/// artist id (`ar-`) lists its albums as sub-directories; an album id (`al-`)
/// lists its tracks as songs — matching what getArtist / getAlbum return but in
/// the directory shape those clients navigate with.
async fn handle_get_music_directory(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let dir_id = match &params.id {
        Some(id) => id.clone(),
        None => return respond(&xml_error(10, "Missing directory id"), &params),
    };
    let base_url = base_url_from_headers(&headers);
    let mut client = state.tidal.lock().await;

    if let Some(artist_id) = dir_id.strip_prefix("ar-").and_then(|s| s.parse::<u64>().ok()) {
        match client.get_artist_detail(artist_id).await {
            Ok(artist) => {
                let mut children = Vec::new();
                if let Ok(albums) = client.get_artist_albums(artist_id, 0, 100).await {
                    children = albums
                        .items
                        .iter()
                        .map(|a| mapping::album_to_directory_child(a, &dir_id))
                        .collect();
                }
                resp.directory = Some(Directory {
                    id: dir_id.clone(),
                    name: artist.name,
                    parent: None,
                    play_count: None,
                    child: children,
                });
            }
            Err(e) => {
                resp.error = Some(SubsonicError { code: 0, message: e });
                resp.status = "failed".to_string();
            }
        }
    } else if let Some(album_id) = dir_id.strip_prefix("al-").and_then(|s| s.parse::<u64>().ok()) {
        match client.get_album_detail(album_id).await {
            Ok(album) => {
                let tracks = client
                    .get_album_tracks(album_id, 0, 200)
                    .await
                    .map(|t| t.items)
                    .unwrap_or_default();
                let children = tracks
                    .iter()
                    .map(|t| mapping::track_to_child(t, &base_url))
                    .collect();
                let artist_parent = album
                    .artist
                    .as_ref()
                    .map(|a| format!("ar-{}", a.id))
                    .or_else(|| {
                        album
                            .artists
                            .as_ref()
                            .and_then(|a| a.first())
                            .map(|a| format!("ar-{}", a.id))
                    });
                resp.directory = Some(Directory {
                    id: dir_id.clone(),
                    name: album.title.clone(),
                    parent: artist_parent,
                    play_count: None,
                    child: children,
                });
            }
            Err(e) => {
                resp.error = Some(SubsonicError { code: 0, message: e });
                resp.status = "failed".to_string();
            }
        }
    } else {
        return respond(&xml_error(70, "Directory not found"), &params);
    }

    respond(&resp, &params)
}

async fn handle_get_song(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let track_id_str = match &params.id {
        Some(id) => id.clone(),
        None => return respond(&xml_error(10, "Missing track id"), &params),
    };
    let track_id: u64 = match track_id_str.strip_prefix("tr-").and_then(|s| s.parse().ok()) {
        Some(id) => id,
        None => return respond(&xml_error(0, "Invalid track id"), &params),
    };

    let mut client = state.tidal.lock().await;
    match client.get_track(track_id).await {
        Ok(track) => {
            let base_url = base_url_from_headers(&headers);
            resp.song = Some(mapping::track_to_child(&track, &base_url));
        }
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    respond(&resp, &params)
}

async fn handle_get_random_songs(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let size = params.size.unwrap_or(10).min(50);
    let mut client = state.tidal.lock().await;
    let user_id = match client.user_id() {
        Some(id) => id,
        None => {
            return respond(&xml_error(0, "Not authenticated with TIDAL"), &params);
        }
    };
    match client.get_favorite_tracks(user_id, 0, 200).await {
        Ok(tracks) => {
            // Simple "random" - just return the first N tracks (or use search for random)
            let base_url = base_url_from_headers(&headers);
            let count = size.min(tracks.items.len() as u32);
            let songs: Vec<SubsonicChild> = tracks.items[..count as usize]
                .iter()
                .map(|t| mapping::track_to_child(t, &base_url))
                .collect();
            resp.random_songs = Some(SongList { song: songs });
        }
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    respond(&resp, &params)
}

/// Fetch and order the album list for the given `type`, backed by the user's
/// TIDAL favorites (the only broad album source available via the proxy).
async fn fetch_album_list(
    client: &mut TidalClient,
    list_type: &str,
    size: u32,
    offset: u32,
) -> Result<Vec<SubsonicAlbum>, String> {
    let user_id = client.user_id().ok_or("Not authenticated with TIDAL")?;
    // Favorites come back ordered by date added (DESC), which matches
    // newest/recent well; other types reorder the same set client-side.
    let albums = client.get_favorite_albums(user_id, offset, size).await?;
    let mut sub_albums: Vec<SubsonicAlbum> = albums
        .items
        .iter()
        .map(mapping::album_to_subsonic)
        .collect();

    match list_type {
        "alphabeticalByName" => {
            sub_albums.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        }
        "alphabeticalByArtist" => {
            sub_albums.sort_by(|a, b| {
                a.artist
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .cmp(&b.artist.as_deref().unwrap_or("").to_lowercase())
            });
        }
        // newest, recent, frequent, starred, random, byYear, byGenre — keep the
        // favorites' date-added order as a reasonable approximation.
        _ => {}
    }
    Ok(sub_albums)
}

async fn handle_get_album_list(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let list_type = params.list_type.as_deref().unwrap_or("alphabeticalByName");
    let size = params.size.unwrap_or(10).min(500);
    let offset = params.offset.unwrap_or(0);
    let mut client = state.tidal.lock().await;
    match fetch_album_list(&mut client, list_type, size, offset).await {
        Ok(sub_albums) => resp.album_list = Some(AlbumList { album: sub_albums }),
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    respond(&resp, &params)
}

async fn handle_get_album_list2(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let list_type = params.list_type.as_deref().unwrap_or("alphabeticalByName");
    let size = params.size.unwrap_or(10).min(500);
    let offset = params.offset.unwrap_or(0);
    let mut client = state.tidal.lock().await;
    match fetch_album_list(&mut client, list_type, size, offset).await {
        Ok(sub_albums) => resp.album_list2 = Some(AlbumList { album: sub_albums }),
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    respond(&resp, &params)
}

async fn handle_get_starred(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let mut client = state.tidal.lock().await;
    let user_id = match client.user_id() {
        Some(id) => id,
        None => {
            return respond(&xml_error(0, "Not authenticated with TIDAL"), &params);
        }
    };
    let base_url = base_url_from_headers(&headers);

    // Fetch starred tracks, albums, and artists
    let mut starred = Starred {
        artist: None,
        album: None,
        song: None,
    };

    if let Ok(tracks) = client.get_favorite_tracks(user_id, 0, 100).await {
        starred.song = Some(
            tracks.items.iter().map(|t| mapping::track_to_child(t, &base_url)).collect()
        );
    }
    if let Ok(albums) = client.get_favorite_albums(user_id, 0, 100).await {
        starred.album = Some(
            albums.items.iter().map(|a| mapping::album_to_subsonic(a)).collect()
        );
    }
    if let Ok(artists) = client.get_favorite_artists(user_id, 0, 100).await {
        starred.artist = Some(
            artists.items.iter().map(|a| mapping::artist_to_subsonic(a)).collect()
        );
    }

    resp.starred = Some(starred);

    // Also populate starred2 for v2 clients
    resp.starred2 = Some(Starred2 {
        artist: resp.starred.as_ref().and_then(|s| s.artist.clone()),
        album: resp.starred.as_ref().and_then(|s| s.album.clone()),
        song: resp.starred.as_ref().and_then(|s| s.song.clone()),
    });

    respond(&resp, &params)
}

/// Collect the targets a star/unstar request refers to, as (kind, tidal_id)
/// pairs. `id` may be a song/album/artist (distinguished by its prefix), while
/// `albumId` and `artistId` are explicitly typed.
fn collect_star_targets(params: &SubsonicParams) -> Vec<(&'static str, u64)> {
    let mut out = Vec::new();
    let mut push = |prefixed: &str| {
        if let Some(rest) = prefixed.strip_prefix("tr-") {
            if let Ok(id) = rest.parse() {
                out.push(("track", id));
            }
        } else if let Some(rest) = prefixed.strip_prefix("al-") {
            if let Ok(id) = rest.parse() {
                out.push(("album", id));
            }
        } else if let Some(rest) = prefixed.strip_prefix("ar-") {
            if let Ok(id) = rest.parse() {
                out.push(("artist", id));
            }
        }
    };

    if let Some(id) = &params.id {
        push(id);
    }
    if let Some(ids) = &params.album_id {
        for id in ids {
            push(id);
        }
    }
    if let Some(id) = &params.artist_id_str {
        push(id);
    }
    out
}

async fn handle_star(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let targets = collect_star_targets(&params);
    let mut client = state.tidal.lock().await;
    for (kind, id) in targets {
        let res = match kind {
            "track" => client.add_favorite_track(id).await,
            "album" => client.add_favorite_album(id).await,
            "artist" => client.add_favorite_artist(id).await,
            _ => Ok(()),
        };
        if let Err(e) = res {
            return respond(&xml_error(0, &format!("Star failed: {}", e)), &params);
        }
    }
    respond(&xml_ok(), &params)
}

async fn handle_unstar(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let targets = collect_star_targets(&params);
    let mut client = state.tidal.lock().await;
    for (kind, id) in targets {
        let res = match kind {
            "track" => client.remove_favorite_track(id).await,
            "album" => client.remove_favorite_album(id).await,
            "artist" => client.remove_favorite_artist(id).await,
            _ => Ok(()),
        };
        if let Err(e) = res {
            return respond(&xml_error(0, &format!("Unstar failed: {}", e)), &params);
        }
    }
    respond(&xml_ok(), &params)
}

async fn handle_search2(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let query = match &params.query {
        Some(q) if !q.is_empty() => q.clone(),
        _ => return respond(&xml_error(10, "Missing query"), &params),
    };

    let artist_count = params.artist_count.unwrap_or(20);
    let artist_offset = params.artist_offset.unwrap_or(0);
    let album_count = params.album_count.unwrap_or(20);
    let album_offset = params.album_offset.unwrap_or(0);
    let song_count = params.song_count.unwrap_or(20);
    let song_offset = params.song_offset.unwrap_or(0);

    let mut client = state.tidal.lock().await;
    match client.search(&query, 50).await {
        Ok(results) => {
            let base_url = base_url_from_headers(&headers);
            let artists: Vec<SubsonicArtist> = results.artists[artist_offset as usize..]
                .iter()
                .take(artist_count as usize)
                .map(|a| mapping::search_artist_to_subsonic(a))
                .collect();
            let albums: Vec<SubsonicAlbum> = results.albums[album_offset as usize..]
                .iter()
                .take(album_count as usize)
                .map(|a| mapping::album_to_subsonic(a))
                .collect();
            let songs: Vec<SubsonicChild> = results.tracks[song_offset as usize..]
                .iter()
                .take(song_count as usize)
                .map(|t| mapping::track_to_child(t, &base_url))
                .collect();

            resp.search_result2 = Some(SearchResult2 {
                artist: Some(artists),
                album: Some(albums),
                song: Some(songs),
            });
        }
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    respond(&resp, &params)
}

async fn handle_search3(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let query = match &params.query {
        Some(q) if !q.is_empty() => q.clone(),
        _ => return respond(&xml_error(10, "Missing query"), &params),
    };

    let artist_count = params.artist_count.unwrap_or(20);
    let artist_offset = params.artist_offset.unwrap_or(0);
    let album_count = params.album_count.unwrap_or(20);
    let album_offset = params.album_offset.unwrap_or(0);
    let song_count = params.song_count.unwrap_or(20);
    let song_offset = params.song_offset.unwrap_or(0);

    let mut client = state.tidal.lock().await;
    match client.search(&query, 50).await {
        Ok(results) => {
            let base_url = base_url_from_headers(&headers);
            let artists: Vec<SubsonicArtist> = results.artists[artist_offset as usize..]
                .iter()
                .take(artist_count as usize)
                .map(|a| mapping::search_artist_to_subsonic(a))
                .collect();
            let albums: Vec<SubsonicAlbum> = results.albums[album_offset as usize..]
                .iter()
                .take(album_count as usize)
                .map(|a| mapping::album_to_subsonic(a))
                .collect();
            let songs: Vec<SubsonicChild> = results.tracks[song_offset as usize..]
                .iter()
                .take(song_count as usize)
                .map(|t| mapping::track_to_child(t, &base_url))
                .collect();

            resp.search_result3 = Some(SearchResult3 {
                artist: Some(artists),
                album: Some(albums),
                song: Some(songs),
            });
        }
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    respond(&resp, &params)
}

async fn handle_get_playlists(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let mut client = state.tidal.lock().await;
    let user_id = match client.user_id() {
        Some(id) => id,
        None => {
            return respond(&xml_error(0, "Not authenticated with TIDAL"), &params);
        }
    };

    match client.get_user_playlists(user_id, 0, 200).await {
        Ok(playlists) => {
            let sub_playlists: Vec<SubsonicPlaylist> = playlists
                .items
                .iter()
                .map(|p| mapping::playlist_to_subsonic(p))
                .collect();
            resp.playlists = Some(PlaylistsWrapper {
                playlist: sub_playlists,
            });
        }
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    respond(&resp, &params)
}

async fn handle_get_playlist(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    let playlist_id_str = match &params.id {
        Some(id) => id.clone(),
        None => return respond(&xml_error(10, "Missing playlist id"), &params),
    };
    let playlist_uuid = match playlist_id_str.strip_prefix("pl-") {
        Some(uuid) => uuid.to_string(),
        None => return respond(&xml_error(0, "Invalid playlist id"), &params),
    };

    let mut client = state.tidal.lock().await;
    // Fetch playlist metadata (name/owner/cover) alongside its tracks.
    let meta = client.get_playlist(&playlist_uuid).await.ok();
    match client.get_playlist_tracks(&playlist_uuid, 0, 500).await {
        Ok(tracks) => {
            let base_url = base_url_from_headers(&headers);
            let songs: Vec<SubsonicChild> = tracks
                .items
                .iter()
                .map(|t| mapping::track_to_child(t, &base_url))
                .collect();
            let cover_art = meta
                .as_ref()
                .map(mapping::playlist_to_subsonic)
                .and_then(|p| p.cover_art)
                .or_else(|| songs.first().and_then(|s| s.cover_art.clone()));
            resp.playlist = Some(PlaylistWithSongs {
                id: format!("pl-{}", playlist_uuid),
                name: meta
                    .as_ref()
                    .map(|p| p.title.clone())
                    .unwrap_or_else(|| "Playlist".to_string()),
                comment: meta.as_ref().and_then(|p| p.description.clone()),
                owner: meta
                    .as_ref()
                    .and_then(|p| p.creator.as_ref())
                    .and_then(|c| c.name.clone()),
                public: meta.as_ref().and_then(|p| p.public_playlist),
                song_count: Some(songs.len() as u32),
                duration: meta.as_ref().and_then(|p| p.duration),
                created: meta.as_ref().and_then(|p| p.created.clone()),
                changed: meta.as_ref().and_then(|p| p.last_updated.clone()),
                cover_art,
                entry: songs,
            });
        }
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    respond(&resp, &params)
}

async fn handle_create_playlist(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    // Playlist creation not implemented via Subsonic API
    let resp = xml_error(0, "Playlist creation not supported via Subsonic API. Use the TIDAL app.");
    respond(&resp, &params)
}

async fn handle_update_playlist(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let resp = xml_error(0, "Playlist updates not supported via Subsonic API.");
    respond(&resp, &params)
}

async fn handle_delete_playlist(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let resp = xml_error(0, "Playlist deletion not supported via Subsonic API.");
    respond(&resp, &params)
}

// ------ Cover Art / Image proxy ------ 

async fn handle_get_cover_art(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let cover_id = match &params.id {
        Some(id) => id.clone(),
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Missing cover id").into_response();
        }
    };

    let size = params.size.unwrap_or(640);
    let candidates = mapping::cover_art_urls(&cover_id, size);
    if candidates.is_empty() {
        return (StatusCode::NOT_FOUND, "Invalid cover ID").into_response();
    }

    // TIDAL serves different size sets per image kind (album vs artist), so try
    // the size-ranked candidates until one exists on the CDN.
    for image_url in &candidates {
        match state.http_client.get(image_url).send().await {
            Ok(response) if response.status().is_success() => {
                let content_type = response
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("image/jpeg")
                    .to_string();
                if let Ok(bytes) = response.bytes().await {
                    return (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, content_type)],
                        bytes.to_vec(),
                    )
                        .into_response();
                }
            }
            Ok(_) => continue,
            Err(e) => {
                tracing::warn!("Cover art fetch error: {} for {}", e, image_url);
            }
        }
    }

    tracing::warn!("Cover art not found for id {}", cover_id);
    (StatusCode::NOT_FOUND, "Cover image not found").into_response()
}

// ------ Streaming ------ 

async fn handle_stream(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let track_id_str = match &params.id {
        Some(id) => id.clone(),
        None => {
            return respond(&xml_error(10, "Missing id parameter"), &params);
        }
    };

    let track_id: u64 = match track_id_str.strip_prefix("tr-").and_then(|s| s.parse().ok()) {
        Some(id) => id,
        None => {
            return respond(&xml_error(0, "Invalid track id"), &params);
        }
    };

    // Determine the quality ceiling.
    //
    // TIDAL delivers LOW/HIGH as AAC-in-MP4 and LOSSLESS/HI_RES as FLAC-in-MP4.
    // We don't transcode, so we map Subsonic's transcode request onto a TIDAL
    // quality the client can already play:
    //   - format=raw (or the server's configured format)  -> server max quality
    //   - any other explicit format (mp3/aac/opus/...)     -> AAC (HIGH), since a
    //     client asking to transcode is signalling it can't take the raw codec
    //     (e.g. FLAC); AAC is the best-quality container it will accept.
    //   - maxBitRate caps quality regardless.
    let max_bit_rate = params.max_bit_rate.unwrap_or(0);
    let requested_format = params.format.as_deref().map(|s| s.to_ascii_lowercase());
    let wants_transcode = matches!(
        requested_format.as_deref(),
        Some(fmt) if fmt != "raw" && !fmt.is_empty()
    );

    let ceiling = if wants_transcode {
        // Client can't take the raw codec: give it AAC, bitrate-capped.
        if max_bit_rate != 0 && max_bit_rate < 128 {
            "LOW"
        } else {
            "HIGH"
        }
    } else if max_bit_rate == 0 || max_bit_rate >= 320 {
        state.max_quality.as_str()
    } else if max_bit_rate >= 128 {
        "HIGH"
    } else {
        "LOW"
    };

    let stream_info = {
        let mut client = state.tidal.lock().await;
        match client.get_streamable_url(track_id, ceiling).await {
            Ok(info) => info,
            Err(e) => {
                return respond(&xml_error(0, &format!("Stream URL error: {}", e)), &params);
            }
        }
    };

    tracing::info!(
        "Streaming track {} (quality: {:?}, codec: {:?}, segments: {})",
        track_id,
        stream_info.audio_quality,
        stream_info.codec,
        stream_info.segments.len(),
    );

    // Segmented DASH: concatenate the ordered segments into one playable file.
    if !stream_info.segments.is_empty() {
        let range = headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        return stream_segments(&state.http_client, stream_info, range).await;
    }

    // Forward the client's Range header so seeking works and clients can
    // stream progressively instead of waiting for the full file. reqwest and
    // axum use different `http` crate versions, so pass the value as a string.
    let mut req = state.http_client.get(&stream_info.url);
    if let Some(range) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        req = req.header("range", range);
    }

    let upstream = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return respond(&xml_error(0, &format!("Stream fetch error: {}", e)), &params);
        }
    };

    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::OK);
    if !status.is_success() {
        return respond(
            &xml_error(0, &format!("Upstream stream error: HTTP {}", status)),
            &params,
        );
    }

    // Preserve the headers a Subsonic client needs for playback and seeking.
    let upstream_get = |name: &str| -> Option<String> {
        upstream
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    };
    let content_type =
        upstream_get("content-type").unwrap_or_else(|| default_content_type(&stream_info.codec));

    let mut out_headers = axum::http::HeaderMap::new();
    out_headers.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    for name in ["content-length", "content-range", "accept-ranges"] {
        if let Some(val) = upstream_get(name) {
            if let (Ok(hn), Ok(hv)) = (
                axum::http::HeaderName::from_bytes(name.as_bytes()),
                val.parse::<axum::http::HeaderValue>(),
            ) {
                out_headers.insert(hn, hv);
            }
        }
    }
    out_headers.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());

    // Stream the body through instead of buffering the whole file in memory.
    let body = axum::body::Body::from_stream(upstream.bytes_stream());
    (status, out_headers, body).into_response()
}

/// Serve a segmented DASH track as a single seekable file. The segments are
/// fetched and concatenated into one fragmented-MP4 buffer, then served with a
/// `Content-Length` and range support — many players require a known size and a
/// `206` response to a `Range` request before they will start playback.
async fn stream_segments(
    client: &ReqwestClient,
    info: StreamInfo,
    range: Option<String>,
) -> Response {
    // TIDAL's segmented streams are always fragmented MP4 (fMP4), regardless of
    // the audio codec inside (AAC or FLAC-in-MP4), so advertise the container.
    let content_type = "audio/mp4";

    // Fetch all segments in order and concatenate. These files are single
    // tracks (~9-40 MB), so buffering to get a seekable, length-known response
    // is worth it.
    let mut buf: Vec<u8> = Vec::new();
    for (idx, seg_url) in info.segments.iter().enumerate() {
        match client.get(seg_url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(bytes) => buf.extend_from_slice(&bytes),
                Err(e) => {
                    return (
                        StatusCode::BAD_GATEWAY,
                        format!("segment {} body error: {}", idx, e),
                    )
                        .into_response();
                }
            },
            Ok(resp) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    format!("segment {} HTTP {}", idx, resp.status()),
                )
                    .into_response();
            }
            Err(e) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    format!("segment {} fetch error: {}", idx, e),
                )
                    .into_response();
            }
        }
    }

    serve_bytes_with_range(buf, content_type, range.as_deref())
}

/// Serve an in-memory body with `Accept-Ranges` and single-range `206` support.
fn serve_bytes_with_range(data: Vec<u8>, content_type: &str, range: Option<&str>) -> Response {
    let total = data.len() as u64;

    // Parse a single "bytes=start-end" range; ignore anything more exotic.
    let parsed = range.and_then(|r| parse_byte_range(r, total));

    let mut out_headers = axum::http::HeaderMap::new();
    out_headers.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    out_headers.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
    out_headers.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());

    match parsed {
        Some((start, end)) => {
            // end is inclusive
            let slice = data[start as usize..=end as usize].to_vec();
            out_headers.insert(
                header::CONTENT_RANGE,
                format!("bytes {}-{}/{}", start, end, total).parse().unwrap(),
            );
            out_headers.insert(header::CONTENT_LENGTH, slice.len().to_string().parse().unwrap());
            (StatusCode::PARTIAL_CONTENT, out_headers, slice).into_response()
        }
        None => {
            out_headers.insert(header::CONTENT_LENGTH, total.to_string().parse().unwrap());
            (StatusCode::OK, out_headers, data).into_response()
        }
    }
}

/// Parse a `Range: bytes=start-end` header into inclusive (start, end) byte
/// offsets, clamped to the content length. Returns None for absent/unsatisfiable
/// or open-ended-from-zero ranges (treated as a full 200 response).
fn parse_byte_range(header: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    // Only handle the first range in a possibly comma-separated list.
    let first = spec.split(',').next()?.trim();
    let (start_s, end_s) = first.split_once('-')?;

    let (start, end) = if start_s.is_empty() {
        // suffix range: last N bytes
        let n: u64 = end_s.parse().ok()?;
        if n == 0 {
            return None;
        }
        (total.saturating_sub(n), total - 1)
    } else {
        let start: u64 = start_s.parse().ok()?;
        let end: u64 = if end_s.is_empty() {
            total - 1
        } else {
            end_s.parse().ok()?
        };
        (start, end.min(total - 1))
    };

    if start > end || start >= total {
        return None;
    }
    // A plain "bytes=0-" over the whole file is best served as a normal 200.
    if start == 0 && end == total - 1 {
        return None;
    }
    Some((start, end))
}

fn default_content_type(codec: &Option<String>) -> String {
    match codec.as_deref().map(|c| c.to_ascii_uppercase()) {
        Some(ref c) if c.contains("FLAC") => "audio/flac".to_string(),
        Some(ref c) if c.contains("AAC") || c.contains("MP4A") => "audio/mp4".to_string(),
        Some(ref c) if c.contains("MP3") => "audio/mpeg".to_string(),
        _ => "audio/flac".to_string(),
    }
}

async fn handle_scrobble(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    // Scrobbling is not implemented - just acknowledge
    let resp = xml_ok();
    respond(&resp, &params)
}

async fn handle_get_user(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    resp.user = Some(SubsonicUser {
        username: state.subsonic_username.clone(),
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
    });
    respond(&resp, &params)
}

async fn handle_get_scan_status(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    // There is no local library to scan — the proxy is always "done scanning".
    resp.scan_status = Some(ScanStatus {
        scanning: false,
        count: Some(0),
    });
    respond(&resp, &params)
}

/// `startScan` on a live proxy has nothing to scan, so we immediately report a
/// finished scan. Clients (e.g. Submariner) poll getScanStatus afterwards and
/// need to see `scanning=false` to consider the scan complete.
async fn handle_start_scan(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    resp.scan_status = Some(ScanStatus {
        scanning: false,
        count: Some(0),
    });
    respond(&resp, &params)
}

/// We don't track cross-client playback, so report an empty now-playing list.
async fn handle_get_now_playing(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    resp.now_playing = Some(NowPlaying { entry: vec![] });
    respond(&resp, &params)
}

async fn handle_get_genres(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state, &params) {
        return respond(&xml_error(40, "Wrong username or password"), &params);
    }
    let mut resp = xml_ok();
    resp.genres = Some(GenresWrapper { genre: vec![] });
    respond(&resp, &params)
}

async fn handle_get_open_subsonic_extensions(
    Query(params): Query<SubsonicParams>,
) -> Response {
    // Per the OpenSubsonic spec this discovery endpoint does not require auth.
    // We advertise no optional extensions (empty list) — enough for clients to
    // recognise us as OpenSubsonic-aware without promising unimplemented ones.
    let mut resp = xml_ok();
    resp.open_subsonic_extensions = Some(vec![]);
    respond(&resp, &params)
}

async fn handle_get_avatar(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
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

// Default handler for unimplemented endpoints
async fn handle_not_implemented(
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

    if !verify_auth(&state, &auth_params) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_full_from_zero_is_treated_as_200() {
        // "bytes=0-" over the whole file → None so we serve a plain 200.
        assert_eq!(parse_byte_range("bytes=0-", 1000), None);
    }

    #[test]
    fn range_explicit_window_is_inclusive() {
        assert_eq!(parse_byte_range("bytes=100-199", 1000), Some((100, 199)));
    }

    #[test]
    fn range_open_ended_and_clamped() {
        assert_eq!(parse_byte_range("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_byte_range("bytes=900-5000", 1000), Some((900, 999)));
    }

    #[test]
    fn range_suffix_returns_last_n_bytes() {
        assert_eq!(parse_byte_range("bytes=-100", 1000), Some((900, 999)));
    }

    #[test]
    fn range_invalid_or_unsatisfiable_is_none() {
        assert_eq!(parse_byte_range("bytes=2000-3000", 1000), None);
        assert_eq!(parse_byte_range("bytes=abc", 1000), None);
        assert_eq!(parse_byte_range("kbytes=0-10", 1000), None);
        assert_eq!(parse_byte_range("bytes=0-10", 0), None);
    }
}

/// Redact sensitive Subsonic auth params (token, salt, password) from a query
/// string before logging it.
fn redact_query(query: &str) -> String {
    query
        .split('&')
        .map(|pair| {
            let key = pair.split('=').next().unwrap_or("");
            match key {
                "t" | "s" | "p" | "token" | "salt" | "password" => format!("{}=<redacted>", key),
                _ => pair.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Log every incoming request (method, path, redacted query) and its response
/// status + latency at INFO level, so request activity is always visible.
async fn log_requests(
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

    let started = std::time::Instant::now();
    let response = next.run(req).await;
    let elapsed_ms = started.elapsed().as_millis();
    let status = response.status().as_u16();

    match query {
        Some(q) => tracing::info!("{} {}?{} -> {} ({} ms)", method, path, q, status, elapsed_ms),
        None => tracing::info!("{} {} -> {} ({} ms)", method, path, status, elapsed_ms),
    }
    response
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
    let tidal = Arc::new(tokio::sync::Mutex::new(TidalClient::from_db_config(&cfg)));

    // If we have tokens but no user_id, try to get session info
    {
        let mut client = tidal.lock().await;
        if client.is_authenticated() && client.user_id().is_none() {
            tracing::info!("Authenticated, fetching session info...");
            match client.get_session_info().await {
                Ok(uid) => {
                    tracing::info!("Logged in as user {}", uid);
                    db::save_tokens(&db, &client.access_token, &client.refresh_token, Some(uid), &client.country_code)
                        .await
                        .ok();
                }
                Err(e) => {
                    tracing::warn!("Failed to get session info: {} - will try again on first API call", e);
                }
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

    let app = auth::auth_routes()
        .route("/rest/ping", get(handle_ping))
        .route("/rest/ping.view", get(handle_ping))
        .route("/rest/getLicense", get(handle_get_license))
        .route("/rest/getLicense.view", get(handle_get_license))
        .route("/rest/getMusicFolders", get(handle_get_music_folders))
        .route("/rest/getMusicFolders.view", get(handle_get_music_folders))
        .route("/rest/getIndexes", get(handle_get_indexes))
        .route("/rest/getIndexes.view", get(handle_get_indexes))
        .route("/rest/getArtists", get(handle_get_artists))
        .route("/rest/getArtists.view", get(handle_get_artists))
        .route("/rest/getArtist", get(handle_get_artist))
        .route("/rest/getArtist.view", get(handle_get_artist))
        .route("/rest/getMusicDirectory", get(handle_get_music_directory))
        .route("/rest/getMusicDirectory.view", get(handle_get_music_directory))
        .route("/rest/getAlbum", get(handle_get_album))
        .route("/rest/getAlbum.view", get(handle_get_album))
        .route("/rest/getSong", get(handle_get_song))
        .route("/rest/getSong.view", get(handle_get_song))
        .route("/rest/getRandomSongs", get(handle_get_random_songs))
        .route("/rest/getRandomSongs.view", get(handle_get_random_songs))
        .route("/rest/getAlbumList", get(handle_get_album_list))
        .route("/rest/getAlbumList.view", get(handle_get_album_list))
        .route("/rest/getAlbumList2", get(handle_get_album_list2))
        .route("/rest/getAlbumList2.view", get(handle_get_album_list2))
        .route("/rest/getStarred", get(handle_get_starred))
        .route("/rest/getStarred.view", get(handle_get_starred))
        .route("/rest/getStarred2", get(handle_get_starred))
        .route("/rest/getStarred2.view", get(handle_get_starred))
        .route("/rest/star", get(handle_star))
        .route("/rest/star.view", get(handle_star))
        .route("/rest/unstar", get(handle_unstar))
        .route("/rest/unstar.view", get(handle_unstar))
        .route("/rest/search2", get(handle_search2))
        .route("/rest/search2.view", get(handle_search2))
        .route("/rest/search3", get(handle_search3))
        .route("/rest/search3.view", get(handle_search3))
        .route("/rest/getPlaylists", get(handle_get_playlists))
        .route("/rest/getPlaylists.view", get(handle_get_playlists))
        .route("/rest/getPlaylist", get(handle_get_playlist))
        .route("/rest/getPlaylist.view", get(handle_get_playlist))
        .route("/rest/createPlaylist", get(handle_create_playlist))
        .route("/rest/createPlaylist.view", get(handle_create_playlist))
        .route("/rest/updatePlaylist", get(handle_update_playlist))
        .route("/rest/updatePlaylist.view", get(handle_update_playlist))
        .route("/rest/deletePlaylist", get(handle_delete_playlist))
        .route("/rest/deletePlaylist.view", get(handle_delete_playlist))
        .route("/rest/getCoverArt", get(handle_get_cover_art))
        .route("/rest/getCoverArt.view", get(handle_get_cover_art))
        .route("/rest/stream", get(handle_stream))
        .route("/rest/stream.view", get(handle_stream))
        .route("/rest/scrobble", get(handle_scrobble))
        .route("/rest/scrobble.view", get(handle_scrobble))
        .route("/rest/getUser", get(handle_get_user))
        .route("/rest/getUser.view", get(handle_get_user))
        .route("/rest/getScanStatus", get(handle_get_scan_status))
        .route("/rest/getScanStatus.view", get(handle_get_scan_status))
        .route("/rest/startScan", get(handle_start_scan))
        .route("/rest/startScan.view", get(handle_start_scan))
        .route("/rest/getGenres", get(handle_get_genres))
        .route("/rest/getGenres.view", get(handle_get_genres))
        .route("/rest/getNowPlaying", get(handle_get_now_playing))
        .route("/rest/getNowPlaying.view", get(handle_get_now_playing))
        .route("/rest/getOpenSubsonicExtensions", get(handle_get_open_subsonic_extensions))
        .route("/rest/getOpenSubsonicExtensions.view", get(handle_get_open_subsonic_extensions))
        .route("/rest/getAvatar", get(handle_get_avatar))
        .route("/rest/getAvatar.view", get(handle_get_avatar))
        .fallback(handle_not_implemented)
        .layer(axum::middleware::from_fn(log_requests))
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
