//! HTTP-level integration tests: build the real app router over a temp DB and
//! drive it in-process with `tower::ServiceExt::oneshot` (no TCP bind, no live
//! TIDAL). These pin down the auth surface, the portal session flow, admin
//! isolation, and routing precedence — the things we otherwise verify by hand.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt; // for `oneshot`

use tidal_subsonic::{build_router, build_state, db};

/// Spin up the full app over a fresh temp DB with a known admin + normal user.
async fn test_app() -> (Router, tempdir::TempGuard) {
    let guard = tempdir::TempGuard::new();
    let db_path = guard.path().join("test.db");
    let database = db::open_db(&db_path).expect("open db");
    let cipher = db::master_cipher(&database).await;

    db::create_user(&database, &cipher, "admin", "adminpw", true)
        .await
        .expect("create admin");
    db::create_user(&database, &cipher, "bob", "bobpw", false)
        .await
        .expect("create bob");

    let cfg = db::DbConfig::default();
    let state = build_state(database, cipher, &cfg).await;
    (build_router(state), guard)
}

/// md5(password + salt) — Subsonic token auth.
fn token(password: &str, salt: &str) -> String {
    format!("{:x}", md5::compute(format!("{}{}", password, salt)))
}

async fn get(app: &Router, uri: &str) -> (StatusCode, String) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

async fn post_json(app: &Router, uri: &str, body: &str) -> (StatusCode, String, Option<String>) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let cookie = resp
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string(), cookie)
}

// ---- Subsonic /rest auth ----

#[tokio::test]
async fn ping_with_valid_token_returns_ok_json() {
    let (app, _g) = test_app().await;
    let salt = "abcd";
    let t = token("adminpw", salt);
    let (status, body) = get(
        &app,
        &format!("/rest/ping?u=admin&t={t}&s={salt}&v=1.16.1&c=test&f=json"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"status\":\"ok\""), "body was: {body}");
}

#[tokio::test]
async fn ping_view_alias_also_works() {
    let (app, _g) = test_app().await;
    let salt = "zz";
    let t = token("adminpw", salt);
    let (status, body) = get(
        &app,
        &format!("/rest/ping.view?u=admin&t={t}&s={salt}&v=1.16.1&c=test&f=json"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"status\":\"ok\""));
}

#[tokio::test]
async fn ping_xml_format_is_default() {
    let (app, _g) = test_app().await;
    let salt = "q1";
    let t = token("adminpw", salt);
    let (status, body) = get(
        &app,
        &format!("/rest/ping?u=admin&t={t}&s={salt}&v=1.16.1&c=test"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<subsonic-response"), "body was: {body}");
    assert!(body.contains("status=\"ok\""));
}

#[tokio::test]
async fn ping_with_wrong_password_fails() {
    let (app, _g) = test_app().await;
    let salt = "abcd";
    let t = token("WRONG", salt);
    let (status, body) = get(
        &app,
        &format!("/rest/ping?u=admin&t={t}&s={salt}&v=1.16.1&c=test&f=json"),
    )
    .await;
    // Subsonic returns HTTP 200 with an error payload (code 40).
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"status\":\"failed\""), "body was: {body}");
    assert!(body.contains("\"code\":40"));
}

#[tokio::test]
async fn ping_unknown_user_fails() {
    let (app, _g) = test_app().await;
    let salt = "abcd";
    let t = token("whatever", salt);
    let (_status, body) = get(
        &app,
        &format!("/rest/ping?u=ghost&t={t}&s={salt}&v=1.16.1&c=test&f=json"),
    )
    .await;
    assert!(body.contains("\"status\":\"failed\""));
}

// ---- routing precedence ----

#[tokio::test]
async fn unknown_rest_endpoint_returns_subsonic_error_not_spa() {
    let (app, _g) = test_app().await;
    let salt = "abcd";
    let t = token("adminpw", salt);
    let (status, body) = get(
        &app,
        &format!("/rest/getNonexistent?u=admin&t={t}&s={salt}&v=1.16.1&c=test&f=json"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"status\":\"failed\""), "body was: {body}");
    assert!(!body.contains("<!DOCTYPE html>"));
}

#[tokio::test]
async fn unknown_api_route_is_404_not_spa() {
    let (app, _g) = test_app().await;
    let (status, _body) = get(&app, "/api/nonexistent").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn non_rest_non_api_route_serves_spa() {
    let (app, _g) = test_app().await;
    let (status, body) = get(&app, "/some/client/route").await;
    assert_eq!(status, StatusCode::OK);
    // The embedded SPA shell.
    assert!(body.contains("<html") || body.contains("<!doctype html"), "body: {body}");
}

// ---- portal /api sessions + admin isolation ----

#[tokio::test]
async fn portal_login_bad_password_401() {
    let (app, _g) = test_app().await;
    let (status, _b, cookie) =
        post_json(&app, "/api/login", r#"{"username":"admin","password":"nope"}"#).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(cookie.is_none());
}

#[tokio::test]
async fn portal_login_sets_session_and_me_works() {
    let (app, _g) = test_app().await;
    let (status, body, cookie) =
        post_json(&app, "/api/login", r#"{"username":"admin","password":"adminpw"}"#).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"isAdmin\":true"));
    let cookie = cookie.expect("session cookie set");
    // HttpOnly + SameSite=Strict on the session cookie.
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));

    // Use the cookie to reach /api/me.
    let cookie_val = cookie.split(';').next().unwrap().to_string();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header(header::COOKIE, &cookie_val)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&bytes);
    assert!(body.contains("\"username\":\"admin\""));
    assert!(body.contains("\"tidalLinked\":false"));
}

#[tokio::test]
async fn portal_me_without_cookie_401() {
    let (app, _g) = test_app().await;
    let (status, _b) = get(&app, "/api/me").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn portal_non_admin_forbidden_on_admin_route() {
    let (app, _g) = test_app().await;
    // bob is a normal user.
    let (_s, _b, cookie) =
        post_json(&app, "/api/login", r#"{"username":"bob","password":"bobpw"}"#).await;
    let cookie_val = cookie.unwrap().split(';').next().unwrap().to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/users")
                .header(header::COOKIE, &cookie_val)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn portal_admin_can_list_users() {
    let (app, _g) = test_app().await;
    let (_s, _b, cookie) =
        post_json(&app, "/api/login", r#"{"username":"admin","password":"adminpw"}"#).await;
    let cookie_val = cookie.unwrap().split(';').next().unwrap().to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/users")
                .header(header::COOKIE, &cookie_val)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&bytes);
    assert!(body.contains("admin") && body.contains("bob"), "body: {body}");
}

/// Minimal temp-dir helper (no external crate): a unique dir under the OS temp,
/// removed on drop.
mod tempdir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    pub struct TempGuard(PathBuf);

    impl TempGuard {
        pub fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let pid = std::process::id();
            let dir = std::env::temp_dir().join(format!("tsub-it-{pid}-{n}"));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempGuard(dir)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
