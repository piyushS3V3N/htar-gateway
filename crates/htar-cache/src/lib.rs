use bytes::Bytes;
use dashmap::DashMap;
use htar_core::{CompressionType, HtarReader};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct CachedResponse {
    pub payload: Bytes,
    pub content_type: String,
    pub etag: String,
    pub compressed: bool,
    pub cached_at: Instant,
    pub ttl: Option<Duration>,
}

#[derive(Debug, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub bytes_served: u64,
    pub archive_loads: u64,
}

pub struct HtarCacheManager {
    /// In-memory hot cache table
    hot_cache: DashMap<String, CachedResponse>,
    /// Backing HTAR archives loaded for O(1) cold reads
    archive_path: Option<PathBuf>,
    reader: Option<Arc<RwLock<HtarReader<File>>>>,
    pub stats: Arc<RwLock<CacheStats>>,
}

impl HtarCacheManager {
    pub fn new() -> Self {
        Self {
            hot_cache: DashMap::new(),
            archive_path: None,
            reader: None,
            stats: Arc::new(RwLock::new(CacheStats::default())),
        }
    }

    /// Load an HTAR archive as cold backing store
    pub fn load_archive<P: AsRef<Path>>(&mut self, path: P) -> Result<(), htar_core::HtarError> {
        let file_path = path.as_ref().to_path_buf();
        let file = File::open(&file_path)?;
        let reader = HtarReader::open(file)?;

        self.archive_path = Some(file_path);
        self.reader = Some(Arc::new(RwLock::new(reader)));
        Ok(())
    }

    /// Insert entry into hot memory cache
    pub fn put(&self, key: String, payload: Bytes, content_type: String, ttl: Option<Duration>) {
        let etag = format!("\"{:x}\"", ahash::AHasher::default().finish_with_bytes(&payload));
        let response = CachedResponse {
            payload,
            content_type,
            etag,
            compressed: false,
            cached_at: Instant::now(),
            ttl,
        };
        self.hot_cache.insert(key, response);
    }

    /// Retrieve entry from hot RAM cache or cold HTAR archive
    pub async fn get(&self, key: &str) -> Option<CachedResponse> {
        // 1. Try Hot RAM Cache
        if let Some(entry) = self.hot_cache.get(key) {
            if let Some(ttl) = entry.ttl {
                if entry.cached_at.elapsed() > ttl {
                    drop(entry);
                    self.hot_cache.remove(key);
                } else {
                    let mut stats = self.stats.write().await;
                    stats.hits += 1;
                    stats.bytes_served += entry.payload.len() as u64;
                    return Some(entry.clone());
                }
            } else {
                let mut stats = self.stats.write().await;
                stats.hits += 1;
                stats.bytes_served += entry.payload.len() as u64;
                return Some(entry.clone());
            }
        }

        // 2. Try Cold HTAR Archive if available
        if let Some(reader_lock) = &self.reader {
            let mut reader = reader_lock.write().await;
            if let Some(meta) = reader.get_entry(key).cloned() {
                if let Ok(payload_bytes) = reader.read_payload(key) {
                    let payload = Bytes::from(payload_bytes);
                    let res = CachedResponse {
                        payload: payload.clone(),
                        content_type: meta.content_type,
                        etag: meta.etag,
                        compressed: meta.compression != CompressionType::None,
                        cached_at: Instant::now(),
                        ttl: None,
                    };

                    // Promote to hot cache
                    self.hot_cache.insert(key.to_string(), res.clone());

                    let mut stats = self.stats.write().await;
                    stats.hits += 1;
                    stats.archive_loads += 1;
                    stats.bytes_served += payload.len() as u64;

                    return Some(res);
                }
            }
        }

        let mut stats = self.stats.write().await;
        stats.misses += 1;
        None
    }

    /// Clear hot cache
    pub fn clear_hot(&self) {
        self.hot_cache.clear();
    }
}

trait QuickHash {
    fn finish_with_bytes(&mut self, bytes: &[u8]) -> u64;
}

impl QuickHash for ahash::AHasher {
    fn finish_with_bytes(&mut self, bytes: &[u8]) -> u64 {
        use std::hash::Hasher;
        self.write(bytes);
        self.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_hot_put_get() {
        let cache = HtarCacheManager::new();
        cache.put(
            "/users".to_string(),
            Bytes::from("user_data"),
            "application/json".to_string(),
            None,
        );

        let res = cache.get("/users").await;
        assert!(res.is_some());
        assert_eq!(res.unwrap().payload, Bytes::from("user_data"));
    }
}
