use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

pub type SharedDb = Arc<Mutex<Connection>>;

#[derive(Debug, Clone)]
pub struct DbConfig {
    pub tidal_client_id: String,
    pub tidal_client_secret: String,
    pub tidal_access_token: String,
    pub tidal_refresh_token: String,
    pub tidal_user_id: Option<u64>,
    pub tidal_country_code: String,
    pub tidal_max_quality: String,
    pub server_host: String,
    pub server_port: u16,
    pub subsonic_username: String,
    pub subsonic_password: String,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            tidal_client_id: String::new(),
            tidal_client_secret: String::new(),
            tidal_access_token: String::new(),
            tidal_refresh_token: String::new(),
            tidal_user_id: None,
            tidal_country_code: "US".to_string(),
            tidal_max_quality: "HIGH".to_string(),
            server_host: "0.0.0.0".to_string(),
            server_port: 4533,
            subsonic_username: "tidal".to_string(),
            subsonic_password: "tidal".to_string(),
        }
    }
}

/// Open (or create) the SQLite database and run migrations.
pub fn open_db(db_path: &Path) -> Result<SharedDb> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(db_path)?;

    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    // Migrations
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS config (
            key   TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tidal_tokens (
            id            INTEGER PRIMARY KEY CHECK (id = 1),
            access_token  TEXT NOT NULL,
            refresh_token TEXT NOT NULL,
            user_id       INTEGER,
            country_code  TEXT NOT NULL DEFAULT 'US',
            updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS subsonic_users (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            username      TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at    TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;

    Ok(Arc::new(Mutex::new(conn)))
}

// ---- Config helpers ----

fn get_config_str(conn: &Connection, key: &str, default: &str) -> String {
    conn.query_row(
        "SELECT value FROM config WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .unwrap_or_else(|_| default.to_string())
}

fn set_config_str(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO config (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn get_config_u64(conn: &Connection, key: &str) -> Option<u64> {
    conn.query_row(
        "SELECT value FROM config WHERE key = ?1",
        params![key],
        |row| {
            let s: String = row.get(0)?;
            Ok(s.parse::<u64>().ok())
        },
    )
    .ok()
    .flatten()
}

pub async fn load_config(db: &SharedDb) -> DbConfig {
    let conn = db.lock().await;
    let mut cfg = DbConfig::default();

    cfg.tidal_client_id = get_config_str(&conn, "tidal_client_id", "");
    cfg.tidal_client_secret = get_config_str(&conn, "tidal_client_secret", "");
    cfg.tidal_max_quality = get_config_str(&conn, "tidal_max_quality", "HIGH");
    cfg.server_host = get_config_str(&conn, "server_host", "0.0.0.0");
    cfg.server_port = get_config_str(&conn, "server_port", "4533").parse().unwrap_or(4533);
    cfg.subsonic_username = get_config_str(&conn, "subsonic_username", "tidal");
    cfg.subsonic_password = get_config_str(&conn, "subsonic_password", "tidal");

    // Load tokens
    let token_row = conn.query_row(
        "SELECT access_token, refresh_token, user_id, country_code FROM tidal_tokens WHERE id = 1",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    );
    if let Ok((at, rt, uid, cc)) = token_row {
        cfg.tidal_access_token = at;
        cfg.tidal_refresh_token = rt;
        cfg.tidal_user_id = uid.map(|u| u as u64);
        cfg.tidal_country_code = cc;
    }

    cfg
}

pub async fn save_config(db: &SharedDb, cfg: &DbConfig) -> Result<()> {
    let conn = db.lock().await;

    set_config_str(&conn, "tidal_client_id", &cfg.tidal_client_id)?;
    set_config_str(&conn, "tidal_client_secret", &cfg.tidal_client_secret)?;
    set_config_str(&conn, "tidal_max_quality", &cfg.tidal_max_quality)?;
    set_config_str(&conn, "server_host", &cfg.server_host)?;
    set_config_str(&conn, "server_port", &cfg.server_port.to_string())?;
    set_config_str(&conn, "subsonic_username", &cfg.subsonic_username)?;
    set_config_str(&conn, "subsonic_password", &cfg.subsonic_password)?;

    Ok(())
}

pub async fn save_tokens(
    db: &SharedDb,
    access_token: &str,
    refresh_token: &str,
    user_id: Option<u64>,
    country_code: &str,
) -> Result<()> {
    let conn = db.lock().await;
    conn.execute(
        "INSERT INTO tidal_tokens (id, access_token, refresh_token, user_id, country_code, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, datetime('now'))
         ON CONFLICT(id) DO UPDATE SET
           access_token = excluded.access_token,
           refresh_token = excluded.refresh_token,
           user_id = excluded.user_id,
           country_code = excluded.country_code,
           updated_at = excluded.updated_at",
        params![access_token, refresh_token, user_id.map(|u| u as i64), country_code],
    )?;
    Ok(())
}

pub async fn is_authenticated(db: &SharedDb) -> bool {
    let conn = db.lock().await;
    conn.query_row(
        "SELECT COUNT(*) FROM tidal_tokens WHERE id = 1 AND access_token != ''",
        [],
        |row| row.get::<_, i32>(0),
    )
    .map(|c| c > 0)
    .unwrap_or(false)
}
