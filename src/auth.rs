use crate::db::{self};
use axum::{
    extract::{Form, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use base64::Engine;
use rand::Rng;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const TIDAL_AUTH_URL: &str = "https://auth.tidal.com/v1/oauth2";
const PKCE_REDIRECT_URI: &str = "https://tidal.com/android/login/auth";
const DEFAULT_CLIENT_ID: &str = "6BDSRdpK9hqEBTgU";

#[derive(Debug, Clone)]
pub struct PkceSession {
    pub code_verifier: String,
    pub client_unique_key: String,
    pub client_id: String,
    pub client_secret: String,
    /// The Subsonic user this OAuth flow is linking a TIDAL account to.
    pub subsonic_user_id: i64,
}

static SETUP_HTML: &str = include_str!("setup.html");

#[derive(Deserialize)]
struct SetupQuery {
    subsonic_username: Option<String>,
    subsonic_password: Option<String>,
}

pub fn auth_routes() -> Router<crate::app::AppState> {
    Router::new()
        .route("/", get(handle_setup_page))
        // POST so the Subsonic password is sent in the request body, not the
        // URL query string (which would be logged / kept in history).
        .route("/authorize", post(handle_authorize))
        .route("/callback", get(handle_callback))
}

async fn handle_setup_page(
    State(_state): State<crate::app::AppState>,
    Query(_params): Query<SetupQuery>,
) -> Response {
    // Multi-user: each user links their own TIDAL account. The setup page asks
    // for their Subsonic credentials, then walks them through TIDAL OAuth.
    let html = SETUP_HTML.to_string();
    (StatusCode::OK, [("content-type", "text/html")], html).into_response()
}

#[axum::debug_handler]
async fn handle_authorize(
    State(state): State<crate::app::AppState>,
    // Credentials arrive in the POST body (axum::Form), keeping the Subsonic
    // password out of the URL/query string. Form must be the last extractor.
    Form(params): Form<SetupQuery>,
) -> Response {
    // Identify WHICH Subsonic user is linking their TIDAL account: verify the
    // submitted username + password against the users table. Anyone who can't
    // prove they own an account can't link (or hijack) a TIDAL session.
    let username = params.subsonic_username.clone().unwrap_or_default();
    let password = params.subsonic_password.clone().unwrap_or_default();
    let user = db::find_user(&state.db, &state.cipher, &username).await;
    let subsonic_user_id = match user {
        Some(u) if !password.is_empty() && u.password == password => u.id,
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                [("content-type", "text/html")],
                r#"<!DOCTYPE html><html><body style="font-family:sans-serif;max-width:600px;margin:60px auto;background:#0a0a0a;color:#eee">
<h1 style="color:#ff6666">Login failed</h1>
<p>Wrong Subsonic username or password. <a href="/" style="color:#00d4aa">Try again</a>.</p>
</body></html>"#,
            )
                .into_response();
        }
    };

    // Use hard-coded client_id, ignore user input
    let client_id = DEFAULT_CLIENT_ID.to_string();
    let client_secret = String::new();

    // Generate PKCE parameters (scoped so rng is dropped before await)
    let (code_verifier, code_challenge, client_unique_key) = {
        let mut rng = rand::thread_rng();
        let random_bytes: [u8; 32] = rng.gen();
        let code_verifier =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes);

        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let code_challenge =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());

        let client_unique_key = format!("{:016x}", rng.gen::<u64>());
        (code_verifier, code_challenge, client_unique_key)
    };

    // Store PKCE session in memory
    {
        let mut sessions = state.pkce_sessions.lock().await;
        sessions.insert(
            client_unique_key.clone(),
            PkceSession {
                code_verifier: code_verifier.clone(),
                client_unique_key: client_unique_key.clone(),
                client_id: client_id.clone(),
                client_secret: client_secret.clone(),
                subsonic_user_id,
            },
        );
    }

    let authorize_url = format!(
        "https://login.tidal.com/authorize?response_type=code&redirect_uri={}&client_id={}&lang=EN&appMode=android&client_unique_key={}&code_challenge={}&code_challenge_method=S256&restrict_signup=true",
        urlencoding(&PKCE_REDIRECT_URI),
        client_id,
        client_unique_key,
        code_challenge,
    );

    // Return a page that shows the auth link and a form to paste the code
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>TIDAL Login</title>
<style>
body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; max-width:600px; margin:60px auto; padding:20px; background:#0a0a0a; color:#eee; }}
h1 {{ color:#00d4aa; }}
.card {{ background:#1a1a1a; border-radius:12px; padding:24px; border:1px solid #333; margin-bottom:16px; }}
.btn {{ display:inline-block; background:#00d4aa; color:#000; padding:12px 24px; border-radius:8px; text-decoration:none; font-weight:bold; font-size:16px; }}
.btn:hover {{ background:#00eebb; }}
input {{ width:100%; padding:12px; margin:8px 0; background:#222; border:1px solid #444; border-radius:8px; color:#fff; font-size:16px; box-sizing:border-box; }}
.submit-btn {{ background:#00d4aa; color:#000; border:none; padding:12px 24px; border-radius:8px; font-weight:bold; font-size:16px; cursor:pointer; width:100%; }}
.submit-btn:hover {{ background:#00eebb; }}
.step {{ margin-bottom:16px; }}
.step-num {{ display:inline-block; background:#333; color:#00d4aa; width:28px; height:28px; border-radius:14px; text-align:center; line-height:28px; margin-right:8px; font-weight:bold; }}
.info {{ color:#888; font-size:14px; margin-top:8px; }}
</style>
</head>
<body>
<h1>🔐 Login with TIDAL</h1>

<div class="card">
<div class="step">
<span class="step-num">1</span>
<b>Open the TIDAL login page:</b>
<p style="margin-top:12px">
<a class="btn" href="{authorize_url}" target="_blank">Open TIDAL Login</a>
</p>
</div>

<div class="step">
<span class="step-num">2</span>
<b>After logging in, TIDAL will redirect you.</b> 
<span class="info">You'll land on a page starting with <code>tidal.com/android/login/auth?code=...</code> – copy the <b>entire URL</b> or just the <code>code=</code> value.</span>
</div>

<div class="step">
<span class="step-num">3</span>
<b>Paste the authorization code here:</b>
<form action="/callback" method="GET">
<input type="text" name="code" placeholder="Paste the code or full redirect URL here" required />
<input type="hidden" name="client_unique_key" value="{client_unique_key}" />
<button class="submit-btn" type="submit">Complete Login</button>
</form>
</div>
</div>

<p class="info">The authorization code is in the URL after <code>code=</code>. You can paste either the full URL or just the code value.</p>

</body>
</html>"#,
        authorize_url = authorize_url,
        client_unique_key = client_unique_key,
    );

    (StatusCode::OK, [("content-type", "text/html")], html).into_response()
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: String,
    client_unique_key: String,
}

#[axum::debug_handler]
async fn handle_callback(
    State(state): State<crate::app::AppState>,
    Query(params): Query<CallbackQuery>,
) -> Response {
    let code = params.code.trim().to_string();
    let client_unique_key = params.client_unique_key.trim().to_string();

    if code.is_empty() || client_unique_key.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing code or session").into_response();
    }

    // Extract code from full URL if needed
    let code = if code.contains("code=") {
        code.split("code=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .unwrap_or(&code)
            .to_string()
    } else {
        code
    };

    // Get PKCE session
    let session = {
        let sessions = state.pkce_sessions.lock().await;
        sessions.get(&client_unique_key).cloned()
    };

    let session = match session {
        Some(s) => s,
        None => {
            let html = r#"<!DOCTYPE html>
<html><body style="font-family:sans-serif;max-width:600px;margin:60px auto;background:#0a0a0a;color:#eee">
<h1 style="color:red">Session Expired</h1>
<p>The login session has expired. Please <a href="/" style="color:#00d4aa">start again</a>.</p>
</body></html>"#;
            return (StatusCode::OK, [("content-type", "text/html")], html).into_response();
        }
    };

    // Exchange code for tokens
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();

    let token_result = exchange_code_for_tokens(
        &client,
        &code,
        &session.code_verifier,
        &session.client_id,
        &session.client_secret,
        &session.client_unique_key,
    )
    .await;

    match token_result {
        Ok((access_token, refresh_token, user_id)) => {
            // Get country code from session
            let cc = get_country_code(&client, &access_token).await.unwrap_or_else(|_| "US".to_string());

            // Link the TIDAL account to the Subsonic user this flow authenticated.
            let account = db::TidalAccount {
                access_token: access_token.clone(),
                refresh_token: refresh_token.clone(),
                tidal_user_id: Some(user_id),
                country_code: cc.clone(),
            };
            let target_user_id = session.subsonic_user_id;
            db::save_tidal_account(&state.db, &state.cipher, target_user_id, &account)
                .await
                .ok();
            // Drop any cached client so the next request rebuilds with fresh tokens.
            state.registry.invalidate(target_user_id).await;
            tracing::info!(
                "Linked TIDAL account (tidal user {}) to Subsonic user {}",
                user_id,
                target_user_id
            );

            // Clear PKCE session
            {
                let mut sessions = state.pkce_sessions.lock().await;
                sessions.remove(&client_unique_key);
            }

            // Show success
            let html = r#"<!DOCTYPE html>
<html><head>
<meta charset="UTF-8">
<meta http-equiv="refresh" content="3;url=/" />
<style>
body { font-family: -apple-system, BlinkMacSystemFont, sans-serif; max-width:600px; margin:60px auto; padding:20px; background:#0a0a0a; color:#eee; }
h1 { color:#00d4aa; }
.card { background:#1a1a1a; border-radius:12px; padding:24px; border:1px solid #333; text-align:center; }
.spinner { display:inline-block; width:40px; height:40px; border:3px solid #333; border-top-color:#00d4aa; border-radius:50%; animation:spin 1s linear infinite; margin-bottom:16px; }
@keyframes spin { to { transform:rotate(360deg); } }
</style>
</head><body>
<div class="card">
<div class="spinner"></div>
<h1>✅ Login Successful!</h1>
<p>Redirecting to your Subsonic connection info...</p>
</div>
</body></html>"#;
            (StatusCode::OK, [("content-type", "text/html")], html).into_response()
        }
        Err(e) => {
            let html = format!(
                r#"<!DOCTYPE html>
<html><body style="font-family:sans-serif;max-width:600px;margin:60px auto;background:#0a0a0a;color:#eee">
<h1 style="color:red">Login Failed</h1>
<p style="color:#ff6666">{}</p>
<p>The authorization code may have expired. <a href="/" style="color:#00d4aa">Try again</a>.</p>
</body></html>"#,
                e
            );
            (StatusCode::OK, [("content-type", "text/html")], html).into_response()
        }
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
    let mut form_params: Vec<(&str, &str)> = vec![
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
        form_params.push(("client_secret", &cs));
    }

    let response = client
        .post(format!("{}/token", TIDAL_AUTH_URL))
        .form(&form_params)
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

    let tokens: TokenResp = serde_json::from_str(&body)
        .map_err(|e| format!("Parse error: {}", e))?;

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
    let sr: SessionResp = serde_json::from_str(&body).unwrap_or(SessionResp { country_code: None });
    Ok(sr.country_code.unwrap_or_else(|| "US".to_string()))
}

fn urlencoding(s: &str) -> String {
    // Simple URL encoding (just for redirect_uri)
    s.replace(":", "%3A").replace("/", "%2F")
}

