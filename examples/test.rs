use std::io::Cursor;

use rustcompress::compressors::lzma::codecs::range_codec::{RangeDecoder, RangeEncoder};

fn main() {
    let mut buf = Vec::new();

    let mut encoder = RangeEncoder::new(&mut buf);
    for i in 0..100 {
        encoder.encode_direct_bits(i, 8).unwrap();
    }
    encoder.finish().unwrap();

    dbg!(buf.len());

    assert_eq!(buf.len(), 105);

    let mut decoder = RangeDecoder::new_stream(Cursor::new(buf)).unwrap();

    for i in 0..100 {
        let result = decoder.decode_direct_bits(8).unwrap();
        assert_eq!(result, i);
    }

    assert!(decoder.is_finished());
}
