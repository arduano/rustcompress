//! # Range Encoding and Decoding for LZMA
//!
//! ### Range Encoding
//!
//! Range encoding is a variant of arithmetic encoding that uses two main
//! variables: `low` and `range`. The `low` variable stores the lower bound
//! of the current range of the output, and `range` stores the size of the
//! current range. As each symbol is encoded, `low` and `range` are updated
//! to narrow down the range of possible output values. The most probable
//! symbols take up a larger portion of this range.
//!
//! ### Probability Modeling
//!
//! This implementation utilizes `RangeEncProbability`, instead of the usual
//! u16 used in other implementations, to make it more clear what's a
//! probability. Probabilities are used by the aglrotihm to determine how
//! likely a given symbol is to occur.

mod price;
mod probability;

pub use price::*;
pub use probability::*;

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

use std::io::{ErrorKind, Write};
use std::io::{Read, Result};

const SHIFT_BITS: u32 = 8;
const TOP_MASK: u32 = 0xFF000000;
const BIT_MODEL_TOTAL_BITS: u32 = 11;
const BIT_MODEL_TOTAL: u32 = 1 << BIT_MODEL_TOTAL_BITS;
const MOVE_BITS: u32 = 5;

const K_TOP_VALUE: u32 = 1 << (32 - SHIFT_BITS);

pub struct RangeEncoder<W: Write> {
    low: u64,
    range: u32,
    cache_size: u32,
    cache: u8,
    stream: W,
    finished: bool,
}

impl<W: Write> RangeEncoder<W> {
    pub fn new(inner: W) -> Self {
        Self {
            low: 0,
            range: 0xFFFFFFFFu32,
            cache_size: 1,
            cache: 0,
            stream: inner,
            finished: false,
        }
    }

    pub fn inner(&mut self) -> &mut W {
        &mut self.stream
    }

    pub fn finish(mut self) -> Result<()> {
        for _i in 0..5 {
            self.shift_low()?;
        }

        self.finished = true;

        Ok(())
    }

    fn write_byte(&mut self, b: u8) -> std::io::Result<()> {
        self.stream.write_u8(b)
    }

    fn shift_low(&mut self) -> std::io::Result<()> {
        let low_hi = (self.low >> 32) as u32;

        if low_hi != 0 || self.low < 0xFF000000u64 {
            let mut temp = self.cache;
            loop {
                self.write_byte((temp as u32 + low_hi) as u8)?;
                temp = 0xFF;
                self.cache_size -= 1;
                if self.cache_size == 0 {
                    break;
                }
            }
            self.cache = (self.low >> 24) as u8;
        }
        self.cache_size += 1;
        self.low = (self.low & 0x00FFFFFF) << 8;

        Ok(())
    }

    pub fn encode_bit(&mut self, prob: &mut RangeEncProbability, bit: u32) -> std::io::Result<()> {
        let bound = (self.range >> BIT_MODEL_TOTAL_BITS) * prob.0 as u32;
        if bit == 0 {
            // Encode the bit as a 0, and update the probability
            self.range = bound;
            prob.increment();
        } else {
            // Encode the bit as a 1, and update the probability
            self.low += bound as u64;
            self.range -= bound;
            prob.decrement();
        }

        if self.range & TOP_MASK == 0 {
            self.range = self.range << SHIFT_BITS;
            self.shift_low()?;
        }
        Ok(())
    }

    pub fn encode_direct_bits(&mut self, value: u32, mut count: u32) -> std::io::Result<()> {
        loop {
            self.range >>= 1;
            count = count - 1;

            let bit = (value >> count) & 1;
            let m = 0u32.wrapping_sub(bit); // 0 or 0xFFFFFFFF
            self.low += (self.range & m) as u64; // If bit is 0, add 0. If bit is 1, add range

            if self.range & TOP_MASK == 0 {
                self.range = self.range << SHIFT_BITS;
                self.shift_low()?;
            }
            if count == 0 {
                break;
            }
        }
        Ok(())
    }
}

impl<T: Write> std::ops::Drop for RangeEncoder<T> {
    fn drop(&mut self) {
        if !self.finished {
            panic!("RangeEncoder dropped without being finished");
        }
    }
}

pub struct RangeDecoder<R: Read> {
    stream: R,
    range: u32,
    code: u32,
}

