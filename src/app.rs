use crate::auth::PkceSession;
use crate::crypto::Cipher;
use crate::db::SharedDb;
use crate::response::ResponseFormat;
use crate::routes::media_cache::MediaCache;
use crate::routes::metadata_cache::MetadataCache;
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
    /// Per-user in-memory cache for browse/search/list responses.
    pub(crate) metadata_cache: MetadataCache,
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
    // getLyrics (classic) params: artist + title of the track.
    #[serde(default)]
    pub(crate) artist: Option<String>,
    #[serde(default)]
    pub(crate) title: Option<String>,
    // Playlist-editing params (createPlaylist/updatePlaylist).
    #[serde(default, rename = "playlistId")]
    pub(crate) playlist_id: Option<String>,
    #[serde(default)]
    pub(crate) comment: Option<String>,
    #[serde(default, rename = "songIdToAdd")]
    pub(crate) song_id_to_add: Option<Vec<String>>,
    #[serde(default, rename = "songIndexToRemove")]
    pub(crate) song_index_to_remove: Option<Vec<u32>>,
}

impl SubsonicParams {
    pub(crate) fn format(&self) -> ResponseFormat {
        ResponseFormat::from_param(self.f.as_deref())
    }

    /// Parse a query string, correctly collecting repeated keys (e.g. `songId=`,
    /// `albumId=`) into the `Vec` fields. `serde_urlencoded` (what axum's `Query`
    /// uses) can't deserialize repeated keys into a `Vec`, so we split the work:
    /// scalar fields go through serde (first value wins), and the known
    /// multi-value keys are collected by hand.
    pub(crate) fn from_query(query: &str) -> Self {
        use std::collections::HashMap;
        // These keys map to Vec fields; serde_urlencoded can't handle them, so
        // exclude them from the scalar query and collect them by hand below.
        const MULTI_KEYS: &[&str] = &["songId", "albumId", "songIdToAdd", "songIndexToRemove"];
        let mut scalars: HashMap<String, String> = HashMap::new();
        let mut multi: HashMap<String, Vec<String>> = HashMap::new();
        for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
            if MULTI_KEYS.contains(&k.as_ref()) {
                multi.entry(k.to_string()).or_default().push(v.to_string());
            } else {
                // First value wins for scalar deserialization.
                scalars.entry(k.to_string()).or_insert_with(|| v.to_string());
            }
        }
        // Re-encode the deduplicated scalars for serde.
        let scalar_query: String = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(scalars.iter())
            .finish();
        let mut params: SubsonicParams =
            serde_urlencoded::from_str(&scalar_query).unwrap_or_default();

        // Overwrite the multi-value fields from the collected lists.
        let take = |key: &str| -> Option<Vec<String>> {
            multi.get(key).filter(|v| !v.is_empty()).cloned()
        };
        params.song_id = take("songId");
        params.album_id = take("albumId");
        params.song_id_to_add = take("songIdToAdd");
        params.song_index_to_remove = take("songIndexToRemove").map(|v| {
            v.iter().filter_map(|s| s.parse::<u32>().ok()).collect()
        });
        params
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_query_parses_scalars_and_auth() {
        let p = SubsonicParams::from_query("u=alice&t=abc123&s=deadbeef&v=1.16.1&c=app&f=json&id=tr-5");
        assert_eq!(p.u, "alice");
        assert_eq!(p.t.as_deref(), Some("abc123"));
        assert_eq!(p.s.as_deref(), Some("deadbeef"));
        assert_eq!(p.id.as_deref(), Some("tr-5"));
        assert_eq!(p.format(), ResponseFormat::Json);
    }

    #[test]
    fn from_query_collects_repeated_keys() {
        let p = SubsonicParams::from_query(
            "u=a&t=x&s=y&songId=tr-1&songId=tr-2&songId=tr-3&albumId=al-9&songIndexToRemove=0&songIndexToRemove=2",
        );
        // Auth fields survive alongside the repeated keys.
        assert_eq!(p.u, "a");
        assert_eq!(p.song_id, Some(vec!["tr-1".into(), "tr-2".into(), "tr-3".into()]));
        assert_eq!(p.album_id, Some(vec!["al-9".into()]));
        assert_eq!(p.song_index_to_remove, Some(vec![0, 2]));
    }

    #[test]
    fn from_query_handles_type_and_percent_encoding() {
        let p = SubsonicParams::from_query("u=u&type=newest&query=daft%20punk");
        assert_eq!(p.list_type.as_deref(), Some("newest"));
        assert_eq!(p.query.as_deref(), Some("daft punk"));
    }
}
