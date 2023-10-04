use std::io::{self, Read, Write};

use crate::utils::const_variable_arr::ConstVariableArr;

use super::range_codec::{RangeDecoder, RangeEncPrice, RangeEncProbability, RangeEncoder};

/// A length-value codec for LZMA, storing probabilities for each bit in a tree.
///
/// BITS_EXP must be a power of two
///
/// **TODO:** Replace BITS_EXP with BITS when https://github.com/rust-lang/rust/issues/76560 is implemented
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LengthValueCodec<const BITS_EXP: usize> {
    probs: [RangeEncProbability; BITS_EXP],
}

impl<const BITS_EXP: usize> Default for LengthValueCodec<BITS_EXP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BITS_EXP: usize> LengthValueCodec<BITS_EXP> {
    pub fn new() -> Self {
        Self {
            probs: [RangeEncProbability::default(); BITS_EXP],
        }
    }

    // TODO: Convert the loops into for loops to allow loop unrolling?
    pub fn encode_bit_tree(
        &mut self,
        enc: &mut RangeEncoder<impl Write>,
        symbol: u32,
    ) -> io::Result<()> {
        debug_assert!(symbol < self.probs.len() as u32);

        let mut index = 1;
        let mut mask = BITS_EXP as u32; // len will always be a power of two
        loop {
            mask >>= 1; // Increment by 1 by shifting right

            let bit = symbol & mask;
            enc.encode_bit(&mut self.probs[index], bit)?;
            index <<= 1;
            if bit != 0 {
                index = index | 1; // TODO: Remove branching here?
            }
            if mask == 1 {
                // When the 1 bit reached the end, we're done
                break;
            }
        }

        Ok(())
    }

    pub fn encode_reverse_bit_tree(
        &mut self,
        enc: &mut RangeEncoder<impl Write>,
        symbol: u32,
    ) -> io::Result<()> {
        debug_assert!(symbol < self.probs.len() as u32);

        let mut index = 1u32;
        let mut symbol = symbol | BITS_EXP as u32; // Add a 1 bit on the end for easier iteration over the bits
        loop {
            let bit = symbol & 1;
            symbol >>= 1;
            enc.encode_bit(&mut self.probs[index as usize], bit)?;
            index = (index << 1) | bit;
            if symbol == 1 {
                break;
            }
        }
        Ok(())
    }

    pub fn decode_bit_tree(&mut self, dec: &mut RangeDecoder<impl Read>) -> io::Result<u32> {
        let mut symbol: u32 = 1;
        loop {
            symbol = (symbol << 1) | dec.decode_bit(&mut self.probs[symbol as usize])?;
            if symbol >= self.probs.len() as u32 {
                break;
            }
        }
        Ok(symbol - self.probs.len() as u32)
    }

    pub fn decode_reverse_bit_tree(
        &mut self,
        dec: &mut RangeDecoder<impl Read>,
    ) -> io::Result<u32> {
        let mut symbol: u32 = 1;
        let mut i = 0;
        let mut result = 0;
        loop {
            let bit = dec.decode_bit(&mut self.probs[symbol as usize])?;
            symbol = (symbol << 1) | bit;
            result |= bit << i;
            i += 1;
            if symbol >= self.probs.len() as u32 {
                break;
            }
        }
        Ok(result as u32)
    }

    pub fn get_bit_tree_price(&self, symbol: u32) -> RangeEncPrice {
        let mut price = RangeEncPrice::zero();
        let mut symbol = symbol | self.probs.len() as u32;
        loop {
            let bit = symbol & 1;
            symbol >>= 1;
            price += self.probs[symbol as usize].get_bit_price(bit);
            if symbol == 1 {
                break;
            }
        }
        price
    }

