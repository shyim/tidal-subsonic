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

/// A saved play queue (Subsonic track ids, current, and position).
#[derive(Debug, Clone)]
pub struct PlayQueueRow {
    pub track_ids: Vec<String>,
    pub current: Option<String>,
    pub position_ms: u64,
    pub changed_by: Option<String>,
    pub changed_at: String,
}

/// A per-track bookmark.
#[derive(Debug, Clone)]
pub struct BookmarkRow {
    pub track_id: String,
    pub position_ms: u64,
    pub comment: Option<String>,
    pub created_at: String,
    pub changed_at: String,
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
        );

        -- Per-user saved play queue (resume across devices). One row per user.
        CREATE TABLE IF NOT EXISTS play_queue (
            user_id     INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
            track_ids   TEXT NOT NULL,          -- comma-separated Subsonic track ids
            current     TEXT,                   -- the current Subsonic track id
            position_ms INTEGER NOT NULL DEFAULT 0,
            changed_by  TEXT,                   -- client name that last saved
            changed_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        -- Per-user per-track bookmarks (resume position within a track).
        CREATE TABLE IF NOT EXISTS bookmarks (
            user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            track_id    TEXT NOT NULL,          -- Subsonic track id
            position_ms INTEGER NOT NULL,
            comment     TEXT,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            changed_at  TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (user_id, track_id)
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
/// On an empty `users` table, either migrate a legacy single-user install or
/// bootstrap the first admin. A legacy install is detected by an explicit
/// `subsonic_username` config row or existing `tidal_tokens`; those keep their
/// original password. A truly fresh install must NOT get a known default
/// password: the admin password comes from `TIDAL_SUBSONIC_ADMIN_PASSWORD`, or a
/// random one is generated and printed to the log once.
pub async fn migrate_single_user(db: &SharedDb, cipher: &Cipher) -> Result<()> {
    let conn = db.lock().await;

    let user_count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
    if user_count > 0 {
        return Ok(());
    }

    // Did a legacy single-user setup exist? (explicit config username, or tokens)
    let legacy_username: Option<String> = conn
        .query_row(
            "SELECT value FROM config WHERE key = 'subsonic_username'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let legacy_tokens = conn
        .query_row(
            "SELECT access_token, refresh_token, user_id, country_code FROM tidal_tokens WHERE id = 1 AND access_token != ''",
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
    let is_legacy = legacy_username.is_some() || legacy_tokens.is_some();

    let (username, password) = if is_legacy {
        // Preserve the legacy credential.
        let u = legacy_username.unwrap_or_else(|| "tidal".to_string());
        let p = config_str(&conn, "subsonic_password", "tidal");
        (u, p)
    } else {
        // Fresh install: never use a known default password.
        let u = "admin".to_string();
        let p = match std::env::var("TIDAL_SUBSONIC_ADMIN_PASSWORD") {
            Ok(v) if !v.is_empty() => v,
            _ => {
                let generated = random_password();
                tracing::warn!(
                    "First run: created admin user 'admin' with generated password: {}  \
                     (set TIDAL_SUBSONIC_ADMIN_PASSWORD to choose your own; change it after login)",
                    generated
                );
                generated
            }
        };
        (u, p)
    };

    conn.execute(
        "INSERT INTO users (username, password_encrypted, is_admin) VALUES (?1, ?2, 1)",
        params![username, cipher.encrypt(&password)],
    )?;
    let user_id = conn.last_insert_rowid();
    if is_legacy {
        tracing::info!(
            "Migrated single user '{}' to multi-user schema (admin, id {})",
            username,
            user_id
        );
    } else {
        tracing::info!("Bootstrapped first admin user 'admin' (id {})", user_id);
    }

    // Migrate legacy TIDAL tokens, if any.
    if let Some((at, rt, tuid, cc)) = legacy_tokens {
        conn.execute(
            "INSERT INTO tidal_accounts (user_id, access_token, refresh_token, tidal_user_id, country_code)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![user_id, cipher.encrypt(&at), cipher.encrypt(&rt), tuid, cc],
        )?;
        tracing::info!("Migrated TIDAL account for user {}", user_id);
    }
    Ok(())
}

/// Generate a random, human-typable admin password (no ambiguous characters).
fn random_password() -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    (0..20)
        .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
        .collect()
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

// ---- Play queue & bookmarks (per user) ----

pub async fn save_play_queue(
    db: &SharedDb,
    user_id: i64,
    track_ids: &[String],
    current: Option<&str>,
    position_ms: u64,
    changed_by: Option<&str>,
) -> Result<()> {
    let conn = db.lock().await;
    conn.execute(
        "INSERT INTO play_queue (user_id, track_ids, current, position_ms, changed_by, changed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
         ON CONFLICT(user_id) DO UPDATE SET
           track_ids = excluded.track_ids, current = excluded.current,
           position_ms = excluded.position_ms, changed_by = excluded.changed_by,
           changed_at = excluded.changed_at",
        params![user_id, track_ids.join(","), current, position_ms as i64, changed_by],
    )?;
    Ok(())
}

pub async fn load_play_queue(db: &SharedDb, user_id: i64) -> Option<PlayQueueRow> {
    let conn = db.lock().await;
    conn.query_row(
        "SELECT track_ids, current, position_ms, changed_by, changed_at FROM play_queue WHERE user_id = ?1",
        params![user_id],
        |row| {
            let ids: String = row.get(0)?;
            Ok(PlayQueueRow {
                track_ids: ids.split(',').filter(|s| !s.is_empty()).map(String::from).collect(),
                current: row.get(1)?,
                position_ms: row.get::<_, i64>(2)? as u64,
                changed_by: row.get(3)?,
                changed_at: row.get(4)?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

pub async fn save_bookmark(
    db: &SharedDb,
    user_id: i64,
    track_id: &str,
    position_ms: u64,
    comment: Option<&str>,
) -> Result<()> {
    let conn = db.lock().await;
    conn.execute(
        "INSERT INTO bookmarks (user_id, track_id, position_ms, comment, changed_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'))
         ON CONFLICT(user_id, track_id) DO UPDATE SET
           position_ms = excluded.position_ms, comment = excluded.comment,
           changed_at = excluded.changed_at",
        params![user_id, track_id, position_ms as i64, comment],
    )?;
    Ok(())
}

pub async fn list_bookmarks(db: &SharedDb, user_id: i64) -> Vec<BookmarkRow> {
    let conn = db.lock().await;
    let mut stmt = match conn.prepare(
        "SELECT track_id, position_ms, comment, created_at, changed_at
         FROM bookmarks WHERE user_id = ?1 ORDER BY changed_at DESC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(params![user_id], |row| {
        Ok(BookmarkRow {
            track_id: row.get(0)?,
            position_ms: row.get::<_, i64>(1)? as u64,
            comment: row.get(2)?,
            created_at: row.get(3)?,
            changed_at: row.get(4)?,
        })
    });
    match rows {
        Ok(iter) => iter.flatten().collect(),
        Err(_) => Vec::new(),
    }
}

pub async fn delete_bookmark(db: &SharedDb, user_id: i64, track_id: &str) -> Result<()> {
    let conn = db.lock().await;
    conn.execute(
        "DELETE FROM bookmarks WHERE user_id = ?1 AND track_id = ?2",
        params![user_id, track_id],
    )?;
    Ok(())
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
