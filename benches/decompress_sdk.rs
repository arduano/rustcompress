use std::{
    io::{Cursor, Read, Write},
    time::Duration,
};

use criterion::{criterion_group, criterion_main, Criterion};
use lzma::{LzmaReader, LzmaWriter};

fn criterion_benchmark(c: &mut Criterion) {
    let data = include_bytes!("../src/compressors/lzma/codecs/range_codec.rs");

    let mut compressed = Vec::new();

    let mut writer = LzmaWriter::new_compressor(&mut compressed, 6).unwrap();

    for _i in 0..1000 {
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap();

    let mut output = Vec::with_capacity(data.len() * 1000);
    let mut c = c.benchmark_group("sdk");
    c.measurement_time(Duration::from_secs(60));
    c.bench_function("decompress small sdk", |b| {
        b.iter(|| {
            output.clear();

            let reader = Cursor::new(&compressed[..]);
            let mut reader = LzmaReader::new_decompressor(reader).unwrap();

            reader.read_to_end(&mut output).unwrap()
        })
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
