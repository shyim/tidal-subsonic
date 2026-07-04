//! TIDAL PKCE OAuth helpers. The web portal's `/api/link/*` endpoints drive the
//! flow: `start_link` generates a PKCE challenge + authorize URL and stashes a
//! pending session; `complete_link` exchanges the pasted code for tokens.

use crate::app::AppState;
use base64::Engine;
use rand::Rng;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const TIDAL_AUTH_URL: &str = "https://auth.tidal.com/v1/oauth2";
const PKCE_REDIRECT_URI: &str = "https://tidal.com/android/login/auth";
const DEFAULT_CLIENT_ID: &str = "6BDSRdpK9hqEBTgU";

/// A pending TIDAL OAuth flow, keyed by `client_unique_key`, awaiting the code.
#[derive(Debug, Clone)]
pub struct PkceSession {
    pub code_verifier: String,
    pub client_unique_key: String,
    pub client_id: String,
    pub client_secret: String,
    /// The Subsonic user this OAuth flow is linking a TIDAL account to.
    pub subsonic_user_id: i64,
}

/// Result of starting a link: the URL the user opens, and the key that ties the
/// eventual code back to this pending session.
pub struct LinkStart {
    pub authorize_url: String,
    pub client_unique_key: String,
}

/// Begin a TIDAL OAuth link for `subsonic_user_id`: generate PKCE, store a
/// pending session, and return the authorize URL + key.
pub async fn start_link(state: &AppState, subsonic_user_id: i64) -> LinkStart {
    let client_id = DEFAULT_CLIENT_ID.to_string();
    let client_secret = String::new();

    // Generate PKCE parameters (scoped so rng drops before any await).
    let (code_verifier, code_challenge, client_unique_key) = {
        let mut rng = rand::rng();
        let random_bytes: [u8; 32] = rng.random();
        let code_verifier =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes);
        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let code_challenge =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());
        // 128-bit unguessable key tying the eventual code back to this session.
        let key_bytes: [u8; 16] = rng.random();
        let client_unique_key: String =
            key_bytes.iter().map(|b| format!("{:02x}", b)).collect();
        (code_verifier, code_challenge, client_unique_key)
    };

    {
        let mut sessions = state.pkce_sessions.lock().await;
        sessions.insert(
            client_unique_key.clone(),
            PkceSession {
                code_verifier,
                client_unique_key: client_unique_key.clone(),
                client_id: client_id.clone(),
                client_secret,
                subsonic_user_id,
            },
        );
    }

    let authorize_url = format!(
        "https://login.tidal.com/authorize?response_type=code&redirect_uri={}&client_id={}&lang=EN&appMode=android&client_unique_key={}&code_challenge={}&code_challenge_method=S256&restrict_signup=true",
        urlencoding(PKCE_REDIRECT_URI),
        client_id,
        client_unique_key,
        code_challenge,
    );

    LinkStart {
        authorize_url,
        client_unique_key,
    }
}

/// Complete a TIDAL link: given the pasted `code` (or full redirect URL) and the
/// `client_unique_key`, exchange for tokens and store them against the pending
/// session's user. `expected_user_id` is the logged-in portal user; the link is
/// refused unless it matches the user who started this PKCE flow — so nobody can
/// complete (or hijack) someone else's pending link. Returns the TIDAL user id.
pub async fn complete_link(
    state: &AppState,
    code_or_url: &str,
    client_unique_key: &str,
    expected_user_id: i64,
) -> Result<u64, String> {
    let client_unique_key = client_unique_key.trim();
    if code_or_url.trim().is_empty() || client_unique_key.is_empty() {
        return Err("Missing code or session".into());
    }

    // Accept either the bare code or the full redirect URL.
    let code = extract_code(code_or_url);

    let session = {
        let sessions = state.pkce_sessions.lock().await;
        sessions.get(client_unique_key).cloned()
    }
    .ok_or_else(|| "Link session expired — start again".to_string())?;

    // The user completing the link must be the one who started it.
    if session.subsonic_user_id != expected_user_id {
        return Err("This link session belongs to a different account".into());
    }

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let (access_token, refresh_token, tidal_user_id) = exchange_code_for_tokens(
        &http,
        &code,
        &session.code_verifier,
        &session.client_id,
        &session.client_secret,
        &session.client_unique_key,
    )
    .await?;

    let cc = get_country_code(&http, &access_token)
        .await
        .unwrap_or_else(|_| "US".to_string());

    let account = crate::db::TidalAccount {
        access_token,
        refresh_token,
        tidal_user_id: Some(tidal_user_id),
        country_code: cc,
    };
    crate::db::save_tidal_account(&state.db, &state.cipher, session.subsonic_user_id, &account)
        .await
        .map_err(|e| e.to_string())?;
    // Drop the cached client so the next request rebuilds with the new tokens.
    state.registry.invalidate(session.subsonic_user_id).await;

    // Clear the pending PKCE session.
    state.pkce_sessions.lock().await.remove(client_unique_key);

    tracing::info!(
        "Linked TIDAL account (tidal user {}) to Subsonic user {}",
        tidal_user_id,
        session.subsonic_user_id
    );
    Ok(tidal_user_id)
}

fn extract_code(input: &str) -> String {
    let input = input.trim();
    if input.contains("code=") {
        input
            .split("code=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .unwrap_or(input)
            .to_string()
    } else {
        input.to_string()
    }
}

async fn exchange_code_for_tokens(
    client: &reqwest::Client,
    code: &str,
    code_verifier: &str,
    client_id: &str,
    client_secret: &str,
    client_unique_key: &str,
) -> Result<(String, String, u64), String> {
    let mut form: Vec<(&str, &str)> = vec![
        ("code", code),
        ("client_id", client_id),
        ("grant_type", "authorization_code"),
        ("redirect_uri", PKCE_REDIRECT_URI),
        ("scope", "r_usr+w_usr+w_sub"),
        ("code_verifier", code_verifier),
        ("client_unique_key", client_unique_key),
    ];
    let cs = client_secret.to_string();
    if !client_secret.is_empty() {
        form.push(("client_secret", &cs));
    }

    let response = client
        .post(format!("{}/token", TIDAL_AUTH_URL))
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("TIDAL API error ({}): {}", status, body));
    }

    #[derive(Deserialize)]
    struct TokenResp {
        access_token: String,
        refresh_token: String,
        #[serde(default)]
        user_id: Option<u64>,
    }
    let tokens: TokenResp =
        serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
    Ok((
        tokens.access_token,
        tokens.refresh_token,
        tokens.user_id.unwrap_or(0),
    ))
}

async fn get_country_code(client: &reqwest::Client, access_token: &str) -> Result<String, String> {
    let resp = client
        .get("https://api.tidal.com/v1/sessions")
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| format!("Session error: {}", e))?;
    let body = resp.text().await.unwrap_or_default();
    #[derive(Deserialize)]
    struct SessionResp {
        #[serde(default)]
        country_code: Option<String>,
    }
    let sr: SessionResp =
        serde_json::from_str(&body).unwrap_or(SessionResp { country_code: None });
    Ok(sr.country_code.unwrap_or_else(|| "US".to_string()))
}

/// Minimal URL-encoding for the redirect_uri.
fn urlencoding(s: &str) -> String {
    s.replace(':', "%3A").replace('/', "%2F")
}