    pub fn get_reverse_bit_tree_price(&self, symbol: u32) -> RangeEncPrice {
        let mut price = RangeEncPrice::zero();
        let mut index = 1u32;
        let mut symbol = symbol | self.probs.len() as u32;
        loop {
            let bit = symbol & 1;
            symbol >>= 1;
            price += self.probs[index as usize].get_bit_price(bit);
            index = (index << 1) | bit;
            if symbol == 1 {
                break;
            }
        }
        price
    }
}

const POS_STATES_MAX: usize = 1 << 4;

const LOW_LENGTH_BITS: usize = 3;
const MID_LENGTH_BITS: usize = 3;
const HIGH_LENGTH_BITS: usize = 8;

const LOW_LENGTH_MAX: usize = 1 << LOW_LENGTH_BITS;
const MID_LENGTH_MAX: usize = 1 << MID_LENGTH_BITS;
const HIGH_LENGTH_MAX: usize = 1 << HIGH_LENGTH_BITS;

pub const MATCH_LEN_MIN: usize = 2;
pub const MATCH_LEN_MAX: usize =
    MATCH_LEN_MIN + LOW_LENGTH_MAX + MID_LENGTH_MAX + HIGH_LENGTH_MAX - 1;

const PRICE_UPDATE_INTERVAL: usize = 32;

const MAX_NICE_LEN: usize = LOW_LENGTH_MAX + MID_LENGTH_MAX + HIGH_LENGTH_MAX + 1;

/// The length probabilities and prices for a single pos_state.
#[derive(Debug, Clone)]
struct LengthCodecPosState {
    low: LengthValueCodec<LOW_LENGTH_MAX>,
    mid: LengthValueCodec<MID_LENGTH_MAX>,
}

impl LengthCodecPosState {
    fn new() -> Self {
        Self {
            low: LengthValueCodec::new(),
            mid: LengthValueCodec::new(),
        }
    }
}

/// Tracker for bit prices for each length value for this pos_state
#[derive(Debug, Clone)]
struct LengthCodecPosStatePrice {
    /// Counter for how many items have been written since the last price update
    counter: i32,
    /// Prices for each length value. The length is constant at compile time for performance,
    /// but the length is variable at runtime depending on nice_len.
    prices: ConstVariableArr<RangeEncPrice, MAX_NICE_LEN>,
}

impl LengthCodecPosStatePrice {
    pub fn new(nice_len: u32) -> Self {
        Self {
            counter: 0,
            prices: ConstVariableArr::new(RangeEncPrice::zero(), nice_len as usize),
        }
    }
}

/// A length codec for LZMA, storing probabilities for each bit in a tree, for each length range.
///
/// The length ranges (as defined in the LZMA spec) are:
/// - 2-9 (low, 3 bits)
/// - 10-17 (mid, 3 bits)
/// - 18-273 (high, 8 bits)
///
/// pos_state represents the modulo position relative to the encoder's `1 << pb` value, allowing for different
/// probability tables to be used for different relative positions.
#[derive(Debug, Clone)]
struct LengthCodec {
    pub first_bit: RangeEncProbability,
    pub second_bit: RangeEncProbability,

    pub pos_states: ConstVariableArr<LengthCodecPosState, POS_STATES_MAX>,

    pub high: LengthValueCodec<HIGH_LENGTH_MAX>,
}

impl LengthCodec {
    pub fn new(pb: u32) -> Self {
        Self {
            first_bit: RangeEncProbability::new(),
            second_bit: RangeEncProbability::new(),
            high: LengthValueCodec::new(),

            pos_states: ConstVariableArr::new(LengthCodecPosState::new(), 1 << pb),
        }
    }
}

pub struct LengthCodecEncoder {
    codec: LengthCodec,
    pos_state_prices: ConstVariableArr<LengthCodecPosStatePrice, POS_STATES_MAX>,
}

impl LengthCodecEncoder {
    pub fn new(pb: u32, nice_len: u32) -> Self {
        assert!(nice_len <= MAX_NICE_LEN as u32);
        assert!(pb <= 4);

        Self {
            codec: LengthCodec::new(pb),

            pos_state_prices: ConstVariableArr::new(
                LengthCodecPosStatePrice::new(nice_len),
                1 << pb,
            ),
        }
    }

