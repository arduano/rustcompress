use super::super::super::data_buffers::EncoderDataBuffer;

use super::utils::{
    cyclic_vec::CyclicVec,
    hash234::Hash234,
    match_positions::{MatchPos, MatchReadPos, PosIncrementResult},
};
use super::{Match, MatchFinder};

pub struct HC4MatchFinder {
    hash: Hash234<MatchPos>,
    chain: CyclicVec<MatchPos>,
    depth_limit: i32,
    nice_len: u32,
    max_match_len: u32,

    /// The relative index of the current "read head". The index isn't absolute as it gets shifted down once it reaches 0x7FFFFFFF.
    lz_pos: MatchReadPos,
}

fn get_next_4_bytes(buffer: &EncoderDataBuffer) -> [u8; 4] {
    [
        buffer.get_byte(0),
        buffer.get_byte(1),
        buffer.get_byte(2),
        buffer.get_byte(3),
    ]
}

impl HC4MatchFinder {
    pub fn get_mem_usage(dict_size: u32) -> u64 {
        Hash234::<MatchPos>::get_mem_usage(dict_size)
            + dict_size as u64 * std::mem::size_of::<MatchPos>() as u64
    }

    pub fn new(dict_size: u32, nice_len: u32, max_match_len: u32, depth_limit: i32) -> Self {
        Self {
            hash: Hash234::new(dict_size),
            chain: CyclicVec::new(dict_size as usize + 1),
            // matches: Matches::new(nice_len as usize - 1),
            depth_limit: if depth_limit > 0 {
                depth_limit as i32
            } else {
                4 + nice_len as i32 / 4
            },
            nice_len,
            max_match_len,
            lz_pos: MatchReadPos::new(dict_size + 1),
        }
    }

    fn increment_pos(&mut self, buffer: &EncoderDataBuffer) {
        if buffer.forwards_bytes() != 0 {
            let result = self.lz_pos.increment();

            if result == PosIncrementResult::ShouldNormalize {
                let norm_offset = self.lz_pos.get_norm_offset(self.chain.len());

                self.hash.map_all_values(|v| v.subtract_offset(norm_offset));
                self.chain
                    .iter_mut()
                    .for_each(|v| *v = v.subtract_offset(norm_offset));

                self.lz_pos = self.lz_pos.subtract_offset(norm_offset);
            }
        }
    }
}

impl MatchFinder for HC4MatchFinder {
    const MIN_FORWARDS_BYTES: u32 = 4;

    fn find_and_write_matches(
        &mut self,
        buffer: &EncoderDataBuffer,
        output_matches_vec: &mut Vec<Match>,
    ) {
        output_matches_vec.clear();

        let lz_pos = self.lz_pos;

        self.increment_pos(buffer);
        let avail = buffer.forwards_bytes() as u32;

        let mut max_match_len = self.max_match_len;
        let mut nice_len = self.nice_len;
        if avail < self.max_match_len {
            if avail == 0 {
                return;
            }
            max_match_len = avail;
            if nice_len > avail {
                nice_len = avail;
            }
        }

        let bytes = get_next_4_bytes(buffer);
        let index = self.hash.calc_hash_index(bytes); // Grab the guessed indexes for the byte values
        let positions = self.hash.get_table_values(&index); // Get the delta values at those table indexes
        self.hash.update_tables(&index, self.lz_pos.as_match_pos()); // Update the tables with the new position

        let delta2 = lz_pos - positions.hash2_value;
        let delta3 = lz_pos - positions.hash3_value;
        self.chain.push(positions.hash4_value);

        let mut len_best = 0;

        // Check if the byte at the current position matches the byte delta2 positions behind it.
        // If so, update the best match length and add a new match to the output vector.
        if delta2 < self.chain.len() as u32
            && buffer.do_bytes_match_at(delta2, 0)
            && buffer.do_bytes_match_at(delta2, 1)
        {
            len_best = 2;
            output_matches_vec.push(Match {
                distance: delta2,
                len: 2,
            });
        }

        let mut latest_delta = delta2;

        // Check if the byte delta3 positions behind the current position matches the current byte.
        // If so, update the best match length and add a new match to the output vector.
        // Set delta2 to delta3 to check for longer matches in the next iteration.
        if latest_delta != delta3
            && delta3 < self.chain.len() as u32
            && buffer.do_bytes_match_at(delta3, 0)
            && buffer.do_bytes_match_at(delta3, 1)
            && buffer.do_bytes_match_at(delta3, 2)
        {
            len_best = 3;
            output_matches_vec.push(Match {
                distance: delta3,
                len: 3,
            });
            latest_delta = delta3;
        }

        // If there are matches in the output vector, check if the current byte matches the byte latest_delta positions behind it.
        // If so, increment the best match length until it reaches the match length limit or the bytes no longer match.
        // If the best match length is long enough, return from the function.
        if output_matches_vec.len() > 0 {
            while len_best < max_match_len && buffer.do_bytes_match_at(latest_delta, len_best) {
                len_best += 1;
            }

            let last = output_matches_vec.last_mut().unwrap();
            last.len = len_best;

            // Return if it is long enough (niceLen or reached the end of
            // the dictionary).
            if len_best >= nice_len {
                return;
            }
        }

        // After this point, the best match length can only be 3 or higher
        // Because we're checking hash4_value which uses all 3 bytes.
        if len_best < 3 {
            len_best = 3;
        }

        // Create an iterator that iterates through the chain of past matches with the same hash4.
        let mut current_match = Some(positions.hash4_value);
        let chain_delta_iter = std::iter::from_fn(|| {
            let val = current_match?;

            let delta = lz_pos - val;
            if delta + 1 < self.chain.len() as u32 {
                current_match = Some(*self.chain.get_backwards(delta as usize + 1));
                Some(delta)
            } else {
                current_match = None;
                None
            }
        });

        // Using the best known match hash, search through the chain of matches to find a longer match.
        for delta in chain_delta_iter.take(self.depth_limit as usize) {
            // First, check if the start and end bytes mach, to sanity check the match.

            if delta == latest_delta {
                // If the delta is the same as the last delta, then we've already checked this match.
                // Skip it.
                continue;
            }

            if buffer.do_bytes_match_at(delta, len_best) && buffer.do_bytes_match_at(delta, 0) {
                // Calculate the length of the match.
                let mut len = 1;
                while len < max_match_len && buffer.do_bytes_match_at(delta, len) {
                    len += 1;
                }

                // Use the match if and only if it is better than the longest
                // match found so far.
                if len > len_best {
                    len_best = len;

                    output_matches_vec.push(Match {
                        distance: delta,
                        len: len,
                    });

                    // Return if it is long enough (nice_len or reached the
                    // end of the dictionary).
                    if len >= nice_len {
                        return;
                    }
                }
            }
        }
    }

