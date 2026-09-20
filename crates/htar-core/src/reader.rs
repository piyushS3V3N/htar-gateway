use crate::index::{CompressionType, HtarEntry, HtarIndex};
use crate::writer::MAGIC_HEADER;
use std::io::{self, Read, Seek, SeekFrom};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HtarError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Invalid magic signature in HTAR archive")]
    InvalidMagic,
    #[error("Corrupted HTAR index footer")]
    CorruptedIndex,
    #[error("Decompression failed for key '{0}': {1}")]
    DecompressionFailed(String, String),
    #[error("Entry '{0}' not found in HTAR archive")]
    NotFound(String),
}

pub struct HtarReader<R: Read + Seek> {
    reader: R,
    index: HtarIndex,
    total_size: u64,
}

impl<R: Read + Seek> HtarReader<R> {
    pub fn open(mut reader: R) -> Result<Self, HtarError> {
        let total_size = reader.seek(SeekFrom::End(0))?;
        if total_size < 16 {
            return Err(HtarError::CorruptedIndex);
        }

        // Read 16-byte trailer: index_offset (8B) + index_len (4B) + magic (4B)
        reader.seek(SeekFrom::End(-16))?;
        let mut trailer = [0u8; 16];
        reader.read_exact(&mut trailer)?;

        let magic = &trailer[12..16];
        if magic != MAGIC_HEADER {
            return Err(HtarError::InvalidMagic);
        }

        let index_offset = u64::from_le_bytes(trailer[0..8].try_into().unwrap());
        let index_len = u32::from_le_bytes(trailer[8..12].try_into().unwrap()) as u64;

        if index_offset + index_len > total_size - 16 {
            return Err(HtarError::CorruptedIndex);
        }

        // Seek directly to index offset
        reader.seek(SeekFrom::Start(index_offset))?;
        let mut index_bytes = vec![0u8; index_len as usize];
        reader.read_exact(&mut index_bytes)?;

        let index = HtarIndex::from_bytes(&index_bytes)
            .map_err(|_| HtarError::CorruptedIndex)?;

        Ok(Self {
            reader,
            index,
            total_size,
        })
    }

    pub fn total_size(&self) -> u64 {
        self.total_size
    }

    pub fn index(&self) -> &HtarIndex {
        &self.index
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Lookup metadata for a given cache key
    pub fn get_entry(&self, key: &str) -> Option<&HtarEntry> {
        self.index.get(key)
    }

    /// Fetch and decompress payload for a key in O(1) time
    pub fn read_payload(&mut self, key: &str) -> Result<Vec<u8>, HtarError> {
        let entry = self.index.get(key).cloned().ok_or_else(|| HtarError::NotFound(key.to_string()))?;

        self.reader.seek(SeekFrom::Start(entry.offset))?;
        let mut compressed = vec![0u8; entry.compressed_size as usize];
        self.reader.read_exact(&mut compressed)?;

        match entry.compression {
            CompressionType::None => Ok(compressed),
            CompressionType::Zstd => {
                zstd::decode_all(&compressed[..])
                    .map_err(|e| HtarError::DecompressionFailed(key.to_string(), e.to_string()))
            }
            CompressionType::Gzip => {
                let mut gz = flate2::read::GzDecoder::new(&compressed[..]);
                let mut decompressed = Vec::with_capacity(entry.uncompressed_size as usize);
                gz.read_to_end(&mut decompressed)
                    .map_err(|e| HtarError::DecompressionFailed(key.to_string(), e.to_string()))?;
                Ok(decompressed)
            }
        }
    }
}
