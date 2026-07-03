use crate::db::{self, DbConfig, SharedDb};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Sentinel error returned by `get_stream_url` when a track resolves to a
/// segmented DASH manifest that has no single downloadable URL.
const DASH_SEGMENTED_ERR: &str = "__dash_segmented__";

/// Tidal audio qualities from highest to lowest. Used as a fallback ladder:
/// HI_RES_LOSSLESS / LOSSLESS often return segmented DASH streams that a
/// single-file Subsonic proxy can't serve, so we step down to a quality that
/// returns a directly downloadable (BTS or single-BaseURL) stream.
const QUALITY_LADDER: &[&str] = &["HI_RES_LOSSLESS", "LOSSLESS", "HIGH", "LOW"];

const TIDAL_AUTH_URL: &str = "https://auth.tidal.com/v1/oauth2";
const TIDAL_API_URL: &str = "https://api.tidal.com/v1";
const TIDAL_API_V2_URL: &str = "https://api.tidal.com/v2";
const TIDAL_OPENAPI_URL: &str = "https://openapi.tidal.com/v2";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TidalTrack {
    pub id: u64,
    pub title: String,
    pub duration: u32,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub artist: Option<TidalArtist>,
    #[serde(default)]
    pub artists: Option<Vec<TidalArtist>>,
    #[serde(default)]
    pub album: Option<TidalAlbum>,
    #[serde(default)]
    pub audio_quality: Option<String>,
    #[serde(default)]
    pub track_number: Option<u32>,
    #[serde(default)]
    pub volume_number: Option<u32>,
    #[serde(default)]
    pub isrc: Option<String>,
    #[serde(default)]
    pub explicit: Option<bool>,
    #[serde(default)]
    pub popularity: Option<u32>,
    #[serde(default)]
    pub replay_gain: Option<f64>,
    #[serde(default)]
    pub peak: Option<f64>,
    #[serde(default)]
    pub copyright: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub stream_ready: Option<bool>,
    #[serde(default)]
    pub allow_streaming: Option<bool>,
    #[serde(default)]
    pub stream_start_date: Option<String>,
    #[serde(default)]
    pub audio_modes: Option<Vec<String>>,
    #[serde(default)]
    pub media_metadata: Option<MediaMetadata>,
    #[serde(default)]
    pub mixes: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetadata {
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalArtist {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub picture: Option<String>,
    #[serde(default, rename = "type")]
    pub artist_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalArtistDetail {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub picture: Option<String>,
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub popularity: Option<u32>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalAlbum {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub vibrant_color: Option<String>,
    #[serde(default)]
    pub video_cover: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalAlbumDetail {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub vibrant_color: Option<String>,
    #[serde(default)]
    pub video_cover: Option<String>,
    #[serde(default)]
    pub artist: Option<TidalArtist>,
    #[serde(default)]
    pub artists: Option<Vec<TidalArtist>>,
    #[serde(default)]
    pub number_of_tracks: Option<u32>,
    #[serde(default)]
    pub number_of_videos: Option<u32>,
    #[serde(default)]
    pub number_of_volumes: Option<u32>,
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub upc: Option<String>,
    #[serde(default, rename = "type")]
    pub album_type: Option<String>,
    #[serde(default)]
    pub copyright: Option<String>,
    #[serde(default)]
    pub explicit: Option<bool>,
    #[serde(default)]
    pub popularity: Option<u32>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub audio_quality: Option<String>,
    #[serde(default)]
    pub stream_ready: Option<bool>,
    #[serde(default)]
    pub allow_streaming: Option<bool>,
    #[serde(default)]
    pub stream_start_date: Option<String>,
    #[serde(default)]
    pub audio_modes: Option<Vec<String>>,
    #[serde(default)]
    pub media_metadata: Option<MediaMetadata>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalPlaylist {
    pub uuid: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub square_image: Option<String>,
    #[serde(default)]
    pub number_of_tracks: Option<u32>,
    #[serde(default)]
    pub creator: Option<TidalPlaylistCreator>,
    #[serde(default, rename = "type")]
    pub playlist_type: Option<String>,
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub last_updated: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub public_playlist: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalPlaylistCreator {
    pub id: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub total_number_of_items: u32,
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub limit: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedTracks {
    pub items: Vec<TidalTrack>,
    pub total_number_of_items: u32,
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub limit: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalSearchResults {
    pub artists: Vec<TidalArtist>,
    pub albums: Vec<TidalAlbumDetail>,
    pub tracks: Vec<TidalTrack>,
    pub playlists: Vec<TidalPlaylist>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub url: String,
    /// For segmented DASH streams: the ordered list of segment URLs (init
    /// segment first) that must be concatenated to reconstruct a playable file.
    /// Empty when `url` is a single directly-downloadable file.
    #[serde(default)]
    pub segments: Vec<String>,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub bit_depth: Option<u32>,
    #[serde(default)]
    pub sample_rate: Option<u32>,
    #[serde(default)]
    pub audio_quality: Option<String>,
    #[serde(default)]
    pub manifest: Option<String>,
    #[serde(default)]
    pub manifest_mime_type: Option<String>,
    #[serde(default)]
    pub album_replay_gain: Option<f64>,
    #[serde(default)]
    pub album_peak_amplitude: Option<f64>,
    #[serde(default)]
    pub track_replay_gain: Option<f64>,
    #[serde(default)]
    pub track_peak_amplitude: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AlbumPageResponse {
    pub album: TidalAlbumDetail,
    pub tracks: Vec<TidalTrack>,
    pub total_tracks: u32,
    pub vibrant_color: Option<String>,
    pub copyright: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TidalLyrics {
    #[serde(default)]
    pub lyrics: Option<String>,
    #[serde(default)]
    pub subtitles: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeviceAuthResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
}

/// The mutable credential state of a `TidalClient`. Kept behind a small mutex
/// inside the client so the reqwest client and outer handle can be shared
/// without holding a lock across upstream HTTP round-trips: methods lock this
/// only briefly to read the current token / refresh it, never across `.await`
/// of an upstream request.
#[derive(Debug, Clone)]
struct Creds {
    access_token: String,
    refresh_token: String,
    user_id: Option<u64>,
    country_code: String,
}

pub type SharedTidalClient = Arc<TidalClient>;

pub struct TidalClient {
    client: Client,
    creds: Mutex<Creds>,
    client_id: String,
    client_secret: String,
    db: SharedDb,
}

impl TidalClient {
    pub fn from_db_config(cfg: &DbConfig, db: SharedDb) -> Self {
        let creds = Creds {
            access_token: cfg.tidal_access_token.clone(),
            refresh_token: cfg.tidal_refresh_token.clone(),
            user_id: cfg.tidal_user_id,
            country_code: cfg.tidal_country_code.clone(),
        };
        let client_id = cfg.tidal_client_id.clone();
        let client_secret = cfg.tidal_client_secret.clone();

        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
            creds: Mutex::new(creds),
            client_id,
            client_secret,
            db,
        }
    }

    pub async fn set_tokens(&self, access_token: String, refresh_token: String, user_id: Option<u64>, country_code: String) {
        let mut creds = self.creds.lock().await;
        creds.access_token = access_token;
        creds.refresh_token = refresh_token;
        creds.user_id = user_id;
        creds.country_code = country_code;
    }

    /// Persist the current credentials to the DB. Call whenever tokens change so
    /// a rotated refresh token survives a restart.
    async fn persist_creds(&self) {
        let creds = self.creds.lock().await.clone();
        if let Err(e) = db::save_tokens(
            &self.db,
            &creds.access_token,
            &creds.refresh_token,
            creds.user_id,
            &creds.country_code,
        )
        .await
        {
            tracing::warn!("Failed to persist TIDAL tokens: {}", e);
        }
    }

    pub async fn is_authenticated(&self) -> bool {
        !self.creds.lock().await.access_token.is_empty()
    }

    pub async fn user_id(&self) -> Option<u64> {
        self.creds.lock().await.user_id
    }

    pub async fn access_token(&self) -> String {
        self.creds.lock().await.access_token.clone()
    }

    pub async fn refresh_token(&self) -> String {
        self.creds.lock().await.refresh_token.clone()
    }

    pub async fn country_code(&self) -> String {
        self.creds.lock().await.country_code.clone()
    }

    // ------ Auth ------ 

    pub async fn start_device_auth(&self) -> Result<DeviceAuthResponse, String> {
        let client_id = &self.client_id;
        if client_id.is_empty() {
            return Err("Client ID not configured. Set it in config.toml".into());
        }

        let response = self
            .client
            .post(format!("{}/device_authorization", TIDAL_AUTH_URL))
            .form(&[
                ("client_id", client_id.as_str()),
                ("scope", "r_usr w_usr w_sub"),
            ])
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(format!("Auth error ({}): {}", status, body));
        }

        #[derive(Deserialize)]
        struct Resp {
            device_code: String,
            user_code: String,
            verification_uri_complete: String,
            expires_in: u64,
        }

        let data: Resp =
            serde_json::from_str(&body).map_err(|e| format!("Parse error: {} - Body: {}", e, body))?;

        Ok(DeviceAuthResponse {
            device_code: data.device_code,
            user_code: data.user_code,
            verification_uri_complete: data.verification_uri_complete,
            expires_in: data.expires_in,
        })
    }

    pub async fn poll_device_token(&self, device_code: &str) -> Result<Option<()>, String> {
        let client_id = &self.client_id;

        let response = self
            .client
            .post(format!("{}/token", TIDAL_AUTH_URL))
            .form(&[
                ("client_id", client_id.as_str()),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("scope", "r_usr w_usr w_sub"),
            ])
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if status.as_u16() == 400
            && (body.contains("authorization_pending") || body.contains("slow_down"))
        {
            return Ok(None);
        }

        if !status.is_success() {
            return Err(format!("Auth error ({}): {}", status, body));
        }

        #[derive(Deserialize)]
        struct TokenResp {
            access_token: String,
            refresh_token: String,
            #[serde(default)]
            user_id: Option<u64>,
        }

        let _tokens: TokenResp = serde_json::from_str(&body)
            .map_err(|e| format!("Parse error: {} - Body: {}", e, body))?;

        // We can't save here because we don't have &self
        Ok(Some(()))
    }

    async fn refresh_token_request(&self) -> Result<(String, String), String> {
        let client_id = &self.client_id;
        let client_secret = &self.client_secret;
        let current_refresh = self.creds.lock().await.refresh_token.clone();

        let mut form_params = vec![
            ("client_id", client_id.as_str()),
            ("refresh_token", current_refresh.as_str()),
            ("grant_type", "refresh_token"),
            ("scope", "r_usr w_usr w_sub"),
        ];
        let cs_present = !client_secret.is_empty();
        if cs_present {
            form_params.push(("client_secret", client_secret));
        }

        let response = self
            .client
            .post(format!("{}/token", TIDAL_AUTH_URL))
            .form(&form_params)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(format!("Refresh error ({}): {}", status, body));
        }

        #[derive(Deserialize)]
        struct RefreshResp {
            access_token: String,
            #[serde(default)]
            refresh_token: Option<String>,
            #[serde(default)]
            user_id: Option<u64>,
        }

        let tokens: RefreshResp = serde_json::from_str(&body)
            .map_err(|e| format!("Parse error: {} - Body: {}", e, body))?;

        Ok((tokens.access_token, tokens.refresh_token.unwrap_or(current_refresh)))
    }

    async fn ensure_auth(&self) -> Result<(), String> {
        let (has_access, has_refresh) = {
            let creds = self.creds.lock().await;
            (!creds.access_token.is_empty(), !creds.refresh_token.is_empty())
        };
        if has_access {
            return Ok(());
        }
        if has_refresh {
            let (access, refresh) = self.refresh_token_request().await?;
            {
                let mut creds = self.creds.lock().await;
                creds.access_token = access;
                creds.refresh_token = refresh;
            }
            // TIDAL rotates the refresh token; persist so it survives a restart.
            self.persist_creds().await;
            return Ok(());
        }
        Err("Not authenticated. Visit the web UI to log in with TIDAL.".into())
    }

    async fn authenticated_get(&self, url: &str, query: &[(&str, &str)]) -> Result<String, String> {
        self.ensure_auth().await?;

        let client = self.client.clone();
        let token = self.access_token().await;
        let url = url.to_string();
        let query: Vec<(String, String)> = query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();

        let make_request = |token: String| {
            let client = client.clone();
            let url = url.clone();
            let query = query.clone();
            async move {
                let resp = client
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", token))
                    .query(&query)
                    .send()
                    .await
                    .map_err(|e| format!("Network error: {}", e))?;
                Ok::<_, String>(resp)
            }
        };

        let response = make_request(token).await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            // Refresh and retry
            let (access, refresh) = self.refresh_token_request().await?;
            {
                let mut creds = self.creds.lock().await;
                creds.access_token = access;
                creds.refresh_token = refresh;
            }
            // TIDAL rotates the refresh token; persist so it survives a restart.
            self.persist_creds().await;
            let response = make_request(self.access_token().await).await?;
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if !status.is_success() {
                return Err(format!("API error ({}): {}", status, body));
            }
            Ok(body)
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if !status.is_success() {
                return Err(format!("API error ({}): {}", status, body));
            }
            Ok(body)
        }
    }

    async fn api_get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, String> {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", TIDAL_API_URL, path)
        };
        let body = self.authenticated_get(&url, query).await?;
        serde_json::from_str(&body).map_err(|e| format!("Parse error: {} - Body: {}", e, &body[..body.len().min(300)]))
    }

    // ------ API Methods ------ 

    pub async fn get_session_info(&self) -> Result<u64, String> {
        let body = self.authenticated_get(
            &format!("{}/sessions", TIDAL_API_URL),
            &[],
        ).await?;

        #[derive(Deserialize)]
        struct SessionResponse {
            user_id: u64,
            #[serde(default)]
            country_code: Option<String>,
        }

        let data: SessionResponse =
            serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        {
            let mut creds = self.creds.lock().await;
            if let Some(cc) = data.country_code {
                if !cc.is_empty() {
                    creds.country_code = cc;
                }
            }
            creds.user_id = Some(data.user_id);
        }
        // Persist the freshly discovered user_id / country_code.
        self.persist_creds().await;
        Ok(data.user_id)
    }

    pub async fn get_user_playlists(
        &self,
        user_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedResponse<TidalPlaylist>, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/users/{}/playlists", TIDAL_API_URL, user_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Resp {
            items: Vec<TidalPlaylist>,
            total_number_of_items: u32,
        }

        let data: Resp = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedResponse {
            items: data.items,
            total_number_of_items: data.total_number_of_items,
            offset,
            limit,
        })
    }

    pub async fn get_playlist(&self, playlist_id: &str) -> Result<TidalPlaylist, String> {
        let cc = self.country_code().await;
        self.api_get(&format!("/playlists/{}", playlist_id), &[("countryCode", &cc)])
            .await
    }

    pub async fn get_playlist_tracks(
        &self,
        playlist_id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedTracks, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/playlists/{}/tracks", TIDAL_API_URL, playlist_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Resp {
            items: Vec<TidalTrack>,
            total_number_of_items: u32,
            #[serde(default)]
            offset: u32,
            #[serde(default)]
            limit: u32,
        }

        let data: Resp = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedTracks {
            items: data.items,
            total_number_of_items: data.total_number_of_items,
            offset: data.offset,
            limit: data.limit,
        })
    }

    pub async fn get_favorite_tracks(
        &self,
        user_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedTracks, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/users/{}/favorites/tracks", TIDAL_API_URL, user_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
                ("order", "DATE"),
                ("orderDirection", "DESC"),
            ],
        ).await?;

        #[derive(Deserialize)]
        struct FavItem {
            item: TidalTrack,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct FavResponse {
            items: Vec<FavItem>,
            total_number_of_items: u32,
            #[serde(default)]
            offset: u32,
            #[serde(default)]
            limit: u32,
        }

        let data: FavResponse = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedTracks {
            items: data.items.into_iter().map(|f| f.item).collect(),
            total_number_of_items: data.total_number_of_items,
            offset: data.offset,
            limit: data.limit,
        })
    }

    pub async fn get_favorite_albums(
        &self,
        user_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedResponse<TidalAlbumDetail>, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/users/{}/favorites/albums", TIDAL_API_URL, user_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
                ("order", "DATE"),
                ("orderDirection", "DESC"),
            ],
        ).await?;

        #[derive(Deserialize)]
        struct FavItem {
            item: TidalAlbumDetail,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct FavResponse {
            items: Vec<FavItem>,
            total_number_of_items: u32,
        }

        let data: FavResponse = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedResponse {
            items: data.items.into_iter().map(|f| f.item).collect(),
            total_number_of_items: data.total_number_of_items,
            offset,
            limit,
        })
    }

    pub async fn get_favorite_artists(
        &self,
        user_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedResponse<TidalArtistDetail>, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/users/{}/favorites/artists", TIDAL_API_URL, user_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
                ("order", "DATE"),
                ("orderDirection", "DESC"),
            ],
        ).await?;

        #[derive(Deserialize)]
        struct FavItem {
            item: TidalArtistDetail,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct FavResponse {
            items: Vec<FavItem>,
            total_number_of_items: u32,
        }

        let data: FavResponse = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedResponse {
            items: data.items.into_iter().map(|f| f.item).collect(),
            total_number_of_items: data.total_number_of_items,
            offset,
            limit,
        })
    }

    /// Add an item to the user's TIDAL favorites. `kind` is the favorites
    /// collection ("tracks" | "albums" | "artists") and `id_param` the matching
    /// id field ("trackIds" | "albumIds" | "artistIds").
    async fn add_favorite(&self, kind: &str, id_param: &str, id: u64) -> Result<(), String> {
        self.ensure_auth().await?;
        let (user_id, cc, token) = {
            let creds = self.creds.lock().await;
            (creds.user_id, creds.country_code.clone(), creds.access_token.clone())
        };
        let user_id = user_id.ok_or("Not authenticated with TIDAL")?;
        let url = format!("{}/users/{}/favorites/{}", TIDAL_API_URL, user_id, kind);
        let id_str = id.to_string();

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .query(&[("countryCode", cc.as_str())])
            .form(&[(id_param, id_str.as_str()), ("onArtifactNotFound", "FAIL")])
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Favorite add error ({}): {}", status, body));
        }
        Ok(())
    }

    /// Remove an item from the user's TIDAL favorites.
    async fn remove_favorite(&self, kind: &str, id: u64) -> Result<(), String> {
        self.ensure_auth().await?;
        let (user_id, cc, token) = {
            let creds = self.creds.lock().await;
            (creds.user_id, creds.country_code.clone(), creds.access_token.clone())
        };
        let user_id = user_id.ok_or("Not authenticated with TIDAL")?;
        let url = format!("{}/users/{}/favorites/{}/{}", TIDAL_API_URL, user_id, kind, id);

        let resp = self
            .client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", token))
            .query(&[("countryCode", cc.as_str())])
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        let status = resp.status();
        if !status.is_success() && status != reqwest::StatusCode::NOT_FOUND {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Favorite remove error ({}): {}", status, body));
        }
        Ok(())
    }

    pub async fn add_favorite_track(&self, id: u64) -> Result<(), String> {
        self.add_favorite("tracks", "trackIds", id).await
    }
    pub async fn add_favorite_album(&self, id: u64) -> Result<(), String> {
        self.add_favorite("albums", "albumIds", id).await
    }
    pub async fn add_favorite_artist(&self, id: u64) -> Result<(), String> {
        self.add_favorite("artists", "artistIds", id).await
    }
    pub async fn remove_favorite_track(&self, id: u64) -> Result<(), String> {
        self.remove_favorite("tracks", id).await
    }
    pub async fn remove_favorite_album(&self, id: u64) -> Result<(), String> {
        self.remove_favorite("albums", id).await
    }
    pub async fn remove_favorite_artist(&self, id: u64) -> Result<(), String> {
        self.remove_favorite("artists", id).await
    }

    pub async fn get_album_detail(&self, album_id: u64) -> Result<TidalAlbumDetail, String> {
        let cc = self.country_code().await;
        self.api_get(&format!("/albums/{}", album_id), &[("countryCode", &cc)]).await
    }

    pub async fn get_album_tracks(
        &self,
        album_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedTracks, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/albums/{}/tracks", TIDAL_API_URL, album_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Resp {
            items: Vec<TidalTrack>,
            total_number_of_items: u32,
            #[serde(default)]
            offset: u32,
            #[serde(default)]
            limit: u32,
        }

        let data: Resp = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedTracks {
            items: data.items,
            total_number_of_items: data.total_number_of_items,
            offset: data.offset,
            limit: data.limit,
        })
    }

    pub async fn get_artist_detail(
        &self,
        artist_id: u64,
    ) -> Result<TidalArtistDetail, String> {
        let cc = self.country_code().await;
        self.api_get(&format!("/artists/{}", artist_id), &[("countryCode", &cc)]).await
    }

    pub async fn get_artist_albums(
        &self,
        artist_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedResponse<TidalAlbumDetail>, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/artists/{}/albums", TIDAL_API_URL, artist_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Resp {
            items: Vec<TidalAlbumDetail>,
            total_number_of_items: u32,
        }

        let data: Resp = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedResponse {
            items: data.items,
            total_number_of_items: data.total_number_of_items,
            offset,
            limit,
        })
    }

    pub async fn get_artist_top_tracks(
        &self,
        artist_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedTracks, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/artists/{}/tracks", TIDAL_API_URL, artist_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Resp {
            items: Vec<TidalTrack>,
            total_number_of_items: u32,
        }

        let data: Resp = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedTracks {
            items: data.items,
            total_number_of_items: data.total_number_of_items,
            offset,
            limit,
        })
    }

    pub async fn search(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<TidalSearchResults, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/search", TIDAL_API_V2_URL),
            &[
                ("query", query),
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("types", "ARTISTS,ALBUMS,TRACKS,PLAYLISTS"),
                ("includeContributors", "true"),
                ("includeUserPlaylists", "true"),
                ("locale", "en_US"),
                ("deviceType", "BROWSER"),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Sec<T> {
            items: Vec<T>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct SR {
            #[serde(default)]
            artists: Option<Sec<TidalArtist>>,
            #[serde(default)]
            albums: Option<Sec<TidalAlbumDetail>>,
            #[serde(default)]
            tracks: Option<Sec<TidalTrack>>,
            #[serde(default)]
            playlists: Option<Sec<TidalPlaylist>>,
        }

        let data: SR = serde_json::from_str(&body)
            .map_err(|e| format!("Search parse error: {}", e))?;

        Ok(TidalSearchResults {
            artists: data.artists.map(|s| s.items).unwrap_or_default(),
            albums: data.albums.map(|s| s.items).unwrap_or_default(),
            tracks: data.tracks.map(|s| s.items).unwrap_or_default(),
            playlists: data.playlists.map(|s| s.items).unwrap_or_default(),
        })
    }

    pub async fn get_track(&self, track_id: u64) -> Result<TidalTrack, String> {
        let cc = self.country_code().await;
        self.api_get(&format!("/tracks/{}", track_id), &[("countryCode", &cc)]).await
    }

    pub async fn get_stream_url(
        &self,
        track_id: u64,
        quality: &str,
    ) -> Result<StreamInfo, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/tracks/{}/playbackinfopostpaywall", TIDAL_API_URL, track_id),
            &[
                ("countryCode", &cc),
                ("audioquality", quality),
                ("playbackmode", "STREAM"),
                ("assetpresentation", "FULL"),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct PlaybackInfo {
            manifest_mime_type: String,
            manifest: String,
            #[serde(default)]
            audio_quality: Option<String>,
            #[serde(default)]
            bit_depth: Option<u32>,
            #[serde(default)]
            sample_rate: Option<u32>,
            #[serde(default)]
            album_replay_gain: Option<f64>,
            #[serde(default)]
            album_peak_amplitude: Option<f64>,
            #[serde(default)]
            track_replay_gain: Option<f64>,
            #[serde(default)]
            track_peak_amplitude: Option<f64>,
        }

        let data: PlaybackInfo = serde_json::from_str(&body)
            .map_err(|e| format!("Parse error: {} - Body: {}", e, body))?;

        let manifest_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &data.manifest,
        ).map_err(|e| format!("Manifest decode error: {}", e))?;

        let manifest_str = String::from_utf8(manifest_bytes)
            .map_err(|e| format!("Invalid manifest: {}", e))?;

        let mut codec: Option<String> = None;
        let url;

        if data.manifest_mime_type.contains("vnd.tidal.bts") {
            #[derive(Deserialize)]
            struct BtsManifest {
                urls: Vec<String>,
                codecs: Option<String>,
            }

            let manifest_data: BtsManifest = serde_json::from_str(&manifest_str)
                .map_err(|e| format!("BTS parse error: {}", e))?;

            codec = manifest_data.codecs
                .map(|c| c.split('.').next().unwrap_or("").to_uppercase());

            url = manifest_data.urls.into_iter().next()
                .ok_or("No URL in BTS manifest".to_string())?;
        } else if data.manifest_mime_type.contains("dash+xml") {
            if let Some(codecs_start) = manifest_str.find("codecs=\"") {
                let start = codecs_start + 8;
                if let Some(codecs_end) = manifest_str[start..].find('\"') {
                    let raw = &manifest_str[start..start + codecs_end];
                    codec = Some(if raw.contains("flac") {
                        "FLAC".to_string()
                    } else {
                        raw.to_uppercase()
                    });
                }
            }
            // First try a single direct BaseURL (some DASH manifests have one).
            url = extract_dash_direct_url(&manifest_str).unwrap_or_default();
            if url.is_empty() {
                // Otherwise reconstruct the segment list from the
                // SegmentTemplate so the caller can concatenate segments into a
                // single playable file.
                let segments = extract_dash_segments(&manifest_str);
                if segments.is_empty() {
                    return Err(DASH_SEGMENTED_ERR.to_string());
                }
                return Ok(StreamInfo {
                    url: String::new(),
                    segments,
                    codec,
                    bit_depth: data.bit_depth,
                    sample_rate: data.sample_rate,
                    audio_quality: data.audio_quality.clone(),
                    manifest: Some(manifest_str),
                    manifest_mime_type: Some(data.manifest_mime_type),
                    album_replay_gain: data.album_replay_gain,
                    album_peak_amplitude: data.album_peak_amplitude,
                    track_replay_gain: data.track_replay_gain,
                    track_peak_amplitude: data.track_peak_amplitude,
                });
            }
        } else {
            return Err(format!("Unknown manifest format: {}", data.manifest_mime_type));
        }

        if url.is_empty() {
            return Err("No stream URL found".into());
        }

        Ok(StreamInfo {
            url,
            segments: vec![],
            codec,
            bit_depth: data.bit_depth,
            sample_rate: data.sample_rate,
            audio_quality: data.audio_quality.clone(),
            manifest: Some(manifest_str),
            manifest_mime_type: Some(data.manifest_mime_type),
            album_replay_gain: data.album_replay_gain,
            album_peak_amplitude: data.album_peak_amplitude,
            track_replay_gain: data.track_replay_gain,
            track_peak_amplitude: data.track_peak_amplitude,
        })
    }

    /// Resolve a directly-streamable URL for a track, capped at `max_quality`.
    /// Walks the quality ladder downward from the requested ceiling until Tidal
    /// returns a single downloadable file (BTS or single-BaseURL DASH), so that
    /// HI-RES tracks — which come back as segmented DASH — still play through
    /// the proxy at the best format that can be served as one file.
    pub async fn get_streamable_url(
        &self,
        track_id: u64,
        max_quality: &str,
    ) -> Result<StreamInfo, String> {
        // Start at the requested ceiling, then step down.
        let start = QUALITY_LADDER
            .iter()
            .position(|q| *q == max_quality)
            .unwrap_or(0);

        let mut last_err = String::from("No stream URL found");
        for quality in &QUALITY_LADDER[start..] {
            match self.get_stream_url(track_id, quality).await {
                Ok(info) => return Ok(info),
                Err(e) if e == DASH_SEGMENTED_ERR => {
                    tracing::debug!(
                        "Track {} segmented DASH at quality {}, trying lower quality",
                        track_id,
                        quality
                    );
                    last_err = "All qualities returned segmented DASH streams".to_string();
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err)
    }

    pub async fn get_track_lyrics(&self, track_id: u64) -> Result<TidalLyrics, String> {
        let cc = self.country_code().await;
        self.api_get(&format!("/tracks/{}/lyrics", track_id), &[("countryCode", &cc)]).await
    }
}

fn extract_dash_direct_url(manifest: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(manifest).ok()?;

    let mut current_url: Option<String> = None;

    for node in doc.descendants().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "BaseURL" => {
                if let Some(text) = node.text() {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        if let Ok(parsed) = reqwest::Url::parse(&text) {
                            current_url = Some(parsed.to_string());
                        } else if let Some(ref base) = current_url {
                            if let Ok(parsed) = reqwest::Url::parse(base) {
                                if let Ok(joined) = parsed.join(&text) {
                                    current_url = Some(joined.to_string());
                                }
                            }
                        }
                    }
                }
            }
            "Representation" => {
                // Check if this representation has SegmentTemplate or SegmentList - skip those
                let has_segments = node
                    .children()
                    .any(|c| {
                        c.is_element()
                            && (c.tag_name().name() == "SegmentTemplate"
                                || c.tag_name().name() == "SegmentList")
                    });
                if has_segments {
                    continue;
                }

                // Look for BaseURL in this representation
                let url = node
                    .children()
                    .find(|c| c.is_element() && c.tag_name().name() == "BaseURL")
                    .and_then(|c| c.text())
                    .map(|t| t.trim().to_string())
                    .or_else(|| current_url.clone());

                if let Some(u) = url {
                    if is_direct_http_media_url(&u) {
                        return Some(u);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn is_direct_http_media_url(raw: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let path = url.path().to_ascii_lowercase();
    !path.ends_with('/') && !path.ends_with(".mpd")
}

/// Reconstruct the ordered list of segment URLs from a DASH `SegmentTemplate`
/// (as used by TIDAL). The returned list is the init segment followed by every
/// media segment; concatenating their bytes yields a single playable MP4/M4A.
///
/// Handles the `$Number$` placeholder with `startNumber` and a `SegmentTimeline`
/// that expresses segment counts via `<S d=".." r="N"/>` (repeat) entries.
fn extract_dash_segments(manifest: &str) -> Vec<String> {
    let Ok(doc) = roxmltree::Document::parse(manifest) else {
        return vec![];
    };

    for tmpl in doc
        .descendants()
        .filter(|n| n.has_tag_name("SegmentTemplate"))
    {
        let Some(media) = tmpl.attribute("media") else {
            continue;
        };
        let init = tmpl.attribute("initialization");
        let start_number: u64 = tmpl
            .attribute("startNumber")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        // Count media segments from the SegmentTimeline: each <S> contributes
        // 1 + r segments (r defaults to 0).
        let mut count: u64 = 0;
        if let Some(timeline) = tmpl
            .children()
            .find(|c| c.has_tag_name("SegmentTimeline"))
        {
            for s in timeline.children().filter(|c| c.has_tag_name("S")) {
                let r: i64 = s.attribute("r").and_then(|v| v.parse().ok()).unwrap_or(0);
                // r can be negative (unknown/until-end); treat as a single segment.
                count += 1 + r.max(0) as u64;
            }
        }

        if count == 0 || !media.contains("$Number$") {
            continue;
        }

        let mut urls = Vec::with_capacity(count as usize + 1);
        if let Some(init) = init {
            urls.push(init.to_string());
        }
        for i in 0..count {
            let n = start_number + i;
            urls.push(media.replace("$Number$", &n.to_string()));
        }
        return urls;
    }

    vec![]
}