    fn skip_byte(&mut self, buffer: &EncoderDataBuffer) {
        if buffer.forwards_bytes() != 0 {
            let index = self.hash.calc_hash_index(get_next_4_bytes(buffer)); // Grab the guessed indexes for the byte values
            let positions = self.hash.get_table_values(&index); // Get the delta values at those table indexes
            self.hash.update_tables(&index, self.lz_pos.as_match_pos()); // Update the tables with the new position

            // We don't increment lz_pos so we don't need to increment the cyclic vec either
            self.chain.replace_current(positions.hash4_value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::brute_force::BruteForceMatchFinder;

    use super::*;

    fn assert_matches_equal(matches_1: &[Match], matches_2: &[Match]) {
        let mut matches1 = matches_1.to_vec();
        let mut matches2 = matches_2.to_vec();

        // dbg!(&matches1);
        // dbg!(&matches2);

        for m in &matches1 {
            if matches2.iter().find(|&m2| m == m2).is_some() {
                // Remove every match2 with the same length but different distance
                let mut i = 0;
                while i < matches2.len() {
                    if matches2[i].len == m.len && matches2[i].distance != m.distance {
                        matches2.remove(i);
                    } else {
                        i += 1;
                    }
                }
            }
        }

        matches1.sort();
        matches2.sort();

        assert_eq!(matches1, matches2);
    }

    #[test]
    fn test_find_short_matches() {
        let mut data = vec![];
        data.extend(vec![0; 1000]); // Pad the data with zeros

        for i in 0..10 {
            data.push(255); // Short 2 byte matches
            data.push(i);
            data.extend(vec![0; 10]); // Space sequences with zeros
        }

        for i in 0..10 {
            data.push(255); // Same matches again
            data.push(i);
            data.extend(vec![0; 10]); // Space sequences with zeros
        }

        let mut buffer = EncoderDataBuffer::new(1000, 2000);
        buffer.append_data(&data);
        buffer.skip(1000); // Skip the padding

        let mut hc4 = HC4MatchFinder::new(998, 5, 5, 10); // max_match_len and nice_len set to allow short matches
        let mut brute = BruteForceMatchFinder::new(5, 998);

        let mut out_vec_1 = Vec::new();
        let mut out_vec_2 = Vec::new();

        for _ in 0..buffer.forwards_bytes() - 4 {
            brute.find_and_write_matches(&buffer, &mut out_vec_2);
            hc4.find_and_write_matches(&buffer, &mut out_vec_1);

            if buffer.get_byte(0) == 255 {
                // If we reach a short match byte, test that the matches are the same
                assert_matches_equal(&out_vec_1, &out_vec_2);
            }

            buffer.increment_pos();
        }
    }

    #[test]
    fn test_find_long_matches() {
        let mut data = vec![];
        data.extend(vec![0; 1000]); // Pad the data with zeros

        let sequence = [1, 2, 3, 4, 5, 6, 7, 8, 9];

        for i in 2..sequence.len() {
            data.push(255); // Match start identifier for testing
            data.extend(&sequence[0..(sequence.len() - i + 2)]); // Long matches, in decreasing length order
            data.push(i as u8 + 100); // Some random data to pad the sequences
            data.extend(vec![0; 10]); // Space sequences with zeros
        }

        let mut buffer = EncoderDataBuffer::new(1000, 2000);
        buffer.append_data(&data);
        buffer.skip(1000); // Skip the padding

        let mut hc4 = HC4MatchFinder::new(998, 12, 12, 20); // max_match_len and nice_len set to allow short matches
        let mut brute = BruteForceMatchFinder::new(12, 998);

        let mut out_vec_1 = Vec::new();
        let mut out_vec_2 = Vec::new();

        for _ in 0..buffer.forwards_bytes() - 4 {
            brute.find_and_write_matches(&buffer, &mut out_vec_2);

            hc4.find_and_write_matches(&buffer, &mut out_vec_1);

            if buffer.get_byte(0) == 255 && out_vec_2.len() > 2 {
                // If we reach a short match byte, test that the matches are the same
                assert_matches_equal(&out_vec_1, &out_vec_2);
            }

            buffer.increment_pos();
        }
    }
}
