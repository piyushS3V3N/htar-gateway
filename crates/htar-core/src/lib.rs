pub mod index;
pub mod reader;
pub mod writer;

pub use index::{CompressionType, HtarEntry, HtarIndex};
pub use reader::{HtarError, HtarReader};
pub use writer::HtarWriter;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_htar_write_and_read() {
        let mut buffer = Vec::new();
        let mut writer = HtarWriter::new(&mut buffer);

        let data1 = b"Hello, World! This is a test response payload for HTAR gateway cache.";
        let data2 = b"{\"status\": \"ok\", \"service\": \"user-service\", \"code\": 200}";

        writer.add_entry("/api/v1/hello", data1, "text/plain", CompressionType::Zstd).unwrap();
        writer.add_entry("/api/v1/user/status", data2, "application/json", CompressionType::Zstd).unwrap();
        writer.finish().unwrap();

        assert!(!buffer.is_empty());

        let mut reader = HtarReader::open(Cursor::new(buffer)).unwrap();
        assert_eq!(reader.len(), 2);

        let entry1 = reader.get_entry("/api/v1/hello").unwrap();
        assert_eq!(entry1.content_type, "text/plain");

        let payload1 = reader.read_payload("/api/v1/hello").unwrap();
        assert_eq!(payload1, data1);

        let payload2 = reader.read_payload("/api/v1/user/status").unwrap();
        assert_eq!(payload2, data2);
    }
}
