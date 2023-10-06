use std::{
    io::{Cursor, Write},
    time::Duration,
};

use criterion::{criterion_group, criterion_main, Criterion};

use rustcompress::compressors::lzma::codecs::{
    header_codec::parse_lzma_header, lzma_stream_codec::{LZMACodecDecoder, data_buffers::DecoderDataBuffer}, range_codec::RangeDecoder,
};

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
    for _ in 0..1000 {
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap();

    dbg!(data.len() * 1000);

    let mut output = vec![0; data.len() * 1000];
    let mut c = c.benchmark_group("mine");
    c.measurement_time(Duration::from_secs(60));
    c.bench_function("decompress small mine", |b| {
        b.iter(|| {
            let mut reader = Cursor::new(&compressed);

            let header = parse_lzma_header(&mut reader).unwrap();
            let mut out_buffer = DecoderDataBuffer::new(header.dict_size, header.uncompressed_size);

            let mut rc = RangeDecoder::new(&mut reader).unwrap();
            let mut decoder = LZMACodecDecoder::new(
                header.props.lc as u32,
                header.props.lp as u32,
                header.props.pb as u32,
            );

            let mut flushed = 0;

            while flushed < header.uncompressed_size as usize {
                decoder.decode_one_packet(&mut rc, &mut out_buffer).unwrap();
                flushed += out_buffer.flush(&mut output[flushed..]);
            }
        })
    });
    c.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