impl<R: Read> RangeDecoder<R> {
    pub fn new(mut stream: R) -> Result<Self> {
        let b = stream.read_u8()?;
        if b != 0x00 {
            return Err(std::io::Error::new(
                ErrorKind::InvalidInput,
                "First byte of the range decoder stream must be 0x00",
            ));
        }
        let code = stream.read_u32::<BigEndian>()?;
        Ok(Self {
            stream,
            code,
            range: (0xFFFFFFFFu32),
        })
    }

    pub fn is_finished(&self) -> bool {
        self.code == 0
    }
}

impl<R: Read> RangeDecoder<R> {
    fn normalize(&mut self) -> Result<()> {
        if self.range < K_TOP_VALUE {
            let next = self.stream.read_u8()? as u32;
            self.code = (self.code << SHIFT_BITS) | next;
            self.range <<= SHIFT_BITS;
        }
        Ok(())
    }

    /// Decode a bit with a given probability
    pub fn decode_bit(&mut self, prob: &mut RangeEncProbability) -> Result<u32> {
        self.normalize()?;
        let bound = (self.range >> (BIT_MODEL_TOTAL_BITS as u32)) * prob.0 as u32;
        if self.code < bound {
            // Decode the bit as a 0, and update the probability
            self.range = bound;
            prob.increment();
            Ok(0)
        } else {
            // Decode the bit as a 1, and update the probability
            self.range -= bound;
            self.code -= bound;
            prob.decrement();
            Ok(1)
        }
    }

    /// Decode bits assuming the probability is 50/50
    pub fn decode_direct_bits(&mut self, count: u32) -> Result<u32> {
        let mut result = 0;
        for _ in 0..count {
            self.normalize()?;

            self.range >>= 1;

            let t = (self.code.wrapping_sub(self.range)) >> 31; // 0 or 1
            self.code -= self.range & (t.wrapping_sub(1)); // If 0, subtract 0. If 1, subtract range
            let bit = 1 - t; // Bit is 1 if t is 0, and 0 if t is 1

            // result = (result << 1) | bit;
            result = (result << 1) | bit;
        }
        Ok(result as _)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_range_encoder() {
        let mut buf = Vec::new();

        let mut encoder = RangeEncoder::new(&mut buf);
        for i in 0..100 {
            encoder.encode_direct_bits(i, 8).unwrap();
        }
        encoder.finish().unwrap();

        assert_eq!(buf.len(), 105);

        let mut decoder = RangeDecoder::new(Cursor::new(buf)).unwrap();

        for i in 0..100 {
            let result = decoder.decode_direct_bits(8).unwrap();
            assert_eq!(result, i);
        }

        assert!(decoder.is_finished());
    }

    #[test]
    fn test_range_encoder_probs() {
        let mut buf = Vec::new();

        let mut prob = RangeEncProbability::new();
        let mut encoder = RangeEncoder::new(&mut buf);
        for i in 0..100 {
            for bit in 0..32 {
                encoder.encode_bit(&mut prob, (i >> bit) & 1).unwrap();
            }
        }
        encoder.finish().unwrap();

        assert_eq!(buf.len(), 197);

        let mut prob = RangeEncProbability::new();
        let mut decoder = RangeDecoder::new(Cursor::new(buf)).unwrap();
        for i in 0..100 {
            let mut result = 0;
            for bit in 0..32 {
                result |= decoder.decode_bit(&mut prob).unwrap() << bit;
            }

            assert_eq!(result, i);
        }

        assert!(decoder.is_finished());
    }

    #[test]
    fn test_range_encoder_probs_zeroes() {
        let mut buf = Vec::new();

        let mut prob = RangeEncProbability::new();
        let mut encoder = RangeEncoder::new(&mut buf);
        for _ in 0..1000 {
            encoder.encode_bit(&mut prob, 0).unwrap();
        }
        encoder.encode_bit(&mut prob, 1).unwrap();
        for _ in 0..1000 {
            encoder.encode_bit(&mut prob, 0).unwrap();
        }
        encoder.finish().unwrap();

        assert_eq!(buf.len(), 14);

        let mut prob = RangeEncProbability::new();
        let mut decoder = RangeDecoder::new(Cursor::new(buf)).unwrap();
        for _ in 0..1000 {
            let result = decoder.decode_bit(&mut prob).unwrap();

            assert_eq!(result, 0);
        }

        let result = decoder.decode_bit(&mut prob).unwrap();
        assert_eq!(result, 1);

        for _ in 0..1000 {
            let result = decoder.decode_bit(&mut prob).unwrap();

            assert_eq!(result, 0);
        }

        assert!(decoder.is_finished());
    }
}
