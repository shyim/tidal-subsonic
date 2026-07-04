use crate::auth::PkceSession;
use crate::crypto::Cipher;
use crate::db::SharedDb;
use crate::response::ResponseFormat;
use crate::routes::media_cache::MediaCache;
use crate::tidal::ClientRegistry;
use axum::http::HeaderMap;
use reqwest::Client as ReqwestClient;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct AppState {
    /// Per-user TIDAL clients — resolve one for the authenticated user.
    pub(crate) registry: ClientRegistry,
    pub(crate) db: SharedDb,
    pub(crate) cipher: Cipher,
    pub(crate) http_client: ReqwestClient,
    pub(crate) pkce_sessions: Arc<tokio::sync::Mutex<HashMap<String, PkceSession>>>,
    pub(crate) max_quality: String,
    pub(crate) media_cache: MediaCache,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) struct SubsonicParams {
    #[serde(default)]
    pub(crate) u: String, // username
    #[serde(default)]
    pub(crate) t: Option<String>, // token = md5(password + salt)
    #[serde(default)]
    pub(crate) s: Option<String>, // salt (random)
    #[serde(default)]
    pub(crate) p: Option<String>, // plaintext or "enc:"-hex password (legacy clients)
    #[serde(default)]
    pub(crate) v: Option<String>, // API version
    #[serde(default)]
    pub(crate) c: Option<String>, // client name
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    #[serde(rename = "type")]
    pub(crate) list_type: Option<String>,
    #[serde(default)]
    pub(crate) size: Option<u32>,
    #[serde(default)]
    pub(crate) offset: Option<u32>,
    #[serde(default)]
    pub(crate) count: Option<u32>,
    #[serde(default)]
    pub(crate) query: Option<String>,
    #[serde(default)]
    #[serde(rename = "artistCount")]
    pub(crate) artist_count: Option<u32>,
    #[serde(default)]
    #[serde(rename = "artistOffset")]
    pub(crate) artist_offset: Option<u32>,
    #[serde(default)]
    #[serde(rename = "albumCount")]
    pub(crate) album_count: Option<u32>,
    #[serde(default)]
    #[serde(rename = "albumOffset")]
    pub(crate) album_offset: Option<u32>,
    #[serde(default)]
    #[serde(rename = "songCount")]
    pub(crate) song_count: Option<u32>,
    #[serde(default)]
    #[serde(rename = "songOffset")]
    pub(crate) song_offset: Option<u32>,
    #[serde(default)]
    #[serde(rename = "musicFolderId")]
    pub(crate) music_folder_id: Option<String>,
    #[serde(default)]
    #[serde(rename = "fromYear")]
    pub(crate) from_year: Option<String>,
    #[serde(default)]
    #[serde(rename = "toYear")]
    pub(crate) to_year: Option<String>,
    #[serde(default)]
    pub(crate) genre: Option<String>,
    #[serde(default)]
    #[serde(rename = "minBitRate")]
    pub(crate) min_bit_rate: Option<u32>,
    #[serde(default)]
    #[serde(rename = "maxBitRate")]
    pub(crate) max_bit_rate: Option<u32>,
    #[serde(default)]
    #[serde(rename = "ifModifiedSince")]
    pub(crate) if_modified_since: Option<u64>,
    #[serde(default)]
    #[serde(rename = "newestMethod")]
    pub(crate) newest_method: Option<String>,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    #[serde(rename = "songId")]
    pub(crate) song_id: Option<Vec<String>>,
    #[serde(default)]
    #[serde(rename = "albumId")]
    pub(crate) album_id: Option<Vec<String>>,
    #[serde(default)]
    #[serde(rename = "artistId")]
    pub(crate) artist_id_str: Option<String>,
    #[serde(default)]
    pub(crate) submission: Option<bool>,
    #[serde(default)]
    pub(crate) time: Option<u64>,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) f: Option<String>,
    #[serde(default)]
    pub(crate) callback: Option<String>,
    #[serde(default)]
    pub(crate) format: Option<String>,
    // User-management params (createUser/updateUser/changePassword/deleteUser).
    // `username` is the *target* user; `password` is the target's new password
    // (distinct from the `p=` auth param, which is the caller's own password).
    #[serde(default)]
    pub(crate) username: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) email: Option<String>,
    #[serde(default, rename = "adminRole")]
    pub(crate) admin_role: Option<bool>,
}

impl SubsonicParams {
    pub(crate) fn format(&self) -> ResponseFormat {
        ResponseFormat::from_param(self.f.as_deref())
    }
}

pub(crate) fn base_url_from_headers(headers: &HeaderMap) -> String {
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
