//! On-disk cache for fully-assembled / transcoded audio tracks.
//!
//! TIDAL delivers audio as ~60 DASH segments that must be concatenated (and, for
//! MP3 clients, transcoded — ~14s of CPU). Without a cache, every replay and
//! every byte-range seek re-fetches the whole track from the CDN and re-runs the
//! transcode. This caches the finished bytes in a content-addressed file per
//! `(track, format, bitrate)`, so a hit is a local file read that also supports
//! HTTP range requests for free.
//!
//! Correctness guards:
//! - **Single-flight**: concurrent requests for the same uncached key wait on one
//!   builder rather than all fetching/transcoding at once.
//! - **Atomic writes**: build into `<key>.part`, then rename — a crash or aborted
//!   build never leaves a truncated file that looks like a valid hit.
//! - **Size cap**: prune least-recently-modified files when the dir exceeds the
//!   configured budget.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Total on-disk budget for cached audio before LRU pruning kicks in.
const CACHE_BUDGET_BYTES: u64 = 3 * 1024 * 1024 * 1024; // 3 GiB

#[derive(Clone)]
pub(crate) struct MediaCache {
    dir: PathBuf,
    /// Per-key build locks, so only one request builds a given file at a time.
    locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl MediaCache {
    /// Open (creating if needed) the media cache under the OS cache dir. Falls
    /// back to a temp subdirectory if the cache dir is unavailable.
    pub(crate) fn open() -> Self {
        let dir = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("tidal-subsonic")
            .join("media");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("media cache dir {}: {}", dir.display(), e);
        }
        MediaCache {
            dir,
            locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Hash the (possibly attacker-influenced) key into a fixed hex filename, so
    /// no character from the key can ever escape the cache directory (path
    /// traversal). Track ids are numeric today, but hashing makes the cache safe
    /// regardless of what callers key on.
    fn file_stem(key: &str) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(key.as_bytes());
        let mut s = String::with_capacity(64);
        for b in digest {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.bin", Self::file_stem(key)))
    }

    /// Return the cached file path for `key`, building it via `build` on a miss.
    /// `build` produces the full track bytes; it runs at most once per key across
    /// concurrent callers (single-flight). On success the file exists on disk.
    pub(crate) async fn get_or_build<F, Fut>(&self, key: &str, build: F) -> Result<PathBuf, String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<u8>, String>>,
    {
        let path = self.path_for(key);
        if path.is_file() {
            touch(&path);
            return Ok(path);
        }

        // Acquire (or create) the per-key build lock.
        let lock = {
            let mut map = self.locks.lock().await;
            map.entry(key.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;

        // Another builder may have finished while we waited for the lock.
        if path.is_file() {
            touch(&path);
            return Ok(path);
        }

        let bytes = build().await?;
        self.write_atomic(key, &bytes)?;
        self.prune_if_over_budget();

        // Best-effort cleanup of the lock map so it doesn't grow unbounded.
        {
            let mut map = self.locks.lock().await;
            if let Some(l) = map.get(key) {
                if Arc::strong_count(l) == 1 {
                    map.remove(key);
                }
            }
        }
        Ok(path)
    }

    /// Write `bytes` to `<hash>.part` then rename to the final path (atomic).
    fn write_atomic(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        let stem = Self::file_stem(key);
        let final_path = self.dir.join(format!("{stem}.bin"));
        let tmp = self.dir.join(format!("{stem}.part"));
        std::fs::write(&tmp, bytes).map_err(|e| format!("cache write: {}", e))?;
        std::fs::rename(&tmp, &final_path).map_err(|e| format!("cache rename: {}", e))?;
        Ok(())
    }

    /// If the cache dir exceeds the budget, delete least-recently-modified files
    /// until back under it. Best-effort — logs but never fails the request.
    fn prune_if_over_budget(&self) {
        let mut entries: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
        let mut total: u64 = 0;
        let Ok(read) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for e in read.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("bin") {
                continue;
            }
            if let Ok(md) = e.metadata() {
                let mtime = md.modified().unwrap_or(std::time::UNIX_EPOCH);
                total += md.len();
                entries.push((p, md.len(), mtime));
            }
        }
        if total <= CACHE_BUDGET_BYTES {
            return;
        }
        // Oldest first.
        entries.sort_by_key(|(_, _, mtime)| *mtime);
        for (path, size, _) in entries {
            if total <= CACHE_BUDGET_BYTES {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(size);
                tracing::debug!("media cache evicted {}", path.display());
            }
        }
    }
}

/// Bump a file's mtime so LRU pruning treats a cache hit as recently used.
fn touch(path: &Path) {
    let now = std::time::SystemTime::now();
    let _ = filetime_set(path, now);
}

/// Set a file's modified time (best-effort, std-only).
fn filetime_set(path: &Path, time: std::time::SystemTime) -> std::io::Result<()> {
    // std has no direct setter; opening for append and writing 0 bytes updates
    // atime/mtime on most platforms. Cheaper: use the File::set_modified API
    // (stable since Rust 1.75).
    let f = std::fs::OpenOptions::new().write(true).open(path)?;
    f.set_modified(time)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_cache(name: &str) -> MediaCache {
        let dir = std::env::temp_dir().join(format!("tsub-cache-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        MediaCache {
            dir,
            locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[tokio::test]
    async fn miss_builds_then_hit_reuses() {
        let cache = temp_cache("miss-hit");
        let calls = Arc::new(AtomicUsize::new(0));

        let c = calls.clone();
        let p1 = cache
            .get_or_build("track.mp3.320", move || async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(b"hello-mp3".to_vec())
            })
            .await
            .unwrap();
        assert!(p1.is_file());
        assert_eq!(std::fs::read(&p1).unwrap(), b"hello-mp3");

        // Second call is a hit: the build closure must NOT run again.
        let c = calls.clone();
        let p2 = cache
            .get_or_build("track.mp3.320", move || async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(b"SHOULD-NOT-RUN".to_vec())
            })
            .await
            .unwrap();
        assert_eq!(p1, p2);
        assert_eq!(std::fs::read(&p2).unwrap(), b"hello-mp3");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "build ran more than once");

        let _ = std::fs::remove_dir_all(&cache.dir);
    }

    #[tokio::test]
    async fn malicious_key_stays_inside_cache_dir() {
        let cache = temp_cache("traversal");
        // A key crafted to escape the dir must still land on a hashed filename
        // directly under the cache dir.
        let evil = "../../../../etc/passwd\0.mp3.320";
        let p = cache
            .get_or_build(evil, || async { Ok(b"x".to_vec()) })
            .await
            .unwrap();
        assert_eq!(p.parent().unwrap(), cache.dir);
        let name = p.file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with(".bin"));
        // 64 hex chars + ".bin"
        assert_eq!(name.len(), 68);
        assert!(name.chars().take(64).all(|c| c.is_ascii_hexdigit()));
        let _ = std::fs::remove_dir_all(&cache.dir);
    }

    #[tokio::test]
    async fn build_error_leaves_no_file() {
        let cache = temp_cache("build-err");
        let res = cache
            .get_or_build("bad.mp3.320", || async { Err("boom".to_string()) })
            .await;
        assert!(res.is_err());
        assert!(!cache.path_for("bad.mp3.320").is_file());
        // No stray .part either.
        let parts: Vec<_> = std::fs::read_dir(&cache.dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("part"))
            .collect();
        assert!(parts.is_empty());
        let _ = std::fs::remove_dir_all(&cache.dir);
    }
}