    pub fn encode(
        &mut self,
        enc: &mut RangeEncoder<impl Write>,
        len: u32,
        pos_state: u32,
    ) -> io::Result<()> {
        debug_assert!(len >= MATCH_LEN_MIN as u32);
        debug_assert!(
            len < (MATCH_LEN_MIN + LOW_LENGTH_MAX + MID_LENGTH_MAX + HIGH_LENGTH_MAX) as u32
        );
        debug_assert!(pos_state < POS_STATES_MAX as u32);

        let mut len = len as usize - MATCH_LEN_MIN;
        let pos_state = pos_state as usize;
        self.pos_state_prices[pos_state].counter -= 1;

        let pos_state_data = &mut self.codec.pos_states[pos_state];

        if len < LOW_LENGTH_MAX {
            enc.encode_bit(&mut self.codec.first_bit, 0)?;
            pos_state_data.low.encode_bit_tree(enc, len as u32)?;
            return Ok(());
        }

        enc.encode_bit(&mut self.codec.first_bit, 1)?;
        len -= LOW_LENGTH_MAX;

        if len < MID_LENGTH_MAX {
            enc.encode_bit(&mut self.codec.second_bit, 0)?;
            pos_state_data.mid.encode_bit_tree(enc, len as u32)?;
            return Ok(());
        }

        enc.encode_bit(&mut self.codec.second_bit, 1)?;
        self.codec
            .high
            .encode_bit_tree(enc, (len - MID_LENGTH_MAX) as u32)?;

        Ok(())
    }

    pub fn get_price(&self, len: usize, pos_state: usize) -> RangeEncPrice {
        self.pos_state_prices[pos_state].prices[len - MATCH_LEN_MIN]
    }

    /// Update the prices of all pos_states that have counted down to 0.
    /// We count every time we encode a length for that pos_state,
    /// and when we reach 0 we update the prices.
    fn update_prices(&mut self) {
        for pos_state in 0..self.codec.pos_states.len() {
            if self.pos_state_prices[pos_state].counter <= 0 {
                self.pos_state_prices[pos_state].counter = PRICE_UPDATE_INTERVAL as _;
                self.update_prices_with_state(pos_state);
            }
        }
    }

    fn update_prices_with_state(&mut self, pos_state: usize) {
        let first_bit_price = self.codec.first_bit.get_bit_price(0);

        // TODO: This function uses a lot of loops, one for each number, and one for each tree length
        // within each number.

        // It may be faster to only update the prices of items that have *changed*.
        // This could be done with a bitmask, and comparing the bitmask of the index
        // to the bitmask of the changed price bits and if there's any overlap then it
        // needs to be updated (I think).

        let pos_state_data = &mut self.codec.pos_states[pos_state];
        let prices = self.pos_state_prices[pos_state].prices.as_mut_slice();

        let start = 0;
        let end = LOW_LENGTH_MAX;
        for i in start..end {
            prices[i] = first_bit_price + pos_state_data.low.get_bit_tree_price(i as u32);
        }

        let start = LOW_LENGTH_MAX;
        let end = LOW_LENGTH_MAX + MID_LENGTH_MAX;
        let first_bit_price = self.codec.first_bit.get_bit_price(1);
        let second_bit_price = self.codec.second_bit.get_bit_price(0);
        for i in start..end {
            prices[i] = first_bit_price
                + second_bit_price
                + pos_state_data.mid.get_bit_tree_price((i - start) as u32);
        }

        let start = LOW_LENGTH_MAX + MID_LENGTH_MAX;
        let end = prices.len();
        let second_bit_price = self.codec.second_bit.get_bit_price(1);
        for i in start..end {
            prices[i] = first_bit_price
                + second_bit_price
                + self.codec.high.get_bit_tree_price((i - start) as u32);
        }
    }
}

