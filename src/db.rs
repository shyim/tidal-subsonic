use crate::crypto::Cipher;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

pub type SharedDb = Arc<Mutex<Connection>>;

/// A registered Subsonic user. Each links (optionally) their own TIDAL account.
#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    /// The Subsonic password. Subsonic token auth is `md5(password + salt)` with
    /// a client-chosen salt, so the server must keep the password recoverable —
    /// it can't be a one-way hash. Stored encrypted at rest.
    pub password: String,
    pub is_admin: bool,
}

/// A user's linked TIDAL account (tokens are encrypted at rest).
#[derive(Debug, Clone)]
pub struct TidalAccount {
    pub access_token: String,
    pub refresh_token: String,
    pub tidal_user_id: Option<u64>,
    pub country_code: String,
}

/// Server-level config (the TIDAL app OAuth credentials + server settings).
/// Per-user Subsonic credentials and TIDAL tokens live in their own tables.
#[derive(Debug, Clone)]
pub struct DbConfig {
    pub tidal_client_id: String,
    pub tidal_client_secret: String,
    pub tidal_max_quality: String,
    pub server_host: String,
    pub server_port: u16,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            tidal_client_id: String::new(),
            tidal_client_secret: String::new(),
            tidal_max_quality: "HIGH".to_string(),
            server_host: "0.0.0.0".to_string(),
            server_port: 4533,
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

        -- Legacy single-user tables (kept for one-time migration into `users`).
        CREATE TABLE IF NOT EXISTS tidal_tokens (
            id            INTEGER PRIMARY KEY CHECK (id = 1),
            access_token  TEXT NOT NULL,
            refresh_token TEXT NOT NULL,
            user_id       INTEGER,
            country_code  TEXT NOT NULL DEFAULT 'US',
            updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
        );

        -- Multi-user: one row per Subsonic user.
        CREATE TABLE IF NOT EXISTS users (
            id                 INTEGER PRIMARY KEY AUTOINCREMENT,
            username           TEXT UNIQUE NOT NULL,
            password_encrypted TEXT NOT NULL,
            is_admin           INTEGER NOT NULL DEFAULT 0,
            created_at         TEXT NOT NULL DEFAULT (datetime('now'))
        );

