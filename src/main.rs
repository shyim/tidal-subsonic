mod auth;
mod db;
mod item_id;
mod mapping;
mod response;
mod subsonic;
mod tidal_client;

use auth::PkceSession;
use item_id::ItemId;
use axum::{
    extract::{FromRequestParts, Query, State},
    http::{header, request::Parts, HeaderMap, StatusCode},
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

tokio::task_local! {
    /// The response format/callback the current request asked for. Set by the
    /// `Authed` extractor (and the fallback handler) before a handler runs so
    /// that `ApiError`'s `IntoResponse` can render errors in the client's format
    /// even though it never sees the query params directly.
    static RESPONSE_CTX: (ResponseFormat, Option<String>);
}

/// Read the current request's response format + jsonp callback, falling back to
/// XML / no-callback if no context was established (should not happen for
/// handlers behind `Authed`).
fn current_response_ctx() -> (ResponseFormat, Option<String>) {
    RESPONSE_CTX
        .try_with(|ctx| ctx.clone())
        .unwrap_or((ResponseFormat::Xml, None))
}

/// Render a Subsonic response using the current request's response context.
fn render_current(resp: &SubsonicResponse) -> Response {
    let (format, callback) = current_response_ctx();
    response::render(resp, format, callback.as_deref())
}

/// Authenticated-request extractor. Running it verifies the Subsonic
/// credentials (token `t`+`s` or legacy `p=` password) exactly as `verify_auth`
/// did per-handler, and rejects with the Subsonic `Wrong username or password`
/// error (code 40) rendered in the client-requested format. On success it
/// exposes the `AppState`, the parsed query `params`, and helpers to reach the
/// TIDAL client and render responses.
struct Authed {
    state: AppState,
    params: SubsonicParams,
}

impl Authed {
    /// The shared TIDAL client for this request.
    fn tidal(&self) -> &SharedTidalClient {
        &self.state.tidal
    }

    /// The authenticated TIDAL user id, or `ApiError::NotAuthedTidal` if the
    /// proxy has no TIDAL session yet.
    async fn tidal_user_id(&self) -> Result<u64, ApiError> {
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
enum ApiError {
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
struct ApiOk(SubsonicResponse);

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
type ApiResult = Result<ApiOk, ApiError>;

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

/// Thin wrapper preserved for call sites: a failed response with an error.
fn xml_error(code: u32, message: &str) -> SubsonicResponse {
    SubsonicResponse::error(code, message)
}

/// Thin wrapper preserved for call sites: a successful response, no payload.
fn xml_ok() -> SubsonicResponse {
    SubsonicResponse::ok()
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

async fn handle_ping(_authed: Authed) -> ApiResult {
    Ok(ApiOk(xml_ok()))
}

async fn handle_get_license(_authed: Authed) -> ApiResult {
    Ok(Payload::License(License {
        valid: true,
        email: None,
        license_expires: None,
        trial_expires: None,
    })
    .into())
}

async fn handle_get_music_folders(_authed: Authed) -> ApiResult {
    Ok(Payload::MusicFolders(MusicFolders {
        music_folder: vec![MusicFolder {
            id: 1,
            name: "TIDAL".to_string(),
        }],
    })
    .into())
}

async fn handle_get_indexes(authed: Authed) -> ApiResult {
    let client = authed.tidal();
    let user_id = authed.tidal_user_id().await?;
    let artists = client
        .get_favorite_artists(user_id, 0, 500)
        .await
        .map_err(ApiError::Tidal)?;
    Ok(Payload::Indexes(mapping::build_indexes(&artists.items)).into())
}

async fn handle_get_artists(authed: Authed) -> ApiResult {
    let client = authed.tidal();
    let user_id = authed.tidal_user_id().await?;
    let artists = client
        .get_favorite_artists(user_id, 0, 500)
        .await
        .map_err(ApiError::Tidal)?;
    let indexes = mapping::build_indexes(&artists.items);
    Ok(Payload::Artists(ArtistsList {
        ignored_articles: indexes.ignored_articles,
        index: indexes.index,
    })
    .into())
}

async fn handle_get_artist(authed: Authed) -> ApiResult {
    let artist_id_str = authed
        .params
        .id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest(10, "Missing artist id".to_string()))?;
    let artist_id = match artist_id_str.parse::<ItemId>() {
        Ok(ItemId::Artist(id)) => id,
        _ => return Err(ApiError::BadRequest(0, "Invalid artist id".to_string())),
    };

    let client = authed.tidal();
    let artist_detail = client
        .get_artist_detail(artist_id)
        .await
        .map_err(ApiError::Tidal)?;
    let sub_artist = mapping::artist_to_subsonic(&artist_detail);
    let mut sub_albums = Vec::new();
    if let Ok(albums) = client.get_artist_albums(artist_id, 0, 100).await {
        sub_albums = albums.items.iter().map(|a| mapping::album_to_subsonic(a)).collect();
    }
    Ok(Payload::Artist(ArtistWithAlbums {
        id: sub_artist.id,
        name: sub_artist.name,
        cover_art: sub_artist.cover_art,
        album_count: Some(sub_albums.len() as u32),
        artist_image_url: sub_artist.artist_image_url,
        starred: None,
        album: sub_albums,
    })
    .into())
}

async fn handle_get_album(authed: Authed, headers: HeaderMap) -> ApiResult {
    let album_id_str = authed
        .params
        .id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest(10, "Missing album id".to_string()))?;
    let album_id = match album_id_str.parse::<ItemId>() {
        Ok(ItemId::Album(id)) => id,
        _ => return Err(ApiError::BadRequest(0, "Invalid album id".to_string())),
    };

    let client = authed.tidal();
    let base_url = base_url_from_headers(&headers);
    let album = client
        .get_album_detail(album_id)
        .await
        .map_err(ApiError::Tidal)?;
    let payload = match client.get_album_tracks(album_id, 0, 200).await {
        Ok(tracks) => Payload::Album(mapping::album_detail_to_album_with_songs(
            &album,
            &tracks.items,
            &base_url,
        )),
        // Still return album without tracks
        Err(_e) => Payload::Album(mapping::album_detail_to_album_with_songs(
            &album, &[], &base_url,
        )),
    };
    Ok(payload.into())
}

/// Classic browsing endpoint used by older clients (e.g. Submariner). An
/// artist id (`ar-`) lists its albums as sub-directories; an album id (`al-`)
/// lists its tracks as songs — matching what getArtist / getAlbum return but in
/// the directory shape those clients navigate with.
async fn handle_get_music_directory(authed: Authed, headers: HeaderMap) -> ApiResult {
    let dir_id = authed
        .params
        .id
        .clone()
        .ok_or_else(|| ApiError::BadRequest(10, "Missing directory id".to_string()))?;
    let base_url = base_url_from_headers(&headers);
    let client = authed.tidal();

    match dir_id.parse::<ItemId>() {
        Ok(ItemId::Artist(artist_id)) => {
            let artist = client
                .get_artist_detail(artist_id)
                .await
                .map_err(ApiError::Tidal)?;
            let mut children = Vec::new();
            if let Ok(albums) = client.get_artist_albums(artist_id, 0, 100).await {
                children = albums
                    .items
                    .iter()
                    .map(|a| mapping::album_to_directory_child(a, &dir_id))
                    .collect();
            }
            Ok(Payload::Directory(Directory {
                id: dir_id.clone(),
                name: artist.name,
                parent: None,
                play_count: None,
                child: children,
            })
            .into())
        }
        Ok(ItemId::Album(album_id)) => {
            let album = client
                .get_album_detail(album_id)
                .await
                .map_err(ApiError::Tidal)?;
            let tracks = client
                .get_album_tracks(album_id, 0, 200)
                .await
                .map(|t| t.items)
                .unwrap_or_default();
            let children = tracks
                .iter()
                .map(|t| mapping::track_to_child(t, &base_url))
                .collect();
            let (_artist_name, artist_id) = mapping::primary_artist(&album);
            let artist_parent = artist_id.map(|id| ItemId::Artist(id).to_string());
            Ok(Payload::Directory(Directory {
                id: dir_id.clone(),
                name: album.title.clone(),
                parent: artist_parent,
                play_count: None,
                child: children,
            })
            .into())
        }
        _ => Err(ApiError::BadRequest(70, "Directory not found".to_string())),
    }
}

async fn handle_get_song(authed: Authed, headers: HeaderMap) -> ApiResult {
    let track_id_str = authed
        .params
        .id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest(10, "Missing track id".to_string()))?;
    let track_id = match track_id_str.parse::<ItemId>() {
        Ok(ItemId::Track(id)) => id,
        _ => return Err(ApiError::BadRequest(0, "Invalid track id".to_string())),
    };

    let client = authed.tidal();
    let track = client.get_track(track_id).await.map_err(ApiError::Tidal)?;
    let base_url = base_url_from_headers(&headers);
    Ok(Payload::Song(mapping::track_to_child(&track, &base_url)).into())
}

async fn handle_get_random_songs(authed: Authed, headers: HeaderMap) -> ApiResult {
    let size = authed.params.size.unwrap_or(10).min(50);
    let client = authed.tidal();
    let user_id = authed.tidal_user_id().await?;
    let tracks = client
        .get_favorite_tracks(user_id, 0, 200)
        .await
        .map_err(ApiError::Tidal)?;
    // Simple "random" - just return the first N tracks (or use search for random)
    let base_url = base_url_from_headers(&headers);
    let count = size.min(tracks.items.len() as u32);
    let songs: Vec<SubsonicChild> = tracks.items[..count as usize]
        .iter()
        .map(|t| mapping::track_to_child(t, &base_url))
        .collect();
    Ok(Payload::RandomSongs(SongList { song: songs }).into())
}

/// Fetch and order the album list for the given `type`, backed by the user's
/// TIDAL favorites (the only broad album source available via the proxy).
async fn fetch_album_list(
    client: &TidalClient,
    list_type: &str,
    size: u32,
    offset: u32,
) -> Result<Vec<SubsonicAlbum>, String> {
    let user_id = client.user_id().await.ok_or("Not authenticated with TIDAL")?;
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

async fn handle_get_album_list(authed: Authed) -> ApiResult {
    let list_type = authed.params.list_type.as_deref().unwrap_or("alphabeticalByName");
    let size = authed.params.size.unwrap_or(10).min(500);
    let offset = authed.params.offset.unwrap_or(0);
    let sub_albums = fetch_album_list(authed.tidal(), list_type, size, offset)
        .await
        .map_err(ApiError::Tidal)?;
    Ok(Payload::AlbumList(AlbumList { album: sub_albums }).into())
}

async fn handle_get_album_list2(authed: Authed) -> ApiResult {
    let list_type = authed.params.list_type.as_deref().unwrap_or("alphabeticalByName");
    let size = authed.params.size.unwrap_or(10).min(500);
    let offset = authed.params.offset.unwrap_or(0);
    let sub_albums = fetch_album_list(authed.tidal(), list_type, size, offset)
        .await
        .map_err(ApiError::Tidal)?;
    Ok(Payload::AlbumList2(AlbumList { album: sub_albums }).into())
}

async fn handle_get_starred(authed: Authed, headers: HeaderMap) -> ApiResult {
    let client = authed.tidal();
    let user_id = authed.tidal_user_id().await?;
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

    // Also populate starred2 for v2 clients, mirroring the same items.
    let starred2 = Starred2 {
        artist: starred.artist.clone(),
        album: starred.album.clone(),
        song: starred.song.clone(),
    };
    Ok(Payload::Starred(starred, starred2).into())
}

/// Collect the targets a star/unstar request refers to, as typed `ItemId`s.
/// `id` may be a song/album/artist (distinguished by its prefix), while
/// `albumId` and `artistId` are explicitly typed. Unparseable ids are skipped.
fn collect_star_targets(params: &SubsonicParams) -> Vec<ItemId> {
    let mut out = Vec::new();
    let mut push = |prefixed: &str| {
        if let Ok(item) = prefixed.parse::<ItemId>() {
            out.push(item);
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

async fn handle_star(authed: Authed) -> ApiResult {
    let targets = collect_star_targets(&authed.params);
    let client = authed.tidal();
    for target in targets {
        let res = match target {
            ItemId::Track(id) => client.add_favorite_track(id).await,
            ItemId::Album(id) => client.add_favorite_album(id).await,
            ItemId::Artist(id) => client.add_favorite_artist(id).await,
            ItemId::Playlist(_) => Ok(()),
        };
        res.map_err(|e| ApiError::Tidal(format!("Star failed: {}", e)))?;
    }
    Ok(ApiOk(xml_ok()))
}

async fn handle_unstar(authed: Authed) -> ApiResult {
    let targets = collect_star_targets(&authed.params);
    let client = authed.tidal();
    for target in targets {
        let res = match target {
            ItemId::Track(id) => client.remove_favorite_track(id).await,
            ItemId::Album(id) => client.remove_favorite_album(id).await,
            ItemId::Artist(id) => client.remove_favorite_artist(id).await,
            ItemId::Playlist(_) => Ok(()),
        };
        res.map_err(|e| ApiError::Tidal(format!("Unstar failed: {}", e)))?;
    }
    Ok(ApiOk(xml_ok()))
}

async fn handle_search2(authed: Authed, headers: HeaderMap) -> ApiResult {
    let query = match &authed.params.query {
        Some(q) if !q.is_empty() => q.clone(),
        _ => return Err(ApiError::BadRequest(10, "Missing query".to_string())),
    };

    let artist_count = authed.params.artist_count.unwrap_or(20);
    let artist_offset = authed.params.artist_offset.unwrap_or(0);
    let album_count = authed.params.album_count.unwrap_or(20);
    let album_offset = authed.params.album_offset.unwrap_or(0);
    let song_count = authed.params.song_count.unwrap_or(20);
    let song_offset = authed.params.song_offset.unwrap_or(0);

    let client = authed.tidal();
    let results = client.search(&query, 50).await.map_err(ApiError::Tidal)?;
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

    Ok(Payload::SearchResult2(SearchResult2 {
        artist: Some(artists),
        album: Some(albums),
        song: Some(songs),
    })
    .into())
}

async fn handle_search3(authed: Authed, headers: HeaderMap) -> ApiResult {
    let query = match &authed.params.query {
        Some(q) if !q.is_empty() => q.clone(),
        _ => return Err(ApiError::BadRequest(10, "Missing query".to_string())),
    };

    let artist_count = authed.params.artist_count.unwrap_or(20);
    let artist_offset = authed.params.artist_offset.unwrap_or(0);
    let album_count = authed.params.album_count.unwrap_or(20);
    let album_offset = authed.params.album_offset.unwrap_or(0);
    let song_count = authed.params.song_count.unwrap_or(20);
    let song_offset = authed.params.song_offset.unwrap_or(0);

    let client = authed.tidal();
    let results = client.search(&query, 50).await.map_err(ApiError::Tidal)?;
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

    Ok(Payload::SearchResult3(SearchResult3 {
        artist: Some(artists),
        album: Some(albums),
        song: Some(songs),
    })
    .into())
}

async fn handle_get_playlists(authed: Authed) -> ApiResult {
    let client = authed.tidal();
    let user_id = authed.tidal_user_id().await?;
    let playlists = client
        .get_user_playlists(user_id, 0, 200)
        .await
        .map_err(ApiError::Tidal)?;
    let sub_playlists: Vec<SubsonicPlaylist> = playlists
        .items
        .iter()
        .map(|p| mapping::playlist_to_subsonic(p))
        .collect();
    Ok(Payload::Playlists(PlaylistsWrapper {
        playlist: sub_playlists,
    })
    .into())
}

async fn handle_get_playlist(authed: Authed, headers: HeaderMap) -> ApiResult {
    let playlist_id_str = authed
        .params
        .id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest(10, "Missing playlist id".to_string()))?;
    let playlist_uuid = match playlist_id_str.parse::<ItemId>() {
        Ok(ItemId::Playlist(uuid)) => uuid,
        _ => return Err(ApiError::BadRequest(0, "Invalid playlist id".to_string())),
    };

    let client = authed.tidal();
    // Fetch playlist metadata (name/owner/cover) alongside its tracks.
    let meta = client.get_playlist(&playlist_uuid).await.ok();
    let tracks = client
        .get_playlist_tracks(&playlist_uuid, 0, 500)
        .await
        .map_err(ApiError::Tidal)?;
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
    Ok(Payload::Playlist(PlaylistWithSongs {
        id: ItemId::Playlist(playlist_uuid.clone()).to_string(),
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
    })
    .into())
}

async fn handle_create_playlist(_authed: Authed) -> ApiResult {
    // Playlist creation not implemented via Subsonic API
    Err(ApiError::Tidal(
        "Playlist creation not supported via Subsonic API. Use the TIDAL app.".to_string(),
    ))
}

async fn handle_update_playlist(_authed: Authed) -> ApiResult {
    Err(ApiError::Tidal(
        "Playlist updates not supported via Subsonic API.".to_string(),
    ))
}

async fn handle_delete_playlist(_authed: Authed) -> ApiResult {
    Err(ApiError::Tidal(
        "Playlist deletion not supported via Subsonic API.".to_string(),
    ))
}

// ------ Cover Art / Image proxy ------ 

async fn handle_get_cover_art(authed: Authed) -> Response {
    let state = &authed.state;
    let cover_id = match &authed.params.id {
        Some(id) => id.clone(),
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Missing cover id").into_response();
        }
    };

    let size = authed.params.size.unwrap_or(640);
    let candidates = mapping::cover_art_urls(&cover_id, size);
    if candidates.is_empty() {
        return (StatusCode::NOT_FOUND, "Invalid cover ID").into_response();
    }

    // TIDAL serves different size sets per image kind (album vs artist), so try
    // the size-ranked candidates until one exists on the CDN.
    for image_url in &candidates {
        // SSRF guard: only ever fetch from TIDAL's image CDN, never a host a
        // crafted cover id might have smuggled in.
        if !is_allowed_cover_host(image_url) {
            tracing::warn!("Blocked non-TIDAL cover art host: {}", image_url);
            continue;
        }
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

async fn handle_stream(authed: Authed, headers: HeaderMap) -> Response {
    let state = &authed.state;
    let params = &authed.params;
    let track_id_str = match &params.id {
        Some(id) => id.clone(),
        None => {
            return render_current(&xml_error(10, "Missing id parameter"));
        }
    };

    let track_id: u64 = match track_id_str.parse::<ItemId>() {
        Ok(ItemId::Track(id)) => id,
        _ => {
            return render_current(&xml_error(0, "Invalid track id"));
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
        let client = &state.tidal;
        match client.get_streamable_url(track_id, ceiling).await {
            Ok(info) => info,
            Err(e) => {
                return render_current(&xml_error(0, &format!("Stream URL error: {}", e)));
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

    // SSRF guard: refuse to proxy any stream/segment URL that points at an
    // internal target. These come from TIDAL's signed manifest, but validate
    // defensively before we make the server fetch them.
    let all_ok = stream_info.segments.iter().all(|u| is_safe_stream_host(u))
        && (stream_info.url.is_empty() || is_safe_stream_host(&stream_info.url));
    if !all_ok {
        tracing::warn!("Blocked unsafe stream host for track {}", track_id);
        return render_current(&xml_error(0, "Refusing to proxy an unsafe stream URL"));
    }

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
            return render_current(&xml_error(0, &format!("Stream fetch error: {}", e)));
        }
    };

    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::OK);
    if !status.is_success() {
        return render_current(&xml_error(0, &format!("Upstream stream error: HTTP {}", status)));
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

async fn handle_scrobble(_authed: Authed) -> ApiResult {
    // Scrobbling is not implemented - just acknowledge
    Ok(ApiOk(xml_ok()))
}

async fn handle_get_user(authed: Authed) -> ApiResult {
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

async fn handle_get_scan_status(_authed: Authed) -> ApiResult {
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
async fn handle_start_scan(_authed: Authed) -> ApiResult {
    Ok(Payload::ScanStatus(ScanStatus {
        scanning: false,
        count: Some(0),
    })
    .into())
}

/// We don't track cross-client playback, so report an empty now-playing list.
async fn handle_get_now_playing(_authed: Authed) -> ApiResult {
    Ok(Payload::NowPlaying(NowPlaying { entry: vec![] }).into())
}

async fn handle_get_genres(_authed: Authed) -> ApiResult {
    Ok(Payload::Genres(GenresWrapper { genre: vec![] }).into())
}

async fn handle_get_open_subsonic_extensions(Query(params): Query<SubsonicParams>) -> Response {
    // Per the OpenSubsonic spec this discovery endpoint does not require auth.
    // We advertise no optional extensions (empty list) — enough for clients to
    // recognise us as OpenSubsonic-aware without promising unimplemented ones.
    let resp = SubsonicResponse::ok_with(Payload::OpenSubsonicExtensions(vec![]));
    respond(&resp, &params)
}

async fn handle_get_avatar(
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
    fn cover_host_allowlist() {
        assert!(is_allowed_cover_host("https://resources.tidal.com/images/a/640x640.jpg"));
        assert!(is_allowed_cover_host("https://foo.tidal.com/x.jpg"));
        assert!(!is_allowed_cover_host("http://169.254.169.254/latest/meta-data/"));
        assert!(!is_allowed_cover_host("https://evil.example.com/x.jpg"));
        assert!(!is_allowed_cover_host("https://resources.tidal.com.evil.com/x"));
    }

    #[test]
    fn stream_host_blocks_internal() {
        assert!(is_safe_stream_host("https://sp-ad-fa.audio.tidal.com/mediatracks/x/0.mp4"));
        assert!(is_safe_stream_host("https://cdn.cloudfront.net/x.mp4")); // regional CDNs allowed
        assert!(!is_safe_stream_host("http://localhost/x"));
        assert!(!is_safe_stream_host("http://127.0.0.1/x"));
        assert!(!is_safe_stream_host("http://169.254.169.254/latest/meta-data/"));
        assert!(!is_safe_stream_host("http://10.0.0.5/x"));
        assert!(!is_safe_stream_host("http://192.168.1.1/x"));
        assert!(!is_safe_stream_host("file:///etc/passwd"));
    }

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

/// Cover-art IDs are user-supplied (a base64 blob that may decode to a full
/// URL), so getCoverArt could be steered at an arbitrary host — an SSRF vector.
/// Restrict cover fetches to TIDAL's image CDN.
fn is_allowed_cover_host(url: &str) -> bool {
    match reqwest::Url::parse(url) {
        Ok(u) => matches!(u.host_str(), Some(h) if h == "resources.tidal.com" || h.ends_with(".tidal.com")),
        Err(_) => false,
    }
}

/// Stream URLs come from TIDAL's own signed playbackinfo manifest, not from
/// user input, so a strict allowlist would break legitimate regional CDNs.
/// Instead just refuse obviously-internal targets (loopback, private, and
/// link-local ranges — the classic SSRF pivots).
fn is_safe_stream_host(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else {
        return false;
    };
    if !matches!(u.scheme(), "http" | "https") {
        return false;
    }
    match u.host() {
        Some(url::Host::Domain(h)) => {
            let h = h.to_ascii_lowercase();
            h != "localhost" && !h.ends_with(".localhost") && !h.ends_with(".internal")
        }
        Some(url::Host::Ipv4(ip)) => {
            !(ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified())
        }
        Some(url::Host::Ipv6(ip)) => !(ip.is_loopback() || ip.is_unspecified()),
        None => false,
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
        ("ping", get(handle_ping)),
        ("getLicense", get(handle_get_license)),
        ("getMusicFolders", get(handle_get_music_folders)),
        ("getIndexes", get(handle_get_indexes)),
        ("getArtists", get(handle_get_artists)),
        ("getArtist", get(handle_get_artist)),
        ("getMusicDirectory", get(handle_get_music_directory)),
        ("getAlbum", get(handle_get_album)),
        ("getSong", get(handle_get_song)),
        ("getRandomSongs", get(handle_get_random_songs)),
        ("getAlbumList", get(handle_get_album_list)),
        ("getAlbumList2", get(handle_get_album_list2)),
        ("getStarred", get(handle_get_starred)),
        ("getStarred2", get(handle_get_starred)),
        ("star", get(handle_star)),
        ("unstar", get(handle_unstar)),
        ("search2", get(handle_search2)),
        ("search3", get(handle_search3)),
        ("getPlaylists", get(handle_get_playlists)),
        ("getPlaylist", get(handle_get_playlist)),
        ("createPlaylist", get(handle_create_playlist)),
        ("updatePlaylist", get(handle_update_playlist)),
        ("deletePlaylist", get(handle_delete_playlist)),
        ("getCoverArt", get(handle_get_cover_art)),
        ("stream", get(handle_stream)),
        ("scrobble", get(handle_scrobble)),
        ("getUser", get(handle_get_user)),
        ("getScanStatus", get(handle_get_scan_status)),
        ("startScan", get(handle_start_scan)),
        ("getGenres", get(handle_get_genres)),
        ("getNowPlaying", get(handle_get_now_playing)),
        ("getOpenSubsonicExtensions", get(handle_get_open_subsonic_extensions)),
        ("getAvatar", get(handle_get_avatar)),
    ] {
        app = rest(app, name, handler);
    }

    let app = app
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
