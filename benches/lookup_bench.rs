use criterion::{criterion_group, criterion_main, Criterion};
use htar_core::{CompressionType, HtarReader, HtarWriter};
use std::io::Cursor;

fn htar_lookup_benchmark(c: &mut Criterion) {
    let mut buffer = Vec::new();
    let mut writer = HtarWriter::new(&mut buffer);

    for i in 0..1000 {
        let key = format!("GET:/api/v1/data_{}", i);
        let payload = format!("{{\"item\": {}, \"value\": \"sample_payload_{}\"}}", i, i);
        writer.add_entry(key, payload.as_bytes(), "application/json", CompressionType::Zstd).unwrap();
    }
    writer.finish().unwrap();

    let mut reader = HtarReader::open(Cursor::new(&buffer)).unwrap();

    c.bench_function("htar_hashtable_random_read", |b| {
        b.iter(|| {
            let _ = reader.read_payload("GET:/api/v1/data_500").unwrap();
        })
    });
}

criterion_group!(benches, htar_lookup_benchmark);
criterion_main!(benches);
