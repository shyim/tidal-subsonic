use crate::app::{base_url_from_headers, SubsonicParams};
use crate::auth_mw::{xml_ok, ApiError, ApiOk, ApiResult, Authed};
use crate::item_id::ItemId;
use crate::mapping;
use crate::subsonic::*;
use crate::tidal::SharedTidalClient;
use axum::http::HeaderMap;

pub(crate) async fn handle_get_random_songs(authed: Authed, headers: HeaderMap) -> ApiResult {
    let size = authed.params.size.unwrap_or(10).min(50);
    let client = authed.tidal().await?;
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
    client: &SharedTidalClient,
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

pub(crate) async fn handle_get_album_list(authed: Authed) -> ApiResult {
    let list_type = authed.params.list_type.as_deref().unwrap_or("alphabeticalByName");
    let size = authed.params.size.unwrap_or(10).min(500);
    let offset = authed.params.offset.unwrap_or(0);
    let client = authed.tidal().await?;
    let sub_albums = fetch_album_list(&client, list_type, size, offset)
        .await
        .map_err(ApiError::Tidal)?;
    Ok(Payload::AlbumList(AlbumList { album: sub_albums }).into())
}

pub(crate) async fn handle_get_album_list2(authed: Authed) -> ApiResult {
    let list_type = authed.params.list_type.as_deref().unwrap_or("alphabeticalByName");
    let size = authed.params.size.unwrap_or(10).min(500);
    let offset = authed.params.offset.unwrap_or(0);
    let client = authed.tidal().await?;
    let sub_albums = fetch_album_list(&client, list_type, size, offset)
        .await
        .map_err(ApiError::Tidal)?;
    Ok(Payload::AlbumList2(AlbumList { album: sub_albums }).into())
}

pub(crate) async fn handle_get_starred(authed: Authed, headers: HeaderMap) -> ApiResult {
    let client = authed.tidal().await?;
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

pub(crate) async fn handle_star(authed: Authed) -> ApiResult {
    let targets = collect_star_targets(&authed.params);
    let client = authed.tidal().await?;
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

pub(crate) async fn handle_unstar(authed: Authed) -> ApiResult {
    let targets = collect_star_targets(&authed.params);
    let client = authed.tidal().await?;
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
