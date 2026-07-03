use super::types::*;
use crate::db::{self, DbConfig, SharedDb};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Sentinel error returned by `get_stream_url` when a track resolves to a
/// segmented DASH manifest that has no single downloadable URL.
pub(super) const DASH_SEGMENTED_ERR: &str = "__dash_segmented__";

/// Tidal audio qualities from highest to lowest. Used as a fallback ladder:
/// HI_RES_LOSSLESS / LOSSLESS often return segmented DASH streams that a
/// single-file Subsonic proxy can't serve, so we step down to a quality that
/// returns a directly downloadable (BTS or single-BaseURL) stream.
pub(super) const QUALITY_LADDER: &[&str] = &["HI_RES_LOSSLESS", "LOSSLESS", "HIGH", "LOW"];

const TIDAL_AUTH_URL: &str = "https://auth.tidal.com/v1/oauth2";
pub(super) const TIDAL_API_URL: &str = "https://api.tidal.com/v1";
pub(super) const TIDAL_API_V2_URL: &str = "https://api.tidal.com/v2";
const TIDAL_OPENAPI_URL: &str = "https://openapi.tidal.com/v2";

/// The mutable credential state of a `TidalClient`. Kept behind a small mutex
/// inside the client so the reqwest client and outer handle can be shared
/// without holding a lock across upstream HTTP round-trips: methods lock this
/// only briefly to read the current token / refresh it, never across `.await`
/// of an upstream request.
#[derive(Debug, Clone)]
pub(in crate::tidal) struct Creds {
    pub(in crate::tidal) access_token: String,
    pub(in crate::tidal) refresh_token: String,
    pub(in crate::tidal) user_id: Option<u64>,
    pub(in crate::tidal) country_code: String,
}

pub type SharedTidalClient = Arc<TidalClient>;

pub struct TidalClient {
    // Visible to the whole `tidal` module tree so the domain `impl` blocks under
    // `tidal::api` can reach the http client and creds without going through an
    // accessor for every call.
    pub(in crate::tidal) client: Client,
    pub(in crate::tidal) creds: Mutex<Creds>,
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
    pub(super) async fn persist_creds(&self) {
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

    pub(super) async fn refresh_token_request(&self) -> Result<(String, String), String> {
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

    pub(super) async fn ensure_auth(&self) -> Result<(), String> {
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

    pub(super) async fn authenticated_get(&self, url: &str, query: &[(&str, &str)]) -> Result<String, String> {
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

    pub(super) async fn api_get<T: serde::de::DeserializeOwned>(
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
}
