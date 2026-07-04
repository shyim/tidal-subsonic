use crate::app::base_url_from_headers;
use crate::auth_mw::{ApiError, ApiOk, ApiResult, Authed};
use crate::item_id::ItemId;
use crate::mapping;
use crate::routes::metadata_cache::{MetadataCache, TTL_FAVORITES};
use crate::subsonic::*;
use crate::tidal::SharedTidalClient;
use axum::http::HeaderMap;

pub(crate) async fn handle_get_playlists(authed: Authed) -> ApiResult {
    let client = authed.tidal().await?;
    let user_id = authed.tidal_user_id().await?;
    let key = MetadataCache::key(authed.user.id, "getPlaylists", "");
    let wrapper = authed
        .state
        .metadata_cache
        .get_or_build(&key, TTL_FAVORITES, || async {
            let playlists = client.get_all_user_playlists(user_id).await?;
            let mut sub_playlists: Vec<SubsonicPlaylist> = playlists
                .iter()
                .map(mapping::playlist_to_subsonic)
                .collect();
            // Append TIDAL's generated mixes (Daily Mix, My Mix, …) as playlists.
            // Best-effort: if the mixes endpoint fails, still return the real ones.
            if let Ok(mixes) = client.get_my_mixes().await {
                sub_playlists.extend(mixes.iter().map(mapping::mix_to_subsonic_playlist));
            }
            Ok(PlaylistsWrapper {
                playlist: sub_playlists,
            })
        })
        .await
        .map_err(ApiError::Tidal)?;
    Ok(Payload::Playlists(wrapper).into())
}

/// Fetch a playlist's metadata and tracks and map it to the Subsonic
/// `getPlaylist` shape. Shared by `getPlaylist` and `createPlaylist` (which
/// returns the freshly-created playlist in the same shape).
async fn build_playlist_response(
    client: &SharedTidalClient,
    playlist_uuid: &str,
    base_url: &str,
) -> Result<PlaylistWithSongs, ApiError> {
    let meta = client.get_playlist(playlist_uuid).await.ok();
    let tracks = client
        .get_all_playlist_tracks(playlist_uuid)
        .await
        .map_err(ApiError::Tidal)?;
    let songs: Vec<SubsonicChild> = tracks
        .iter()
        .map(|t| mapping::track_to_child(t, base_url))
        .collect();
    let cover_art = meta
        .as_ref()
        .map(mapping::playlist_to_subsonic)
        .and_then(|p| p.cover_art)
        .or_else(|| songs.first().and_then(|s| s.cover_art.clone()));
    Ok(PlaylistWithSongs {
        id: ItemId::Playlist(playlist_uuid.to_string()).to_string(),
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
        // Normalize to strict RFC 3339 — go-subsonic (Supersonic) errors the whole
        // response otherwise. Matches the getPlaylists list path.
        created: meta
            .as_ref()
            .and_then(|p| p.created.clone())
            .map(|d| mapping::to_rfc3339(&d)),
        changed: meta
            .as_ref()
            .and_then(|p| p.last_updated.clone())
            .map(|d| mapping::to_rfc3339(&d)),
        cover_art,
        entry: songs,
    })
}

/// Parse a `pl-`-prefixed Subsonic id into a TIDAL playlist uuid.
fn parse_playlist_id(id: &str) -> Result<String, ApiError> {
    match id.parse::<ItemId>() {
        Ok(ItemId::Playlist(uuid)) => Ok(uuid),
        _ => Err(ApiError::BadRequest(0, "Invalid playlist id".to_string())),
    }
}

/// Parse a `tr-`-prefixed Subsonic song id into a TIDAL numeric track id.
fn parse_track_id(id: &str) -> Option<u64> {
    match id.parse::<ItemId>() {
        Ok(ItemId::Track(tid)) => Some(tid),
        _ => None,
    }
}

pub(crate) async fn handle_get_playlist(authed: Authed, headers: HeaderMap) -> ApiResult {
    let id_str = authed
        .params
        .id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest(10, "Missing playlist id".to_string()))?;

    let client = authed.tidal().await?;
    let base_url = base_url_from_headers(&headers);

    // A `mix-` id is one of TIDAL's generated mixes — fetch its tracks from the
    // mix endpoint rather than the user-playlist endpoint.
    let playlist = match id_str.parse::<ItemId>() {
        Ok(ItemId::Mix(mix_id)) => build_mix_response(&client, &mix_id, &base_url).await?,
        _ => {
            let uuid = parse_playlist_id(id_str)?;
            build_playlist_response(&client, &uuid, &base_url).await?
        }
    };
    Ok(Payload::Playlist(playlist).into())
}

