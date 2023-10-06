use std::io::{Cursor, Write};

use lzma_rust::LZMA2Options;
use rustcompress::compressors::lzma::codecs::{
    header_codec::parse_lzma_header,
    lzma_stream_codec::{data_buffers::DecoderDataBuffer, LZMACodecDecoder},
    range_codec::RangeDecoder,
};

fn main() {
    let data = include_bytes!("./test_decompress.rs");

    let mut compressed = Vec::new();

    // let mut f = LzmaWriter::new_compressor(&mut compressed, 6).unwrap();
    // f.write(data).unwrap();
    // f.finish().unwrap();

    let counting_writer = lzma_rust::CountingWriter::new(&mut compressed);
    let mut writer = lzma_rust::LZMAWriter::new(
        counting_writer,
        &LZMA2Options::default(),
        true,
        false,
        Some(data.len() as u64),
    )
    .unwrap();
    writer.write_all(data).unwrap();
    writer.finish().unwrap();

    // dbg!(&compressed);

    // let mut output = File::create("test.rs.xz").unwrap();
    // output.write_all(&compressed).unwrap();

    let mut reader = Cursor::new(&compressed);

    let header = parse_lzma_header(&mut reader).unwrap();
    let mut out_buffer = DecoderDataBuffer::new(header.dict_size, header.uncompressed_size);

    dbg!(&header);

    let mut rc = RangeDecoder::new(&mut reader).unwrap();
    let mut decoder = LZMACodecDecoder::new(
        header.props.lc as u32,
        header.props.lp as u32,
        header.props.pb as u32,
    );

    let mut output = vec![0; header.uncompressed_size as usize];
    let mut flushed = 0;

    while flushed < header.uncompressed_size as usize {
        decoder.decode_one_packet(&mut rc, &mut out_buffer).unwrap();
        flushed += out_buffer.flush(&mut output[flushed..]);
        dbg!(String::from_utf8_lossy(&output[..flushed]));
        dbg!(flushed);
    }

    println!("{}", String::from_utf8_lossy(&output));
    dbg!(flushed);
}
