use crate::app::base_url_from_headers;
use crate::auth_mw::{ApiError, ApiResult, Authed};
use crate::item_id::ItemId;
use crate::mapping;
use crate::subsonic::*;
use axum::http::HeaderMap;

pub(crate) async fn handle_get_indexes(authed: Authed) -> ApiResult {
    let client = authed.tidal();
    let user_id = authed.tidal_user_id().await?;
    let artists = client
        .get_favorite_artists(user_id, 0, 500)
        .await
        .map_err(ApiError::Tidal)?;
    Ok(Payload::Indexes(mapping::build_indexes(&artists.items)).into())
}

pub(crate) async fn handle_get_artists(authed: Authed) -> ApiResult {
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

pub(crate) async fn handle_get_artist(authed: Authed) -> ApiResult {
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

pub(crate) async fn handle_get_album(authed: Authed, headers: HeaderMap) -> ApiResult {
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
pub(crate) async fn handle_get_music_directory(authed: Authed, headers: HeaderMap) -> ApiResult {
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

pub(crate) async fn handle_get_song(authed: Authed, headers: HeaderMap) -> ApiResult {
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
