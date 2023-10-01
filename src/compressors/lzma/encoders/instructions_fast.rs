use crate::compressors::lzma::{
    length_codec::{MATCH_LEN_MAX, MATCH_LEN_MIN},
    match_finding::{Match, MatchFinder},
    LZMACoderState,
};

use super::{LZMAEncoderInput, LZMAInstructionPicker};

pub struct LZMAFastInstructionPicker {
    nice_len: u32,
}

const REPS: usize = 4;

impl LZMAInstructionPicker for LZMAFastInstructionPicker {
    // TODO: Ensure that when this function returns, input gets skipped by the match length - 1
    fn get_next_symbol(
        &mut self,
        input: &mut LZMAEncoderInput<impl MatchFinder>,
        state: &LZMACoderState,
    ) -> Option<Match> {
        let avail = input
            .buffer()
            .available_bytes_forward()
            .min(MATCH_LEN_MAX as u32);

        // If there aren't enough bytes to encode a match, just return None
        if avail < MATCH_LEN_MIN as u32 {
            return None;
        }

        let mut best_rep_len = 0;
        let mut best_rep_index = 0;
        for rep in state.reps {
            let len = input.buffer().projection().get_match_length(rep, avail);
            if len < MATCH_LEN_MIN as u32 {
                continue;
            }

            if len >= self.nice_len {
                return Some(Match { distance: rep, len });
            }

            if len > best_rep_len {
                best_rep_index = rep;
                best_rep_len = len;
            }
        }

        let mut main_len = 0;
        let mut main_dist = 0;

        let matches = input.calc_matches();

        if matches.len() > 0 {
            let mut main = *matches.last().unwrap();

            // If the match is long enough, return it
            if main.len >= self.nice_len as u32 {
                return Some(Match {
                    distance: main.distance + REPS as u32, // TODO: Why are we adding here?
                    len: main.len,
                });
            }

            if main.len == MATCH_LEN_MIN as u32 && main.distance >= 0x80 {
                // (not sure why this this specific condition is used, but it's copied from the reference code)
                // I assume this means it's a fairly inefficient match, so we don't want to use it
                main_len = 0;
                main_dist = 0;
            } else {
                main_len = main.len;
                main_dist = main.distance;
            }
        }

        if best_rep_len >= MATCH_LEN_MIN as u32 {
            if best_rep_len + 1 >= main_len
                || (best_rep_len + 2 >= main_len && main_dist >= (1 << 9))
                || (best_rep_len + 3 >= main_len && main_dist >= (1 << 15))
            {
                return Some(Match {
                    distance: best_rep_index as u32,
                    len: best_rep_len,
                });
            }
        }

        if main_len < MATCH_LEN_MIN as u32 {
            return None;
        }

        drop(matches); // Ensure that we don't have a previous borrow of matches when we call calc_matches_at_offset

        // Get the next match. Test if it is better than the current match.
        // If so, encode the current byte as a literal.
        let matches = input.calc_matches_at_offset(1);
        if matches.count > 0 {
            let new_len = matches.len[matches.count as usize - 1];
            let new_dist = matches.dist[matches.count as usize - 1];

            if (new_len >= main_len && new_dist < main_dist)
                || (new_len == main_len + 1 && !change_pair(main_dist as _, new_dist as _))
                || new_len > main_len + 1
                || (new_len + 1 >= main_len
                    && main_len >= MATCH_LEN_MIN as u32 + 1
                    && change_pair(new_dist as _, main_dist as _))
            {
                return 1;
            }
        }

        let limit = (main_len - 1).max(MATCH_LEN_MIN as _);
        for rep in 0..REPS {
            if encoder.lz.get_match_len(encoder.reps[rep], limit as i32) == limit as _ {
                return 1;
            }
        }

        encoder.data.back = (main_dist + REPS as i32) as _;
        encoder.skip((main_len - 2) as _);
        main_len
    }
}
