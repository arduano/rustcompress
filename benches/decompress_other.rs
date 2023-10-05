use std::io::{Cursor, Read, Write};

use criterion::{criterion_group, criterion_main, Criterion};
use lzma_rust::LZMAReader;

fn criterion_benchmark(c: &mut Criterion) {
    let data = include_bytes!("../src/compressors/lzma/codecs/range_codec.rs");

    let mut compressed = Vec::new();

    let counting_writer = lzma_rust::CountingWriter::new(&mut compressed);
    let mut writer = lzma_rust::LZMAWriter::new(
        counting_writer,
        &lzma_rust::LZMA2Options::default(),
        true,
        false,
        Some(data.len() as u64 * 1000),
    )
    .unwrap();
    for _i in 0..1000 {
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap();

    let mut output = Vec::with_capacity(data.len() * 1000);
    c.bench_function("decompress small other", |b| {
        b.iter(|| {
            output.clear();
            let reader = Cursor::new(&compressed);
            let mut parser = LZMAReader::new_mem_limit(reader, u32::MAX, None).unwrap();

            parser.read_to_end(&mut output).unwrap();
        })
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
