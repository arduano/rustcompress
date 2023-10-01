//! A brute force match finder, which searches for matches by comparing each byte in the
//! input buffer with each byte in the history buffer.
//!
//! This is mainly used for testing to ensure that the more complex match finders are
//! working correctly.

use crate::compressors::lzma::encoder_data_buffer::EncoderDataBuffer;

use super::{Match, MatchFinder};

pub struct BruteForceMatchFinder {
    max_match_len: u32,
    dict_size: u32,
}

impl BruteForceMatchFinder {
    pub fn new(max_match_len: u32, dict_size: u32) -> Self {
        Self {
            max_match_len,
            dict_size,
        }
    }
}

impl MatchFinder for BruteForceMatchFinder {
    fn find_and_write_matches(
        &mut self,
        buffer: &EncoderDataBuffer,
        output_matches_vec: &mut Vec<Match>,
    ) {
        output_matches_vec.clear();

        let read_len = buffer.available_bytes_forward() as usize;

        let dict_size = self.dict_size.min(buffer.available_bytes_back());

        let start = -(dict_size as i32);
        for i in start..0 {
            if buffer.get_byte(i) != buffer.get_byte(0) {
                continue;
            }

            let mut len = 0;
            while len + 1 < read_len
                && len + 1 < self.max_match_len as usize
                && buffer.get_byte(i + len as i32 + 1) == buffer.get_byte(len as i32 + 1)
            {
                len += 1;
            }

            if len >= 2 {
                dbg!(buffer.get_byte(i), buffer.get_byte(0));
                output_matches_vec.push(Match {
                    distance: -i as u32 - 1,
                    len: len as u32,
                });
            }
        }
    }

    fn skip_byte(&mut self, _buffer: &EncoderDataBuffer) {
        // N/A
    }
}
