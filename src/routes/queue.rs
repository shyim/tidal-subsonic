//! Play queue + bookmarks — per-user "resume where you left off" across devices.
//! Backed by the `play_queue` and `bookmarks` tables (scoped by Subsonic user).

use crate::app::base_url_from_headers;
use crate::auth_mw::{xml_ok, ApiError, ApiOk, ApiResult, Authed};
use crate::db;
use crate::item_id::ItemId;
use crate::mapping;
use crate::subsonic::*;
use crate::tidal::SharedTidalClient;
use axum::http::HeaderMap;

/// Resolve a Subsonic track id (`tr-N`) to a `SubsonicChild` via TIDAL.
async fn track_child(
    client: &SharedTidalClient,
    id: &str,
    base_url: &str,
) -> Option<SubsonicChild> {
    match id.parse::<ItemId>() {
        Ok(ItemId::Track(tid)) => client
            .get_track(tid)
            .await
            .ok()
            .map(|t| mapping::track_to_child(&t, base_url)),
        _ => None,
    }
}

pub(crate) async fn handle_save_play_queue(authed: Authed) -> ApiResult {
    // Subsonic sends the queue as repeated id= params (collected into `id` only
    // keeps the last one), so accept both `id` and `songId[]` where present.
    let ids: Vec<String> = authed
        .params
        .song_id
        .clone()
        .or_else(|| authed.params.id.clone().map(|i| vec![i]))
        .unwrap_or_default();
    // Empty save clears the queue.
    let current = authed.params.id.clone();
    // `position` arrives via the generic time/position param; Subsonic uses
    // `position` (ms). Reuse the `time` field which we parse as u64.
    let position_ms = authed.params.time.unwrap_or(0);
    let changed_by = authed.params.c.clone();

    db::save_play_queue(
        &authed.state.db,
        authed.user.id,
        &ids,
        current.as_deref(),
        position_ms,
        changed_by.as_deref(),
    )
    .await
    .map_err(|e| ApiError::Tidal(format!("Save play queue failed: {}", e)))?;
    Ok(ApiOk(xml_ok()))
}

pub(crate) async fn handle_get_play_queue(authed: Authed, headers: HeaderMap) -> ApiResult {
    let Some(row) = db::load_play_queue(&authed.state.db, authed.user.id).await else {
        // No saved queue → an empty OK response (no playQueue element).
        return Ok(ApiOk(xml_ok()));
    };
    let base_url = base_url_from_headers(&headers);
    let client = authed.tidal().await?;
    let mut entries = Vec::new();
    for id in &row.track_ids {
        if let Some(child) = track_child(&client, id, &base_url).await {
            entries.push(child);
        }
    }
    Ok(Payload::PlayQueue(PlayQueue {
        current: row.current,
        position: Some(row.position_ms),
        username: authed.user.username.clone(),
        changed: Some(row.changed_at),
        changed_by: row.changed_by,
        entry: entries,
    })
    .into())
}

pub(crate) async fn handle_create_bookmark(authed: Authed) -> ApiResult {
    let id = authed
        .params
        .id
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest(10, "Missing id".to_string()))?;
    let position_ms = authed.params.time.unwrap_or(0);
    let comment = authed.params.comment.as_deref();
    db::save_bookmark(&authed.state.db, authed.user.id, id, position_ms, comment)
        .await
        .map_err(|e| ApiError::Tidal(format!("Create bookmark failed: {}", e)))?;
    Ok(ApiOk(xml_ok()))
}

pub(crate) async fn handle_delete_bookmark(authed: Authed) -> ApiResult {
    let id = authed
        .params
        .id
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest(10, "Missing id".to_string()))?;
    db::delete_bookmark(&authed.state.db, authed.user.id, id)
        .await
        .map_err(|e| ApiError::Tidal(format!("Delete bookmark failed: {}", e)))?;
    Ok(ApiOk(xml_ok()))
}

pub(crate) async fn handle_get_bookmarks(authed: Authed, headers: HeaderMap) -> ApiResult {
    let rows = db::list_bookmarks(&authed.state.db, authed.user.id).await;
    let base_url = base_url_from_headers(&headers);
    let client = authed.tidal().await?;
    let mut bookmarks = Vec::new();
    for row in rows {
        if let Some(entry) = track_child(&client, &row.track_id, &base_url).await {
            bookmarks.push(Bookmark {
                position: row.position_ms,
                username: authed.user.username.clone(),
                comment: row.comment,
                created: Some(row.created_at),
                changed: Some(row.changed_at),
                entry,
            });
        }
    }
    Ok(Payload::Bookmarks(Bookmarks { bookmark: bookmarks }).into())
}