/// Build the `getPlaylist` response for a TIDAL generated mix.
async fn build_mix_response(
    client: &SharedTidalClient,
    mix_id: &str,
    base_url: &str,
) -> Result<PlaylistWithSongs, ApiError> {
    let detail = client
        .get_mix_tracks(mix_id)
        .await
        .map_err(ApiError::Tidal)?;
    let songs: Vec<SubsonicChild> = detail
        .tracks
        .iter()
        .map(|t| mapping::track_to_child(t, base_url))
        .collect();
    let cover_art = songs.first().and_then(|s| s.cover_art.clone());
    Ok(PlaylistWithSongs {
        id: ItemId::Mix(mix_id.to_string()).to_string(),
        name: detail.title.unwrap_or_else(|| "Mix".to_string()),
        comment: None,
        owner: Some("TIDAL".to_string()),
        public: Some(false),
        song_count: Some(songs.len() as u32),
        duration: Some(songs.iter().filter_map(|s| s.duration).sum()),
        created: None,
        changed: None,
        cover_art,
        entry: songs,
    })
}

pub(crate) async fn handle_create_playlist(authed: Authed, headers: HeaderMap) -> ApiResult {
    let name = authed
        .params
        .name
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest(10, "Missing playlist name".to_string()))?;

    let client = authed.tidal().await?;
    let created = client
        .create_playlist(name, "")
        .await
        .map_err(ApiError::Tidal)?;

    // Populate the new playlist with any provided songs.
    let track_ids: Vec<u64> = authed
        .params
        .song_id
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter_map(|id| parse_track_id(id))
        .collect();
    if !track_ids.is_empty() {
        client
            .add_tracks_to_playlist(&created.uuid, &track_ids)
            .await
            .map_err(ApiError::Tidal)?;
    }

    // The playlist set changed: drop this user's cached getPlaylists view.
    authed.state.metadata_cache.invalidate_user(authed.user.id).await;

    let base_url = base_url_from_headers(&headers);
    let playlist = build_playlist_response(&client, &created.uuid, &base_url).await?;
    Ok(Payload::Playlist(playlist).into())
}

pub(crate) async fn handle_update_playlist(authed: Authed) -> ApiResult {
    let playlist_id_str = authed
        .params
        .playlist_id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest(10, "Missing playlistId".to_string()))?;
    let playlist_uuid = parse_playlist_id(playlist_id_str)?;

    let client = authed.tidal().await?;

    // Rename / re-describe if either was supplied.
    let name = authed.params.name.as_deref();
    let comment = authed.params.comment.as_deref();
    if name.is_some() || comment.is_some() {
        client
            .update_playlist(&playlist_uuid, name, comment)
            .await
            .map_err(ApiError::Tidal)?;
    }

    // Add any requested tracks.
    let track_ids: Vec<u64> = authed
        .params
        .song_id_to_add
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter_map(|id| parse_track_id(id))
        .collect();
    if !track_ids.is_empty() {
        client
            .add_tracks_to_playlist(&playlist_uuid, &track_ids)
            .await
            .map_err(ApiError::Tidal)?;
    }

    // Remove tracks by index. Removing shifts later indices down, so delete from
    // the highest index first to keep the remaining indices valid.
    if let Some(indices) = &authed.params.song_index_to_remove {
        let mut indices = indices.clone();
        indices.sort_unstable();
        indices.dedup();
        for index in indices.into_iter().rev() {
            client
                .remove_track_from_playlist(&playlist_uuid, index)
                .await
                .map_err(ApiError::Tidal)?;
        }
    }

    authed.state.metadata_cache.invalidate_user(authed.user.id).await;
    Ok(ApiOk(SubsonicResponse::ok()))
}

pub(crate) async fn handle_delete_playlist(authed: Authed) -> ApiResult {
    let playlist_id_str = authed
        .params
        .id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest(10, "Missing playlist id".to_string()))?;
    let playlist_uuid = parse_playlist_id(playlist_id_str)?;

    let client = authed.tidal().await?;
    client
        .delete_playlist(&playlist_uuid)
        .await
        .map_err(ApiError::Tidal)?;

    authed.state.metadata_cache.invalidate_user(authed.user.id).await;
    Ok(ApiOk(SubsonicResponse::ok()))
}
