mod config;
mod mapping;
mod subsonic;
mod tidal_client;

use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use config::Config;
use reqwest::Client as ReqwestClient;
use serde::Deserialize;
use std::sync::Arc;
use subsonic::*;
use tidal_client::*;

const API_VERSION: &str = "1.16.1";
const SERVER_NAME: &str = "tidal-subsonic";

#[derive(Clone)]
struct AppState {
    tidal: SharedTidalClient,
    config: Config,
    http_client: ReqwestClient,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubsonicParams {
    u: String,          // username
    t: String,          // token = md5(password + salt)
    s: String,          // salt (random)
    v: String,          // API version
    c: String,          // client name
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
}

fn verify_auth(config: &Config, params: &SubsonicParams) -> bool {
    // Subsonic auth: token = md5(password + salt)
    let expected = format!("{:x}", md5::compute(format!("{}{}", config.subsonic.password, params.s)));
    params.u == config.subsonic.username && params.t == expected
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
    }
}

fn serialize_to_xml(resp: &SubsonicResponse) -> String {
    let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    xml.push('\n');
    xml.push_str(&serde_xml_rs::to_string(resp).unwrap_or_default());
    xml
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
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let resp = xml_ok();
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_license(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    resp.license = Some(License {
        valid: true,
        email: None,
        license_expires: None,
        trial_expires: None,
    });
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_music_folders(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    resp.music_folders = Some(MusicFolders {
        music_folder: vec![MusicFolder {
            id: 1,
            name: "TIDAL".to_string(),
        }],
    });
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_indexes(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    let mut client = state.tidal.lock().await;
    let user_id = match client.user_id() {
        Some(id) => id,
        None => {
            return (StatusCode::OK, [("content-type", "text/xml")], 
                serialize_to_xml(&xml_error(0, "Not authenticated with TIDAL"))).into_response();
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
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_artists(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    let mut client = state.tidal.lock().await;
    let user_id = match client.user_id() {
        Some(id) => id,
        None => {
            return (StatusCode::OK, [("content-type", "text/xml")], 
                serialize_to_xml(&xml_error(0, "Not authenticated with TIDAL"))).into_response();
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
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_artist(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    let artist_id_str = match &params.id {
        Some(id) => id.clone(),
        None => return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(10, "Missing artist id"))).into_response(),
    };
    let artist_id: u64 = match artist_id_str.strip_prefix("ar-").and_then(|s| s.parse().ok()) {
        Some(id) => id,
        None => return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(0, "Invalid artist id"))).into_response(),
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
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_album(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    let album_id_str = match &params.id {
        Some(id) => id.clone(),
        None => return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(10, "Missing album id"))).into_response(),
    };
    let album_id: u64 = match album_id_str.strip_prefix("al-").and_then(|s| s.parse().ok()) {
        Some(id) => id,
        None => return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(0, "Invalid album id"))).into_response(),
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
                Err(e) => {
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
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_song(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    let track_id_str = match &params.id {
        Some(id) => id.clone(),
        None => return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(10, "Missing track id"))).into_response(),
    };
    let track_id: u64 = match track_id_str.strip_prefix("tr-").and_then(|s| s.parse().ok()) {
        Some(id) => id,
        None => return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(0, "Invalid track id"))).into_response(),
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
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_random_songs(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    let size = params.size.unwrap_or(10).min(50);
    let mut client = state.tidal.lock().await;
    let user_id = match client.user_id() {
        Some(id) => id,
        None => {
            return (StatusCode::OK, [("content-type", "text/xml")], 
                serialize_to_xml(&xml_error(0, "Not authenticated with TIDAL"))).into_response();
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
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_album_list(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    let list_type = params.list_type.as_deref().unwrap_or("alphabeticalByName");
    let size = params.size.unwrap_or(10).min(500);
    let offset = params.offset.unwrap_or(0);
    let mut client = state.tidal.lock().await;
    let user_id = match client.user_id() {
        Some(id) => id,
        None => {
            return (StatusCode::OK, [("content-type", "text/xml")], 
                serialize_to_xml(&xml_error(0, "Not authenticated with TIDAL"))).into_response();
        }
    };

    // For most types, we use favorite albums as the source
    match client.get_favorite_albums(user_id, offset, size).await {
        Ok(albums) => {
            let sub_albums: Vec<SubsonicAlbum> = albums
                .items
                .iter()
                .map(|a| mapping::album_to_subsonic(a))
                .collect();
            match list_type {
                "alphabeticalByName" => {
                    resp.album_list = Some(AlbumList { album: sub_albums });
                }
                "alphabeticalByArtist" => {
                    resp.album_list2 = Some(AlbumList { album: sub_albums });
                }
                "newest" | "recent" | "frequent" | "starred" => {
                    resp.album_list2 = Some(AlbumList { album: sub_albums });
                }
                _ => {
                    resp.album_list = Some(AlbumList { album: sub_albums });
                }
            }
        }
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_starred(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    let mut client = state.tidal.lock().await;
    let user_id = match client.user_id() {
        Some(id) => id,
        None => {
            return (StatusCode::OK, [("content-type", "text/xml")], 
                serialize_to_xml(&xml_error(0, "Not authenticated with TIDAL"))).into_response();
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

    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_star(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    // Starring not implemented - just return OK
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&xml_ok())).into_response()
}

async fn handle_unstar(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    // Unstarring not implemented - just return OK
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&xml_ok())).into_response()
}

async fn handle_search2(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    let query = match &params.query {
        Some(q) if !q.is_empty() => q.clone(),
        _ => return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(10, "Missing query"))).into_response(),
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
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_search3(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    let query = match &params.query {
        Some(q) if !q.is_empty() => q.clone(),
        _ => return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(10, "Missing query"))).into_response(),
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
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_playlists(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    let mut client = state.tidal.lock().await;
    let user_id = match client.user_id() {
        Some(id) => id,
        None => {
            return (StatusCode::OK, [("content-type", "text/xml")], 
                serialize_to_xml(&xml_error(0, "Not authenticated with TIDAL"))).into_response();
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
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_playlist(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
    headers: HeaderMap,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    let playlist_id_str = match &params.id {
        Some(id) => id.clone(),
        None => return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(10, "Missing playlist id"))).into_response(),
    };
    let playlist_uuid = match playlist_id_str.strip_prefix("pl-") {
        Some(uuid) => uuid.to_string(),
        None => return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(0, "Invalid playlist id"))).into_response(),
    };

    let mut client = state.tidal.lock().await;
    match client.get_playlist_tracks(&playlist_uuid, 0, 500).await {
        Ok(tracks) => {
            let base_url = base_url_from_headers(&headers);
            let songs: Vec<SubsonicChild> = tracks
                .items
                .iter()
                .map(|t| mapping::track_to_child(t, &base_url))
                .collect();
            resp.playlist = Some(PlaylistWithSongs {
                id: format!("pl-{}", playlist_uuid),
                name: "Playlist".to_string(), // We don't have the name here from tracks endpoint
                comment: None,
                owner: None,
                public: Some(false),
                song_count: Some(songs.len() as u32),
                duration: None,
                created: None,
                changed: None,
                cover_art: songs.first().and_then(|s| s.cover_art.clone()),
                entry: songs,
            });
        }
        Err(e) => {
            resp.error = Some(SubsonicError { code: 0, message: e });
            resp.status = "failed".to_string();
        }
    }
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_create_playlist(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    // Playlist creation not implemented via Subsonic API
    let resp = xml_error(0, "Playlist creation not supported via Subsonic API. Use the TIDAL app.");
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_update_playlist(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let resp = xml_error(0, "Playlist updates not supported via Subsonic API.");
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_delete_playlist(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let resp = xml_error(0, "Playlist deletion not supported via Subsonic API.");
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

// ------ Cover Art / Image proxy ------ 

async fn handle_get_cover_art(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let cover_id = match &params.id {
        Some(id) => id.clone(),
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Missing cover id").into_response();
        }
    };

    // The cover_id is in format "cover-<url-fragment>" where the URL was escaped
    // Actually we stored the Tidal image URL, replacing / with _ and : with _
    // Let's try to reconstruct it
    // Most Tidal image URLs look like: https://resources.tidal.com/images/...
    // or are just UUIDs

    let image_url = if cover_id.starts_with("cover-") {
        let encoded = &cover_id[6..];
        // Try to reconstruct the URL
        if encoded.starts_with("http") || encoded.starts_with("https") {
            // Replace our escape chars back
            encoded.replace("_slash_", "/").replace("_colon_", ":")
        } else {
            // It's a UUID, try constructing a Tidal image URL
            // Tidal images are typically at: https://resources.tidal.com/images/<uuid>/<size>.jpg
            format!("https://resources.tidal.com/images/{}/640x640.jpg", encoded)
        }
    } else {
        return (StatusCode::NOT_FOUND, "Invalid cover ID").into_response();
    };

    // Proxy the image
    let img_url = image_url.replace("_", "/").replace("_colon_", ":");
    // Try multiple possible URL constructions
    let urls_to_try = vec![
        img_url.clone(),
        // Original stored as cover-<uuid> where uuid might be what we stored
        if img_url.contains("resources.tidal.com") {
            img_url.clone()
        } else {
            format!("https://resources.tidal.com/images/{}/640x640.jpg", img_url)
        },
    ];

    for url in &urls_to_try {
        if let Ok(response) = state.http_client.get(url).send().await {
            if response.status().is_success() {
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
                    ).into_response();
                }
            }
        }
    }

    (StatusCode::NOT_FOUND, "Cover image not found").into_response()
}

// ------ Streaming ------ 

async fn handle_stream(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let track_id_str = match &params.id {
        Some(id) => id.clone(),
        None => {
            return (StatusCode::OK, [("content-type", "text/xml")], 
                serialize_to_xml(&xml_error(10, "Missing id parameter"))).into_response();
        }
    };

    let track_id: u64 = match track_id_str.strip_prefix("tr-").and_then(|s| s.parse().ok()) {
        Some(id) => id,
        None => {
            return (StatusCode::OK, [("content-type", "text/xml")], 
                serialize_to_xml(&xml_error(0, "Invalid track id"))).into_response();
        }
    };

    let max_bit_rate = params.max_bit_rate.unwrap_or(0);

    // Map bitrate to quality
    let quality = if max_bit_rate == 0 || max_bit_rate >= 320 {
        &state.config.tidal.max_quality
    } else if max_bit_rate >= 128 {
        "HIGH"
    } else {
        "LOW"
    };

    let mut client = state.tidal.lock().await;
    match client.get_stream_url(track_id, quality).await {
        Ok(stream_info) => {
            tracing::info!(
                "Streaming track {} (quality: {:?}, codec: {:?})",
                track_id,
                stream_info.audio_quality,
                stream_info.codec
            );

            // Fetch the actual stream from the CDN
            match state.http_client.get(&stream_info.url).send().await {
                Ok(response) => {
                    let content_type = response
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("audio/flac")
                        .to_string();
                    if let Ok(bytes) = response.bytes().await {
                        return (
                            StatusCode::OK,
                            [
                                (header::CONTENT_TYPE, content_type),
                                (header::CACHE_CONTROL, "no-cache".to_string()),
                            ],
                            bytes.to_vec(),
                        ).into_response();
                    }
                }
                Err(e) => {
                    return (StatusCode::OK, [("content-type", "text/xml")], 
                        serialize_to_xml(&xml_error(0, &format!("Stream fetch error: {}", e)))).into_response();
                }
            }
        }
        Err(e) => {
            return (StatusCode::OK, [("content-type", "text/xml")], 
                serialize_to_xml(&xml_error(0, &format!("Stream URL error: {}", e)))).into_response();
        }
    }

    (StatusCode::OK, [("content-type", "text/xml")], 
        serialize_to_xml(&xml_error(0, "Unknown streaming error"))).into_response()
}

async fn handle_scrobble(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    // Scrobbling is not implemented - just acknowledge
    let resp = xml_ok();
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_user(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    resp.user = Some(SubsonicUser {
        username: state.config.subsonic.username.clone(),
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
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_scan_status(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    resp.scan_status = Some(ScanStatus {
        scanning: false,
        count: None,
    });
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_genres(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    resp.genres = Some(GenresWrapper { genre: vec![] });
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

async fn handle_get_avatar(
    State(state): State<AppState>,
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
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
    Query(params): Query<SubsonicParams>,
) -> Response {
    if !verify_auth(&state.config, &params) {
        return (StatusCode::OK, [("content-type", "text/xml")], 
            serialize_to_xml(&xml_error(40, "Wrong username or password"))).into_response();
    }
    let mut resp = xml_ok();
    // Return empty success for unknown endpoints
    (StatusCode::OK, [("content-type", "text/xml")], serialize_to_xml(&resp)).into_response()
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = Config::load_or_create();
    let tidal = Arc::new(tokio::sync::Mutex::new(TidalClient::new(config.clone())));

    // If we have tokens but no user_id, try to get session info
    {
        let mut client = tidal.lock().await;
        if client.is_authenticated() && client.user_id().is_none() {
            tracing::info!("Authenticated, fetching session info...");
            match client.get_session_info().await {
                Ok(uid) => {
                    tracing::info!("Logged in as user {}", uid);
                    let mut cfg = config.clone();
                    cfg.tidal.user_id = Some(uid);
                    cfg.tidal.access_token = Some(client.access_token.clone());
                    cfg.tidal.refresh_token = Some(client.refresh_token.clone());
                    cfg.save().ok();
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

    let state = AppState {
        tidal: tidal.clone(),
        config: config.clone(),
        http_client,
    };

    let app = Router::new()
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
        .route("/rest/getAlbum", get(handle_get_album))
        .route("/rest/getAlbum.view", get(handle_get_album))
        .route("/rest/getSong", get(handle_get_song))
        .route("/rest/getSong.view", get(handle_get_song))
        .route("/rest/getRandomSongs", get(handle_get_random_songs))
        .route("/rest/getRandomSongs.view", get(handle_get_random_songs))
        .route("/rest/getAlbumList", get(handle_get_album_list))
        .route("/rest/getAlbumList.view", get(handle_get_album_list))
        .route("/rest/getAlbumList2", get(handle_get_album_list))
        .route("/rest/getAlbumList2.view", get(handle_get_album_list))
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
        .route("/rest/getGenres", get(handle_get_genres))
        .route("/rest/getGenres.view", get(handle_get_genres))
        .route("/rest/getAvatar", get(handle_get_avatar))
        .route("/rest/getAvatar.view", get(handle_get_avatar))
        .route("/rest/{*path}", get(handle_not_implemented))
        .with_state(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!("Starting tidal-subsonic server on {}", addr);
    tracing::info!("Username: '{}', Password: '{}'", config.subsonic.username, config.subsonic.password);

    if !tidal.lock().await.is_authenticated() {
        tracing::warn!("Not authenticated with TIDAL. Set client_id in config.toml then restart.");
        tracing::warn!("Config file: ~/.config/tidal-subsonic/config.toml");
    }

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
