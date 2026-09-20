use crate::index::{CompressionType, HtarEntry, HtarIndex};
use std::io::{self, Write};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAGIC_HEADER: &[u8; 4] = b"HTAR";

pub struct HtarWriter<W: Write> {
    writer: W,
    index: HtarIndex,
    current_offset: u64,
    compression_level: i32,
}

impl<W: Write> HtarWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            index: HtarIndex::new(),
            current_offset: 0,
            compression_level: 3, // Default Zstd compression level
        }
    }

    pub fn with_compression_level(mut self, level: i32) -> Self {
        self.compression_level = level;
        self
    }

    /// Add a payload entry into the archive
    pub fn add_entry(
        &mut self,
        key: impl Into<String>,
        payload: &[u8],
        content_type: &str,
        compression: CompressionType,
    ) -> io::Result<()> {
        let key_str = key.into();
        let uncompressed_size = payload.len() as u64;
        let start_offset = self.current_offset;

        let compressed_bytes = match compression {
            CompressionType::None => payload.to_vec(),
            CompressionType::Zstd => zstd::encode_all(payload, self.compression_level)?,
            CompressionType::Gzip => {
                let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(payload)?;
                encoder.finish()?
            }
        };

        let compressed_size = compressed_bytes.len() as u64;

        // Write TAR block-style entry framing
        // Header block: Key len (2B) + Content-Type len (2B) + Key + Content-Type + Compressed payload
        let key_bytes = key_str.as_bytes();
        let ct_bytes = content_type.as_bytes();

        let header_meta = format!("{}:{}", key_bytes.len(), ct_bytes.len());
        let meta_len_bytes = (header_meta.len() as u16).to_le_bytes();

        self.writer.write_all(&meta_len_bytes)?;
        self.writer.write_all(header_meta.as_bytes())?;
        self.writer.write_all(key_bytes)?;
        self.writer.write_all(ct_bytes)?;
        self.writer.write_all(&compressed_bytes)?;

        let payload_offset = start_offset
            + 2
            + header_meta.len() as u64
            + key_bytes.len() as u64
            + ct_bytes.len() as u64;

        let total_entry_bytes = payload_offset + compressed_size - start_offset;
        self.current_offset += total_entry_bytes;

        // ETag computed from hash
        let etag = format!("\"{:x}\"", ahash::AHasher::default().finish_with_bytes(payload));
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let entry = HtarEntry {
            key: key_str,
            offset: payload_offset,
            compressed_size,
            uncompressed_size,
            compression,
            content_type: content_type.to_string(),
            etag,
            created_at,
        };

        self.index.insert(entry);
        Ok(())
    }

    /// Finalize writing by appending the Hashtable Index segment & Footer
    pub fn finish(mut self) -> io::Result<W> {
        let index_bytes = self.index.to_bytes().map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("Failed to serialize index: {:?}", e))
        })?;

        let index_offset = self.current_offset;
        let index_len = index_bytes.len() as u32;

        // Write Index payload
        self.writer.write_all(&index_bytes)?;

        // Write Footer: Index Offset (8B) + Index Len (4B) + Magic (4B)
        self.writer.write_all(&index_offset.to_le_bytes())?;
        self.writer.write_all(&index_len.to_le_bytes())?;
        self.writer.write_all(MAGIC_HEADER)?;

        self.writer.flush()?;
        Ok(self.writer)
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
