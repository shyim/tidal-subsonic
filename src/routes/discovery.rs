//! Discovery endpoints: top songs, similar songs, and artist/album info.
//!
//! TIDAL has no true "similar songs" API, so we approximate: for any target we
//! resolve a primary artist and return that artist's top tracks. `getTopSongs`
//! is the same data keyed by artist name. Artist info maps TIDAL's artist bio +
//! picture; album info is minimal (TIDAL lacks rich album notes).

use crate::app::base_url_from_headers;
use crate::auth_mw::{ApiError, ApiResult, Authed};
use crate::item_id::ItemId;
use crate::mapping;
use crate::routes::metadata_cache::{MetadataCache, TTL_CATALOG};
use crate::subsonic::*;
use crate::tidal::SharedTidalClient;
use axum::http::HeaderMap;

/// Resolve the primary TIDAL artist id for a Subsonic item id (artist → itself;
/// album → its artist; track → its artist).
async fn artist_id_for(client: &SharedTidalClient, id: &ItemId) -> Result<u64, ApiError> {
    match id {
        ItemId::Artist(a) => Ok(*a),
        ItemId::Album(al) => {
            let album = client.get_album_detail(*al).await.map_err(ApiError::Tidal)?;
            mapping::primary_artist(&album)
                .1
                .ok_or_else(|| ApiError::Tidal("Album has no artist".to_string()))
        }
        ItemId::Track(t) => {
            let track = client.get_track(*t).await.map_err(ApiError::Tidal)?;
            track
                .artist
                .as_ref()
                .map(|a| a.id)
                .or_else(|| track.artists.as_ref().and_then(|v| v.first()).map(|a| a.id))
                .ok_or_else(|| ApiError::Tidal("Track has no artist".to_string()))
        }
        ItemId::Playlist(_) | ItemId::Mix(_) => {
            Err(ApiError::BadRequest(0, "Invalid id for this operation".into()))
        }
    }
}

/// Fetch an artist's top tracks as Subsonic songs (memoized per user).
async fn top_songs_for_artist(
    authed: &Authed,
    artist_id: u64,
    count: u32,
    base_url: &str,
) -> Result<Vec<SubsonicChild>, ApiError> {
    let client = authed.tidal().await?;
    let key = MetadataCache::key(
        authed.user.id,
        "topSongs",
        &format!("artist={artist_id}&count={count}&base={base_url}"),
    );
    let base = base_url.to_string();
    authed
        .state
        .metadata_cache
        .get_or_build(&key, TTL_CATALOG, || async move {
            let tracks = client.get_artist_top_tracks(artist_id, 0, count).await?;
            Ok(tracks
                .items
                .iter()
                .map(|t| mapping::track_to_child(t, &base))
                .collect::<Vec<_>>())
        })
        .await
        .map_err(ApiError::Tidal)
}

/// `getTopSongs` — top tracks for an artist (by name, per the Subsonic spec).
pub(crate) async fn handle_get_top_songs(authed: Authed, headers: HeaderMap) -> ApiResult {
    let artist_name = authed
        .params
        .artist
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest(10, "Missing artist".to_string()))?;
    let count = authed.params.count.unwrap_or(50).min(100);
    let base_url = base_url_from_headers(&headers);

    // Resolve the artist name to an id via search.
    let client = authed.tidal().await?;
    let results = client.search(artist_name, 5).await.map_err(ApiError::Tidal)?;
    let artist_id = results
        .artists
        .first()
        .map(|a| a.id)
        .ok_or_else(|| ApiError::Tidal("Artist not found".to_string()))?;

    let songs = top_songs_for_artist(&authed, artist_id, count, &base_url).await?;
    Ok(Payload::TopSongs(SongList { song: songs }).into())
}

async fn similar_songs(authed: &Authed, headers: &HeaderMap) -> Result<Vec<SubsonicChild>, ApiError> {
    let id_str = authed
        .params
        .id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest(10, "Missing id".to_string()))?;
    let id: ItemId = id_str
        .parse()
        .map_err(|_| ApiError::BadRequest(0, "Invalid id".to_string()))?;
    let count = authed.params.count.unwrap_or(50).min(100);
    let base_url = base_url_from_headers(headers);

    let client = authed.tidal().await?;
    let artist_id = artist_id_for(&client, &id).await?;
    top_songs_for_artist(authed, artist_id, count, &base_url).await
}

