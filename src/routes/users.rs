//! Admin-gated user management (Subsonic-standard createUser / updateUser /
//! deleteUser / changePassword / getUsers), so accounts can be managed from any
//! Subsonic client without a bespoke UI.

use crate::auth_mw::{decode_password, xml_ok, ApiError, ApiOk, ApiResult, Authed};
use crate::db::{self, User};
use crate::subsonic::{Payload, SubsonicUser, Users};

/// Reject non-admin callers.
fn require_admin(authed: &Authed) -> Result<(), ApiError> {
    if authed.user.is_admin {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            50,
            "User is not authorized for the given operation".to_string(),
        ))
    }
}

/// A stored user rendered as a Subsonic `<user>` (roles reflect admin status;
/// the proxy grants everyone streaming/playlist but not upload/podcast).
fn to_subsonic_user(u: &User) -> SubsonicUser {
    SubsonicUser {
        username: u.username.clone(),
        email: None,
        scrobbling_enabled: Some(false),
        admin_role: Some(u.is_admin),
        settings_role: Some(u.is_admin),
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
    }
}

pub(crate) async fn handle_create_user(authed: Authed) -> ApiResult {
    require_admin(&authed)?;
    let username = authed
        .params
        .username
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest(10, "Missing username".to_string()))?;
    let password = authed
        .params
        .password
        .as_deref()
        .map(decode_password)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest(10, "Missing password".to_string()))?;
    let is_admin = authed.params.admin_role.unwrap_or(false);

    // Reject duplicate usernames with a clear message.
    if db::find_user(&authed.state.db, &authed.state.cipher, username)
        .await
        .is_some()
    {
        return Err(ApiError::BadRequest(0, "User already exists".to_string()));
    }

    db::create_user(
        &authed.state.db,
        &authed.state.cipher,
        username,
        &password,
        is_admin,
    )
    .await
    .map_err(|e| ApiError::Tidal(format!("Create user failed: {}", e)))?;
    Ok(ApiOk(xml_ok()))
}

pub(crate) async fn handle_delete_user(authed: Authed) -> ApiResult {
    require_admin(&authed)?;
    let username = authed
        .params
        .username
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest(10, "Missing username".to_string()))?;
    // Don't let an admin delete themselves and lock everyone out.
    if username == authed.user.username {
        return Err(ApiError::BadRequest(
            0,
            "Refusing to delete the account you're signed in as".to_string(),
        ));
    }
    match db::delete_user(&authed.state.db, username).await {
        Some(id) => {
            authed.state.registry.invalidate(id).await;
            Ok(ApiOk(xml_ok()))
        }
        None => Err(ApiError::BadRequest(70, "User not found".to_string())),
    }
}

pub(crate) async fn handle_update_user(authed: Authed) -> ApiResult {
    require_admin(&authed)?;
    let username = authed
        .params
        .username
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest(10, "Missing username".to_string()))?;
    let new_password = authed
        .params
        .password
        .as_deref()
        .map(decode_password)
        .filter(|s| !s.is_empty());
    let new_admin = authed.params.admin_role;

    match db::update_user(
        &authed.state.db,
        &authed.state.cipher,
        username,
        new_password.as_deref(),
        new_admin,
    )
    .await
    {
        Some(_) => Ok(ApiOk(xml_ok())),
        None => Err(ApiError::BadRequest(70, "User not found".to_string())),
    }
}

/// `changePassword` — a user may change their own password; an admin may change
/// anyone's.
pub(crate) async fn handle_change_password(authed: Authed) -> ApiResult {
    let username = authed
        .params
        .username
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest(10, "Missing username".to_string()))?;
    if username != authed.user.username && !authed.user.is_admin {
        return Err(ApiError::BadRequest(
            50,
            "User is not authorized for the given operation".to_string(),
        ));
    }
    let password = authed
        .params
        .password
        .as_deref()
        .map(decode_password)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest(10, "Missing password".to_string()))?;
    match db::update_user(
        &authed.state.db,
        &authed.state.cipher,
        username,
        Some(&password),
        None,
    )
    .await
    {
        Some(_) => Ok(ApiOk(xml_ok())),
        None => Err(ApiError::BadRequest(70, "User not found".to_string())),
    }
}

pub(crate) async fn handle_get_users(authed: Authed) -> ApiResult {
    require_admin(&authed)?;
    let users = db::list_users(&authed.state.db, &authed.state.cipher).await;
    let list = Users {
        user: users.iter().map(to_subsonic_user).collect(),
    };
    Ok(Payload::Users(list).into())
}