        -- One TIDAL account per user; tokens encrypted at rest.
        CREATE TABLE IF NOT EXISTS tidal_accounts (
            user_id             INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
            access_token        TEXT NOT NULL,
            refresh_token       TEXT NOT NULL,
            tidal_user_id       INTEGER,
            country_code        TEXT NOT NULL DEFAULT 'US',
            updated_at          TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;

    Ok(Arc::new(Mutex::new(conn)))
}

/// Read the encryption master key: from `TIDAL_SUBSONIC_KEY` (base64) if set,
/// else a persisted generated key in `config`, generating one on first run.
pub async fn master_cipher(db: &SharedDb) -> Cipher {
    if let Ok(env_key) = std::env::var("TIDAL_SUBSONIC_KEY") {
        if let Some(key) = Cipher::key_from_base64(&env_key) {
            return Cipher::new(key);
        }
        tracing::warn!("TIDAL_SUBSONIC_KEY is set but not a valid base64 32-byte key; ignoring");
    }
    let conn = db.lock().await;
    let existing: Option<String> = conn
        .query_row(
            "SELECT value FROM config WHERE key = 'master_key'",
            [],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    if let Some(k) = existing.and_then(|s| Cipher::key_from_base64(&s)) {
        return Cipher::new(k);
    }
    let key = Cipher::generate_key();
    let _ = conn.execute(
        "INSERT INTO config (key, value) VALUES ('master_key', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![Cipher::key_to_base64(&key)],
    );
    tracing::info!("Generated a new encryption master key (stored in DB). Set TIDAL_SUBSONIC_KEY to manage it yourself.");
    Cipher::new(key)
}

/// One-time migration: fold the legacy single-user config credential and
/// `tidal_tokens(id=1)` into a `users` row (admin) + `tidal_accounts` row.
/// No-op once any user exists.
pub async fn migrate_single_user(db: &SharedDb, cipher: &Cipher) -> Result<()> {
    let conn = db.lock().await;

    let user_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
    if user_count > 0 {
        return Ok(());
    }

    // Legacy Subsonic credential lived in config (defaults tidal/tidal).
    let username = config_str(&conn, "subsonic_username", "tidal");
    let password = config_str(&conn, "subsonic_password", "tidal");

    conn.execute(
        "INSERT INTO users (username, password_encrypted, is_admin)
         VALUES (?1, ?2, 1)",
        params![username, cipher.encrypt(&password)],
    )?;
    let user_id = conn.last_insert_rowid();
    tracing::info!("Migrated single user '{}' to multi-user schema (admin, id {})", username, user_id);

    // Migrate the legacy TIDAL tokens, if any.
    let tokens = conn
        .query_row(
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
        )
        .optional()?;
    if let Some((at, rt, tuid, cc)) = tokens {
        if !at.is_empty() {
            conn.execute(
                "INSERT INTO tidal_accounts (user_id, access_token, refresh_token, tidal_user_id, country_code)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![user_id, cipher.encrypt(&at), cipher.encrypt(&rt), tuid, cc],
            )?;
            tracing::info!("Migrated TIDAL account for user {}", user_id);
        }
    }
    Ok(())
}

fn config_str(conn: &Connection, key: &str, default: &str) -> String {
    conn.query_row(
        "SELECT value FROM config WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .unwrap_or_else(|_| default.to_string())
}

// ---- Users ----

/// Look up a user by username, decrypting their stored password.
pub async fn find_user(db: &SharedDb, cipher: &Cipher, username: &str) -> Option<User> {
    let conn = db.lock().await;
    conn.query_row(
        "SELECT id, username, password_encrypted, is_admin FROM users WHERE username = ?1",
        params![username],
        |row| {
            Ok(User {
                id: row.get(0)?,
                username: row.get(1)?,
                password: cipher.decrypt(&row.get::<_, String>(2)?),
                is_admin: row.get::<_, i64>(3)? != 0,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

/// Create a user. Returns the new id, or an error if the username is taken.
pub async fn create_user(
    db: &SharedDb,
    cipher: &Cipher,
    username: &str,
    password: &str,
    is_admin: bool,
) -> Result<i64> {
    let conn = db.lock().await;
    conn.execute(
        "INSERT INTO users (username, password_encrypted, is_admin) VALUES (?1, ?2, ?3)",
        params![username, cipher.encrypt(password), is_admin as i64],
    )?;
    Ok(conn.last_insert_rowid())
}

pub async fn user_count(db: &SharedDb) -> i64 {
    let conn = db.lock().await;
    conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
        .unwrap_or(0)
}

/// All users, ordered by id (for admin listing).
pub async fn list_users(db: &SharedDb, cipher: &Cipher) -> Vec<User> {
    let conn = db.lock().await;
    let mut stmt = match conn
        .prepare("SELECT id, username, password_encrypted, is_admin FROM users ORDER BY id")
    {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map([], |row| {
        Ok(User {
            id: row.get(0)?,
            username: row.get(1)?,
            password: cipher.decrypt(&row.get::<_, String>(2)?),
            is_admin: row.get::<_, i64>(3)? != 0,
        })
    });
    match rows {
        Ok(iter) => iter.flatten().collect(),
        Err(_) => Vec::new(),
    }
}

/// Delete a user by username (cascades to their tidal_accounts row). Returns the
/// deleted user's id, if any.
pub async fn delete_user(db: &SharedDb, username: &str) -> Option<i64> {
    let conn = db.lock().await;
    let id: Option<i64> = conn
        .query_row(
            "SELECT id FROM users WHERE username = ?1",
            params![username],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    if let Some(id) = id {
        let _ = conn.execute("DELETE FROM users WHERE id = ?1", params![id]);
    }
    id
}

/// Update a user's password and/or admin flag. Returns their id, or None if the
/// user doesn't exist.
pub async fn update_user(
    db: &SharedDb,
    cipher: &Cipher,
    username: &str,
    new_password: Option<&str>,
    new_admin: Option<bool>,
) -> Option<i64> {
    let conn = db.lock().await;
    let id: i64 = conn
        .query_row(
            "SELECT id FROM users WHERE username = ?1",
            params![username],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()?;
    if let Some(pw) = new_password {
        let _ = conn.execute(
            "UPDATE users SET password_encrypted = ?1 WHERE id = ?2",
            params![cipher.encrypt(pw), id],
        );
    }
    if let Some(admin) = new_admin {
        let _ = conn.execute(
            "UPDATE users SET is_admin = ?1 WHERE id = ?2",
            params![admin as i64, id],
        );
    }
    Some(id)
}

// ---- Per-user TIDAL accounts ----

/// Load a user's linked TIDAL account, decrypting the tokens. None if unlinked.
pub async fn load_tidal_account(
    db: &SharedDb,
    cipher: &Cipher,
    user_id: i64,
) -> Option<TidalAccount> {
    let conn = db.lock().await;
    conn.query_row(
        "SELECT access_token, refresh_token, tidal_user_id, country_code
         FROM tidal_accounts WHERE user_id = ?1",
        params![user_id],
        |row| {
            Ok(TidalAccount {
                access_token: cipher.decrypt(&row.get::<_, String>(0)?),
                refresh_token: cipher.decrypt(&row.get::<_, String>(1)?),
                tidal_user_id: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                country_code: row.get(3)?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

/// Insert or update a user's TIDAL tokens (encrypted at rest).
pub async fn save_tidal_account(
    db: &SharedDb,
    cipher: &Cipher,
    user_id: i64,
    account: &TidalAccount,
) -> Result<()> {
    let conn = db.lock().await;
    conn.execute(
        "INSERT INTO tidal_accounts (user_id, access_token, refresh_token, tidal_user_id, country_code, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
         ON CONFLICT(user_id) DO UPDATE SET
           access_token = excluded.access_token,
           refresh_token = excluded.refresh_token,
           tidal_user_id = excluded.tidal_user_id,
           country_code = excluded.country_code,
           updated_at = excluded.updated_at",
        params![
            user_id,
            cipher.encrypt(&account.access_token),
            cipher.encrypt(&account.refresh_token),
            account.tidal_user_id.map(|v| v as i64),
            account.country_code,
        ],
    )?;
    Ok(())
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

/// Load the server/TIDAL-app config (not per-user data). Per-user credentials
/// and TIDAL tokens live in the `users` / `tidal_accounts` tables.
pub async fn load_config(db: &SharedDb) -> DbConfig {
    let conn = db.lock().await;
    let mut cfg = DbConfig::default();
    cfg.tidal_client_id = get_config_str(&conn, "tidal_client_id", "");
    cfg.tidal_client_secret = get_config_str(&conn, "tidal_client_secret", "");
    cfg.tidal_max_quality = get_config_str(&conn, "tidal_max_quality", "HIGH");
    cfg.server_host = get_config_str(&conn, "server_host", "0.0.0.0");
    cfg.server_port = get_config_str(&conn, "server_port", "4533").parse().unwrap_or(4533);
    cfg
}
