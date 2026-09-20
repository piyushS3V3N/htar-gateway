use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Compression algorithm used for an entry payload
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompressionType {
    None = 0,
    Zstd = 1,
    Gzip = 2,
}

/// Metadata stored in the O(1) hashtable for each entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HtarEntry {
    /// URI path or cache key (e.g., "/api/v1/static/main.js" or "GET:/users?id=10")
    pub key: String,
    /// Absolute byte offset in the HTAR file where compressed payload starts
    pub offset: u64,
    /// Length of the compressed payload chunk in bytes
    pub compressed_size: u64,
    /// Original uncompressed size in bytes
    pub uncompressed_size: u64,
    /// Compression type applied to this entry payload
    pub compression: CompressionType,
    /// Content type header (e.g. "application/json", "text/css")
    pub content_type: String,
    /// HTTP ETag checksum
    pub etag: String,
    /// Creation timestamp (unix epoch ms)
    pub created_at: u64,
}

/// In-memory Hashtable Index structure for fast lookups
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HtarIndex {
    entries: HashMap<String, HtarEntry>,
}

impl HtarIndex {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn insert(&mut self, entry: HtarEntry) {
        self.entries.insert(entry.key.clone(), entry);
    }

    pub fn get(&self, key: &str) -> Option<&HtarEntry> {
        self.entries.get(key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &HtarEntry)> {
        self.entries.iter()
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, bincode_lite::BincodeError> {
        // Simple binary JSON or Bincode serialization for hashtable
        serde_json::to_vec(&self.entries).map_err(|_| bincode_lite::BincodeError)
    }

    pub fn from_bytes(slice: &[u8]) -> Result<Self, serde_json::Error> {
        let entries: HashMap<String, HtarEntry> = serde_json::from_slice(slice)?;
        Ok(Self { entries })
    }
}

pub mod bincode_lite {
    #[derive(Debug)]
    pub struct BincodeError;
}
