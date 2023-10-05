use std::io::{Cursor, Read, Write};

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use lzma_rust::LZMAReader;
use rustcompress::compressors::lzma::{
    codecs::{
        header_codec::parse_lzma_header, lzma_stream_codec::LZMACodecDecoder,
        range_codec::RangeDecoder,
    },
    data_buffers::DecoderDataBuffer,
};

fn fibonacci(n: u64) -> u64 {
    match n {
        0 => 1,
        1 => 1,
        n => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

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
    for i in 0..1000 {
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
