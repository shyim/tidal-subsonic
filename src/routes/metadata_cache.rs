//! Per-user in-memory metadata cache for browse/search/list responses.
//!
//! Endpoints like getArtist / getAlbum / getAlbumList(2) / search2 / search3 /
//! getArtists / getPlaylists re-hit TIDAL on every request. Chatty clients fire
//! the same request many times in a burst (e.g. getArtist 40x in one second),
//! and users re-open the same views constantly. Without a cache each of those
//! becomes an upstream round-trip.
//!
//! This caches the *mapped Subsonic domain structs* the handler builds (e.g.
//! `Vec<SubsonicAlbum>`, `ArtistWithAlbums`, `SearchResult3`) — cloneable and
//! cheap to re-serialize — keyed by `(subsonic_user_id, endpoint, params)`.
//!
//! Correctness guards mirroring `media_cache`:
//! - **Per-user keys**: every key is prefixed with the Subsonic user id, so no
//!   user can ever observe another user's cached data.
//! - **TTL**: entries expire after a per-call time-to-live (favorites/playlists
//!   are volatile → short; artist/album/search are stable → long).
//! - **Single-flight**: concurrent misses for the same key wait on one builder
//!   rather than all stampeding TIDAL at once.
//! - **Size cap**: a bounded entry count with cheap eviction of the oldest
//!   inserted key, so the map can't grow without bound.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Max distinct cache entries before the oldest inserted one is evicted.
const MAX_ENTRIES: usize = 4096;

/// TTL for volatile, user-mutable views (favorites-backed: album lists, starred,
/// artists index, playlists). Kept short so a star/unstar or a change made in
/// the TIDAL app shows up quickly even without explicit invalidation.
pub(crate) const TTL_FAVORITES: Duration = Duration::from_secs(60);

/// TTL for stable catalog metadata (artist detail, album detail, search
/// results) — these rarely change, so cache them longer.
pub(crate) const TTL_CATALOG: Duration = Duration::from_secs(300);

/// A cached value: an expiry instant plus the type-erased cloned domain struct.
struct Entry {
    expires_at: Instant,
    /// The cached value. `Arc<dyn Any>` so one cache holds heterogeneous types
    /// (album lists, artist detail, search results, …); each `get_or_build`
    /// downcasts back to its concrete `T`.
    value: Arc<dyn Any + Send + Sync>,
    /// Monotonic insertion order, for oldest-first eviction when over capacity.
    seq: u64,
}

#[derive(Clone)]
pub(crate) struct MetadataCache {
    inner: Arc<Mutex<Inner>>,
    /// Per-key build locks, so only one request builds a given key at a time.
    locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

struct Inner {
    map: HashMap<String, Entry>,
    next_seq: u64,
}

impl MetadataCache {
    pub(crate) fn new() -> Self {
        MetadataCache {
            inner: Arc::new(Mutex::new(Inner {
                map: HashMap::new(),
                next_seq: 0,
            })),
            locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Build the canonical cache key. `user` namespaces every entry so users
    /// never share cached data; `endpoint` and `params` distinguish requests.
    pub(crate) fn key(user: i64, endpoint: &str, params: &str) -> String {
        format!("u{user}\u{1f}{endpoint}\u{1f}{params}")
    }

    /// Return a cached `T` for `key`, or build it via `build` on a miss / expiry.
    /// `build` runs at most once per key across concurrent callers
    /// (single-flight) and its result is cached for `ttl`.
    ///
    /// `T` is the cloneable mapped domain struct the handler would otherwise
    /// build every time. A hit clones the cached value (cheap relative to a
    /// TIDAL round-trip) and returns it without calling `build`.
    pub(crate) async fn get_or_build<T, F, Fut>(
        &self,
        key: &str,
        ttl: Duration,
        build: F,
    ) -> Result<T, String>
    where
        T: Clone + Send + Sync + 'static,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, String>>,
    {
        if let Some(v) = self.get_fresh::<T>(key).await {
            return Ok(v);
        }

        // Acquire (or create) the per-key build lock for single-flight.
        let lock = {
            let mut map = self.locks.lock().await;
            map.entry(key.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;

        // Another builder may have finished while we waited for the lock.
        if let Some(v) = self.get_fresh::<T>(key).await {
            self.release_lock(key).await;
            return Ok(v);
        }

        let built = build().await?;
        self.insert(key, built.clone(), ttl).await;
        self.release_lock(key).await;
        Ok(built)
    }

    /// Look up a live (non-expired) entry and clone its value back to `T`.
    async fn get_fresh<T: Clone + Send + Sync + 'static>(&self, key: &str) -> Option<T> {
        let inner = self.inner.lock().await;
        let entry = inner.map.get(key)?;
        if entry.expires_at <= Instant::now() {
            return None;
        }
        entry.value.downcast_ref::<T>().cloned()
    }

    async fn insert<T: Send + Sync + 'static>(&self, key: &str, value: T, ttl: Duration) {
        let mut inner = self.inner.lock().await;
        let seq = inner.next_seq;
        inner.next_seq += 1;
        inner.map.insert(
            key.to_string(),
            Entry {
                expires_at: Instant::now() + ttl,
                value: Arc::new(value),
                seq,
            },
        );
        Self::evict_if_over_cap(&mut inner);
    }

    /// Evict the oldest-inserted entry while over the capacity cap.
    fn evict_if_over_cap(inner: &mut Inner) {
        while inner.map.len() > MAX_ENTRIES {
            if let Some(oldest) = inner
                .map
                .iter()
                .min_by_key(|(_, e)| e.seq)
                .map(|(k, _)| k.clone())
            {
                inner.map.remove(&oldest);
            } else {
                break;
            }
        }
    }

    /// Drop the per-key build lock from the map if no one else holds it, so the
    /// lock map doesn't grow unbounded.
    async fn release_lock(&self, key: &str) {
        let mut map = self.locks.lock().await;
        if let Some(l) = map.get(key) {
            if Arc::strong_count(l) == 1 {
                map.remove(key);
            }
        }
    }

    /// Invalidate every cached entry belonging to `user`. Called after a
    /// mutation (star/unstar) so the user's favorites / starred / albumList /
    /// artists views rebuild from TIDAL on their next request. Other users'
    /// entries are untouched.
    pub(crate) async fn invalidate_user(&self, user: i64) {
        let prefix = format!("u{user}\u{1f}");
        let mut inner = self.inner.lock().await;
        inner.map.retain(|k, _| !k.starts_with(&prefix));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn miss_builds_once_then_hit() {
        let cache = MetadataCache::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let ttl = Duration::from_secs(300);
        let key = MetadataCache::key(1, "getArtist", "id=42");

        let c = calls.clone();
        let v1: Vec<u32> = cache
            .get_or_build(&key, ttl, move || async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(vec![1, 2, 3])
            })
            .await
            .unwrap();
        assert_eq!(v1, vec![1, 2, 3]);

        // Second call is a hit: the build closure must NOT run again.
        let c = calls.clone();
        let v2: Vec<u32> = cache
            .get_or_build(&key, ttl, move || async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(vec![9, 9, 9])
            })
            .await
            .unwrap();
        assert_eq!(v2, vec![1, 2, 3], "hit should return cached value");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "build ran more than once");
    }

