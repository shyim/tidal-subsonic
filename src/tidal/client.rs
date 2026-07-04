use crate::db::{self, SharedDb};
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
    /// Serializes token refreshes: TIDAL rotates the refresh token on use, so N
    /// concurrent requests must not all refresh at once (the winner's rotation
    /// would invalidate the losers' refresh token). Held only around a refresh.
    refresh_lock: Mutex<()>,
    client_id: String,
    client_secret: String,
    db: SharedDb,
    /// The Subsonic user this client belongs to — token persistence writes to
    /// this user's `tidal_accounts` row so users can't clobber each other.
    subsonic_user_id: i64,
    cipher: crate::crypto::Cipher,
}

impl TidalClient {
    /// Build a client for a specific Subsonic user from their linked TIDAL
    /// account. `client_id`/`client_secret` are the app-level TIDAL OAuth
    /// credentials (shared across users).
    pub fn for_user(
        subsonic_user_id: i64,
        account: &crate::db::TidalAccount,
        client_id: String,
        client_secret: String,
        db: SharedDb,
        cipher: crate::crypto::Cipher,
    ) -> Self {
        let creds = Creds {
            access_token: account.access_token.clone(),
            refresh_token: account.refresh_token.clone(),
            user_id: account.tidal_user_id,
            country_code: account.country_code.clone(),
        };
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
            creds: Mutex::new(creds),
            refresh_lock: Mutex::new(()),
            client_id,
            client_secret,
            db,
            subsonic_user_id,
            cipher,
        }
    }

    /// Persist the current credentials to this user's `tidal_accounts` row. Call
    /// whenever tokens change so a rotated refresh token survives a restart.
    pub(super) async fn persist_creds(&self) {
        let creds = self.creds.lock().await.clone();
        let account = db::TidalAccount {
            access_token: creds.access_token,
            refresh_token: creds.refresh_token,
            tidal_user_id: creds.user_id,
            country_code: creds.country_code,
        };
        if let Err(e) =
            db::save_tidal_account(&self.db, &self.cipher, self.subsonic_user_id, &account).await
        {
            tracing::warn!(
                "Failed to persist TIDAL tokens for user {}: {}",
                self.subsonic_user_id,
                e
            );
        }
    }

    pub async fn user_id(&self) -> Option<u64> {
        self.creds.lock().await.user_id
    }

    pub async fn access_token(&self) -> String {
        self.creds.lock().await.access_token.clone()
    }

    pub async fn country_code(&self) -> String {
        self.creds.lock().await.country_code.clone()
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
        }

        let tokens: RefreshResp = serde_json::from_str(&body)
            .map_err(|e| format!("Parse error: {} - Body: {}", e, body))?;

        Ok((tokens.access_token, tokens.refresh_token.unwrap_or(current_refresh)))
    }

    /// Refresh the access token, single-flight. `stale_access` is the token the
    /// caller saw fail (empty if it just found no token). Under the refresh lock
    /// we re-check the current token: if it already changed, another task
    /// refreshed while we waited, so we return without hitting TIDAL again — this
    /// prevents a concurrent stampede of refreshes racing TIDAL's token rotation.
    async fn refresh_tokens(&self, stale_access: &str) -> Result<(), String> {
        let _guard = self.refresh_lock.lock().await;

        // Someone else refreshed while we waited for the lock.
        let current = self.creds.lock().await.access_token.clone();
        if current != stale_access && !current.is_empty() {
            return Ok(());
        }

        let (access, refresh) = self.refresh_token_request().await?;
        {
            let mut creds = self.creds.lock().await;
            creds.access_token = access;
            creds.refresh_token = refresh;
        }
        // TIDAL rotates the refresh token; persist so it survives a restart.
        self.persist_creds().await;
        Ok(())
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
            // No access token yet — refresh (single-flight; stale_access is "").
            return self.refresh_tokens("").await;
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

        let response = make_request(token.clone()).await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            // Refresh (single-flight — pass the token that just failed so a
            // concurrent refresh isn't repeated) and retry once.
            self.refresh_tokens(&token).await?;
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

}