/// `getSimilarSongs` — approximated by the primary artist's top tracks.
pub(crate) async fn handle_get_similar_songs(authed: Authed, headers: HeaderMap) -> ApiResult {
    let songs = similar_songs(&authed, &headers).await?;
    Ok(Payload::SimilarSongs(SongList { song: songs }).into())
}

/// `getSimilarSongs2` — same data under the v2 element name.
pub(crate) async fn handle_get_similar_songs2(authed: Authed, headers: HeaderMap) -> ApiResult {
    let songs = similar_songs(&authed, &headers).await?;
    Ok(Payload::SimilarSongs2(SongList { song: songs }).into())
}

/// Build the artist info (biography + image URLs) for an artist id.
async fn build_artist_info(authed: &Authed, artist_id: u64) -> Result<ArtistInfo, ApiError> {
    let client = authed.tidal().await?;
    let key = MetadataCache::key(authed.user.id, "artistInfo", &format!("id={artist_id}"));
    authed
        .state
        .metadata_cache
        .get_or_build(&key, TTL_CATALOG, || async move {
            let detail = client.get_artist_detail(artist_id).await?;
            let biography = client.get_artist_bio(artist_id).await.unwrap_or_default();
            // Artist picture → Subsonic small/medium/large image URLs.
            let (small, medium, large) = detail
                .picture
                .as_ref()
                .map(|p| {
                    let cid = mapping::cover_art_id(p);
                    (
                        mapping::cover_art_urls(&cid, 160).into_iter().next(),
                        mapping::cover_art_urls(&cid, 320).into_iter().next(),
                        mapping::cover_art_urls(&cid, 640).into_iter().next(),
                    )
                })
                .unwrap_or((None, None, None));
            Ok(ArtistInfo {
                biography,
                music_brainz_id: None,
                last_fm_url: None,
                small_image_url: small,
                medium_image_url: medium,
                large_image_url: large,
                similar_artist: Vec::new(),
            })
        })
        .await
        .map_err(ApiError::Tidal)
}

fn parse_artist_id(id: Option<&str>) -> Result<u64, ApiError> {
    match id.map(|s| s.parse::<ItemId>()) {
        Some(Ok(ItemId::Artist(a))) => Ok(a),
        _ => Err(ApiError::BadRequest(10, "Missing or invalid artist id".to_string())),
    }
}

pub(crate) async fn handle_get_artist_info(authed: Authed) -> ApiResult {
    let artist_id = parse_artist_id(authed.params.id.as_deref())?;
    let info = build_artist_info(&authed, artist_id).await?;
    Ok(Payload::ArtistInfo(info).into())
}

pub(crate) async fn handle_get_artist_info2(authed: Authed) -> ApiResult {
    let artist_id = parse_artist_id(authed.params.id.as_deref())?;
    let info = build_artist_info(&authed, artist_id).await?;
    Ok(Payload::ArtistInfo2(info).into())
}

/// `getAlbumInfo` / `getAlbumInfo2`. TIDAL lacks album notes, so this returns
/// the album's cover-art image URLs (and empty notes).
pub(crate) async fn handle_get_album_info(authed: Authed) -> ApiResult {
    let album_id = match authed.params.id.as_deref().map(|s| s.parse::<ItemId>()) {
        Some(Ok(ItemId::Album(a))) => a,
        _ => return Err(ApiError::BadRequest(10, "Missing or invalid album id".to_string())),
    };
    let client = authed.tidal().await?;
    let album = client.get_album_detail(album_id).await.map_err(ApiError::Tidal)?;
    let (small, medium, large) = album
        .cover
        .as_ref()
        .map(|c| {
            let cid = mapping::cover_art_id(c);
            (
                mapping::cover_art_urls(&cid, 160).into_iter().next(),
                mapping::cover_art_urls(&cid, 320).into_iter().next(),
                mapping::cover_art_urls(&cid, 640).into_iter().next(),
            )
        })
        .unwrap_or((None, None, None));
    Ok(Payload::AlbumInfo(AlbumInfo {
        notes: String::new(),
        music_brainz_id: None,
        small_image_url: small,
        medium_image_url: medium,
        large_image_url: large,
    })
    .into())
}