pub struct LengthCodecDecoder {
    codec: LengthCodec,
}

impl LengthCodecDecoder {
    pub fn new(pb: u32) -> Self {
        Self {
            codec: LengthCodec::new(pb),
        }
    }

    pub fn decode(&mut self, dec: &mut RangeDecoder<impl Read>, pos_state: u32) -> io::Result<u32> {
        if dec.decode_bit(&mut self.codec.first_bit)? == 0 {
            let l = self.codec.pos_states[pos_state as usize]
                .low
                .decode_bit_tree(dec)?
                + MATCH_LEN_MIN as u32;
            return Ok(l);
        }

        if dec.decode_bit(&mut self.codec.second_bit)? == 0 {
            let l = self.codec.pos_states[pos_state as usize]
                .mid
                .decode_bit_tree(dec)?
                + MATCH_LEN_MIN as u32
                + LOW_LENGTH_MAX as u32;

            return Ok(l);
        }

        let l = self.codec.high.decode_bit_tree(dec)?
            + MATCH_LEN_MIN as u32
            + LOW_LENGTH_MAX as u32
            + MID_LENGTH_MAX as u32;

        Ok(l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_length_value_codec() {
        let mut buf = Vec::new();

        let mut codec = LengthValueCodec::<256>::new();
        let mut encoder = RangeEncoder::new(&mut buf);
        for i in 0..256 {
            codec.encode_bit_tree(&mut encoder, i).unwrap();
        }
        encoder.finish().unwrap();

        assert_eq!(buf.len(), 227);

        let mut codec = LengthValueCodec::<256>::new();
        let mut decoder = RangeDecoder::new_stream(Cursor::new(buf)).unwrap();
        for i in 0..256 {
            let result = codec.decode_bit_tree(&mut decoder).unwrap();
            assert_eq!(result, i);
        }

        assert!(decoder.is_finished());
    }

    #[test]
    fn test_length_value_codec_reverse() {
        let mut buf = Vec::new();

        let mut codec = LengthValueCodec::<256>::new();
        let mut encoder = RangeEncoder::new(&mut buf);
        for i in 0..256 {
            codec.encode_reverse_bit_tree(&mut encoder, i).unwrap();
        }
        encoder.finish().unwrap();

        assert_eq!(buf.len(), 266);

        let mut codec = LengthValueCodec::<256>::new();
        let mut decoder = RangeDecoder::new_stream(Cursor::new(buf)).unwrap();
        for i in 0..256 {
            let result = codec.decode_reverse_bit_tree(&mut decoder).unwrap();
            assert_eq!(result, i);
        }

        assert!(decoder.is_finished());
    }

    #[test]
    fn test_length_codec() {
        let mut buf = Vec::new();

        let mut codec = LengthCodecEncoder::new(1, MAX_NICE_LEN as u32);
        let mut encoder = RangeEncoder::new(&mut buf);

        let max = (MATCH_LEN_MIN + LOW_LENGTH_MAX + MID_LENGTH_MAX + HIGH_LENGTH_MAX) as u32;

        for i in 2..max {
            codec.encode(&mut encoder, i, 0).unwrap();
        }
        for i in (2..max).rev() {
            codec.encode(&mut encoder, i, 0).unwrap();
        }

        encoder.finish().unwrap();

        assert_eq!(buf.len(), 475);

        let mut codec = LengthCodecDecoder::new(1);
        let mut decoder = RangeDecoder::new_stream(Cursor::new(buf)).unwrap();

        for i in 2..max {
            let result = codec.decode(&mut decoder, 0).unwrap();
            assert_eq!(result, i);
        }
        for i in (2..max).rev() {
            let result = codec.decode(&mut decoder, 0).unwrap();
            assert_eq!(result, i);
        }

        assert!(decoder.is_finished());
    }
}
