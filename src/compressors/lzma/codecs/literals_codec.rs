mod subcoder;

use std::io::{self, Read, Write};

use self::subcoder::LiteralSubcoder;

use super::range_codec::{RangeDecoder, RangeEncPrice, RangeEncProbability, RangeEncoder};

/// A struct that helps choose the probability set to use for encoding/decoding
/// the next literal based on the previous uncompressed byte. lp and lc are
/// LZMA parameters.
pub(crate) struct LiteralCoderContextBits {
    lc: u32,
    literal_pos_mask: u32,
}

impl LiteralCoderContextBits {
    pub fn new(lc: u32, lp: u32) -> Self {
        Self {
            lc,

            // Store the mask to avoid having to calculate it every time
            literal_pos_mask: (1 << lp) - 1,
        }
    }

    pub(crate) fn get_sub_coder_index(&self, prev_byte: u32, pos: u32) -> u32 {
        let low = prev_byte >> (8 - self.lc);
        let high = pos & self.literal_pos_mask;
        low | (high << self.lc)
    }
}

pub struct LiteralCodec {
    coder: LiteralCoderContextBits,
    sub_decoders: Vec<LiteralSubcoder>,
}

impl LiteralCodec {
    fn new(lc: u32, lp: u32) -> Self {
        let coder = LiteralCoderContextBits::new(lc, lp);

        let count = 1 << (lc + lp);
        let mut sub_decoders = Vec::with_capacity(count as usize);
        for _ in 0..count {
            sub_decoders.push(LiteralSubcoder::new());
        }

        Self {
            coder,
            sub_decoders,
        }
    }

    fn get_subcoder(&mut self, prev_byte: u32, pos: u32) -> &mut LiteralSubcoder {
        let i = self.coder.get_sub_coder_index(prev_byte, pos);
        &mut self.sub_decoders[i as usize]
    }
}

pub struct LiteralCodecDecoder {
    codec: LiteralCodec,
}

impl LiteralCodecDecoder {
    pub fn new(lc: u32, lp: u32) -> Self {
        Self {
            codec: LiteralCodec::new(lc, lp),
        }
    }

    fn decode_normal<R: Read>(
        &mut self,
        rc: &mut RangeDecoder<R>,
        prev_byte: u8,
        pos: usize,
    ) -> io::Result<u8> {
        let subcoder = self.codec.get_subcoder(prev_byte as u32, pos as u32);
        subcoder.decode_normal_literal(rc)
    }

    fn decode_matched<R: Read>(
        &mut self,
        rc: &mut RangeDecoder<R>,
        prev_byte: u8,
        pos: usize,
        prev_match_byte: u8,
    ) -> io::Result<u8> {
        let subcoder = self.codec.get_subcoder(prev_byte as u32, pos as u32);
        subcoder.decode_matched_literal(rc, prev_match_byte)
    }
}

pub struct LiteralCodecEncoder {
    codec: LiteralCodec,
}

impl LiteralCodecEncoder {
    pub fn new(lc: u32, lp: u32) -> Self {
        Self {
            codec: LiteralCodec::new(lc, lp),
        }
    }

    fn encode_normal<W: Write>(
        &mut self,
        rc: &mut RangeEncoder<W>,
        symbol: u8,
        prev_byte: u8,
        pos: usize,
    ) -> io::Result<()> {
        let subcoder = self.codec.get_subcoder(prev_byte as u32, pos as u32);
        subcoder.encode_normal_literal(rc, symbol)
    }

    fn encode_matched<W: Write>(
        &mut self,
        rc: &mut RangeEncoder<W>,
        symbol: u8,
        prev_byte: u8,
        pos: usize,
        match_byte: u8,
    ) -> io::Result<()> {
        let subcoder = self.codec.get_subcoder(prev_byte as u32, pos as u32);
        subcoder.encode_matched_literal(rc, symbol, match_byte)
    }
}
