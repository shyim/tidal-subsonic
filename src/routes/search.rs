use crate::app::base_url_from_headers;
use crate::auth_mw::{ApiError, ApiResult, Authed};
use crate::mapping;
use crate::subsonic::*;
use axum::http::HeaderMap;

pub(crate) async fn handle_search2(authed: Authed, headers: HeaderMap) -> ApiResult {
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

pub(crate) async fn handle_search3(authed: Authed, headers: HeaderMap) -> ApiResult {
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
