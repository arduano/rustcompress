use std::io::{Read, Write};

use super::super::range_codec::{RangeDecoder, RangeEncPrice, RangeEncProbability, RangeEncoder};

#[derive(Debug, Clone)]
pub(crate) struct LiteralSubcoder {
    probs: [RangeEncProbability; 256 * 3],
}

impl LiteralSubcoder {
    pub fn new() -> Self {
        Self {
            probs: [RangeEncProbability::new(); 256 * 3],
        }
    }

    pub fn encode_normal_literal(
        &mut self,
        rc: &mut RangeEncoder<impl Write>,
        symbol: u8,
    ) -> std::io::Result<()> {
        // Add an extra 1 bit to the symbol on the 9th bit
        // This is helpful for iteration later
        let mut symbol = symbol as u32 | 0x100;

        loop {
            let subencoder_index = symbol >> 8;
            let bit = (symbol >> 7) & 1;
            rc.encode_bit(&mut self.probs[subencoder_index as usize], bit)?;
            symbol <<= 1;
            if symbol >= (0x100 << 8) {
                break;
            }
        }

        Ok(())
    }

    pub fn encode_matched_literal(
        &mut self,
        rc: &mut RangeEncoder<impl Write>,
        symbol: u8,
        match_byte: u8,
    ) -> std::io::Result<()> {
        // Add an extra 1 bit to the symbol on the 9th bit
        // This is helpful for iteration later
        let mut symbol = symbol as u32 | 0x100;
        let mut match_byte = match_byte as u32;

        let mut offset = 0x100;

        loop {
            match_byte <<= 1;
            let match_bit = match_byte as u32 & offset;
            let subencoder_index = offset + match_bit + (symbol >> 8);
            let bit = (symbol >> 7) & 1;
            rc.encode_bit(&mut self.probs[subencoder_index as usize], bit)?;
            symbol <<= 1;
            offset &= !(match_byte ^ symbol);
            if symbol >= (0x100 << 8) {
                break;
            }
        }

        Ok(())
    }

    pub fn decode_normal_literal(
        &mut self,
        rc: &mut RangeDecoder<impl Read>,
    ) -> Result<u8, std::io::Error> {
        let mut symbol: u32 = 1;
        loop {
            let b = rc.decode_bit(&mut self.probs[symbol as usize])? as u32;
            symbol = (symbol << 1) | b;
            if symbol >= 0x100 {
                break;
            }
        }
        Ok(symbol as u8)
    }

    pub fn decode_matched_literal(
        &mut self,
        rc: &mut RangeDecoder<impl Read>,
        match_byte: u8,
    ) -> Result<u8, std::io::Error> {
        let mut symbol: u32 = 1;
        let mut match_byte = match_byte as u32;
        let mut offset = 0x100;
        loop {
            match_byte = match_byte << 1;
            let match_bit = match_byte & offset;

            let bit = rc.decode_bit(&mut self.probs[(offset + match_bit + symbol) as usize])?;

            symbol = (symbol << 1) | bit;
            offset &= (0u32.wrapping_sub(bit)) ^ !match_bit;
            if symbol >= 0x100 {
                break;
            }
        }
        Ok(symbol as u8)
    }

    pub fn get_normal_price(&self, symbol: u8) -> RangeEncPrice {
        let mut price = RangeEncPrice::zero();

        // Add an extra 1 bit to the symbol on the 9th bit
        // This is helpful for iteration later
        let mut symbol = symbol as u32 | 0x100;

        loop {
            let subencoder_index = symbol >> 8;
            let bit = (symbol >> 7) & 1;
            price += self.probs[subencoder_index as usize].get_bit_price(bit);
            symbol <<= 1;
            if symbol >= (0x100 << 8) {
                break;
            }
        }
        price
    }

    pub fn get_matched_price(&self, symbol: u8, match_byte: u8) -> RangeEncPrice {
        let mut price = RangeEncPrice::zero();

        // Add an extra 1 bit to the symbol on the 9th bit
        // This is helpful for iteration later
        let mut symbol = symbol as u32 | 0x100;
        let mut match_byte = match_byte as u32;

        let mut offset = 0x100;

        loop {
            match_byte <<= 1;
            let match_bit = match_byte & offset;
            let subencoder_index = offset + match_bit + (symbol >> 8);
            let bit = (symbol >> 7) & 1;
            price += self.probs[subencoder_index as usize].get_bit_price(bit);
            symbol <<= 1;
            offset &= !(match_byte ^ symbol);
            if symbol >= (0x100 << 8) {
                break;
            }
        }
        return price;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_literal_normal_codec() {
        let mut buf = Vec::new();

        let mut subcoder = LiteralSubcoder::new();
        let mut encoder = RangeEncoder::new(&mut buf);
        for i in 0..256 {
            subcoder
                .encode_normal_literal(&mut encoder, i as u8)
                .unwrap();
        }
        encoder.finish().unwrap();

        let mut subcoder = LiteralSubcoder::new();
        let mut decoder = RangeDecoder::new(Cursor::new(buf)).unwrap();
        for i in 0..256 {
            let result = subcoder.decode_normal_literal(&mut decoder).unwrap();
            assert_eq!(result, i as u8);
        }

        assert!(decoder.is_finished());
    }

    #[test]
    fn test_literal_matched_codec() {
        let mut buf = Vec::new();

        let match_byte = 127;

        let mut subcoder = LiteralSubcoder::new();
        let mut encoder = RangeEncoder::new(&mut buf);
        for i in 0..256 {
            subcoder
                .encode_matched_literal(&mut encoder, i as u8, match_byte)
                .unwrap();
        }
        encoder.finish().unwrap();

        let mut subcoder = LiteralSubcoder::new();
        let mut decoder = RangeDecoder::new(Cursor::new(buf)).unwrap();
        for i in 0..256 {
            let result = subcoder
                .decode_matched_literal(&mut decoder, match_byte)
                .unwrap();
            assert_eq!(result, i as u8);
        }

        assert!(decoder.is_finished());
    }
}