    #[tokio::test]
    async fn per_user_isolation() {
        let cache = MetadataCache::new();
        let ttl = Duration::from_secs(300);
        let k1 = MetadataCache::key(1, "getAlbum", "id=7");
        let k2 = MetadataCache::key(2, "getAlbum", "id=7");

        let v1: String = cache
            .get_or_build(&k1, ttl, || async { Ok("user-one".to_string()) })
            .await
            .unwrap();
        // Same endpoint+params, different user → separate key → separate build.
        let v2: String = cache
            .get_or_build(&k2, ttl, || async { Ok("user-two".to_string()) })
            .await
            .unwrap();
        assert_eq!(v1, "user-one");
        assert_eq!(v2, "user-two");

        // Hits stay isolated.
        let v1b: String = cache
            .get_or_build(&k1, ttl, || async { Ok("SHOULD-NOT-RUN".to_string()) })
            .await
            .unwrap();
        assert_eq!(v1b, "user-one");
    }

    #[tokio::test]
    async fn ttl_expiry_rebuilds() {
        let cache = MetadataCache::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let key = MetadataCache::key(1, "getPlaylists", "");

        let c = calls.clone();
        let _: u32 = cache
            .get_or_build(&key, Duration::from_millis(20), move || async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(1)
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(40)).await;

        let c = calls.clone();
        let _: u32 = cache
            .get_or_build(&key, Duration::from_millis(20), move || async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(2)
            })
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2, "expired entry should rebuild");
    }

    #[tokio::test]
    async fn invalidate_user_clears_only_that_user() {
        let cache = MetadataCache::new();
        let ttl = Duration::from_secs(300);
        let k1 = MetadataCache::key(1, "getArtists", "");
        let k2 = MetadataCache::key(2, "getArtists", "");
        let _: u32 = cache.get_or_build(&k1, ttl, || async { Ok(1) }).await.unwrap();
        let _: u32 = cache.get_or_build(&k2, ttl, || async { Ok(2) }).await.unwrap();

        cache.invalidate_user(1).await;

        // User 1's entry is gone → build runs again.
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let _: u32 = cache
            .get_or_build(&k1, ttl, move || async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(11)
            })
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "user 1 should have rebuilt");

        // User 2's entry survived → build does NOT run.
        let c = calls.clone();
        let v2: u32 = cache
            .get_or_build(&k2, ttl, move || async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(99)
            })
            .await
            .unwrap();
        assert_eq!(v2, 2, "user 2's entry should have survived");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "user 2 should not rebuild");
    }
}
