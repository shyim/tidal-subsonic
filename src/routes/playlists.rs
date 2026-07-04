use crate::app::base_url_from_headers;
use crate::auth_mw::{ApiError, ApiResult, Authed};
use crate::item_id::ItemId;
use crate::mapping;
use crate::subsonic::*;
use axum::http::HeaderMap;

pub(crate) async fn handle_get_playlists(authed: Authed) -> ApiResult {
    let client = authed.tidal().await?;
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

pub(crate) async fn handle_get_playlist(authed: Authed, headers: HeaderMap) -> ApiResult {
    let playlist_id_str = authed
        .params
        .id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest(10, "Missing playlist id".to_string()))?;
    let playlist_uuid = match playlist_id_str.parse::<ItemId>() {
        Ok(ItemId::Playlist(uuid)) => uuid,
        _ => return Err(ApiError::BadRequest(0, "Invalid playlist id".to_string())),
    };

    let client = authed.tidal().await?;
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

pub(crate) async fn handle_create_playlist(_authed: Authed) -> ApiResult {
    // Playlist creation not implemented via Subsonic API
    Err(ApiError::Tidal(
        "Playlist creation not supported via Subsonic API. Use the TIDAL app.".to_string(),
    ))
}

pub(crate) async fn handle_update_playlist(_authed: Authed) -> ApiResult {
    Err(ApiError::Tidal(
        "Playlist updates not supported via Subsonic API.".to_string(),
    ))
}

pub(crate) async fn handle_delete_playlist(_authed: Authed) -> ApiResult {
    Err(ApiError::Tidal(
        "Playlist deletion not supported via Subsonic API.".to_string(),
    ))
}
