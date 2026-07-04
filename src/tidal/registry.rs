//! Per-user TIDAL client registry.
//!
//! Each Subsonic user links their own TIDAL account, so requests must route to
//! that user's `TidalClient` (own tokens, own favorites). The registry builds a
//! client lazily on first use from the user's `tidal_accounts` row and caches it
//! keyed by Subsonic user id.

use crate::crypto::Cipher;
use crate::db::{self, SharedDb};
use crate::tidal::{SharedTidalClient, TidalClient};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct ClientRegistry {
    db: SharedDb,
    cipher: Cipher,
    /// App-level TIDAL OAuth credentials (shared across all users).
    client_id: String,
    client_secret: String,
    clients: Arc<Mutex<HashMap<i64, SharedTidalClient>>>,
}

impl ClientRegistry {
    pub fn new(db: SharedDb, cipher: Cipher, client_id: String, client_secret: String) -> Self {
        ClientRegistry {
            db,
            cipher,
            client_id,
            client_secret,
            clients: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get (or lazily build) the TIDAL client for a Subsonic user. Returns None
    /// if the user hasn't linked a TIDAL account yet.
    pub async fn get(&self, subsonic_user_id: i64) -> Option<SharedTidalClient> {
        {
            let map = self.clients.lock().await;
            if let Some(c) = map.get(&subsonic_user_id) {
                return Some(c.clone());
            }
        }

        // Miss: load the user's linked account and build a client.
        let account = db::load_tidal_account(&self.db, &self.cipher, subsonic_user_id).await?;
        if account.access_token.is_empty() && account.refresh_token.is_empty() {
            return None;
        }
        let client = Arc::new(TidalClient::for_user(
            subsonic_user_id,
            &account,
            self.client_id.clone(),
            self.client_secret.clone(),
            self.db.clone(),
            self.cipher.clone(),
        ));

        let mut map = self.clients.lock().await;
        // Another request may have built it while we were loading.
        let entry = map
            .entry(subsonic_user_id)
            .or_insert_with(|| client.clone());
        Some(entry.clone())
    }

    /// Drop a user's cached client (e.g. after they re-link or are deleted), so
    /// the next request rebuilds from fresh DB state.
    pub async fn invalidate(&self, subsonic_user_id: i64) {
        self.clients.lock().await.remove(&subsonic_user_id);
    }
}
