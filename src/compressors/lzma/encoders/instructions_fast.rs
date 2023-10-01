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

/// Returns true if the distance is shorter than 1/128th of the big distance,
/// which
fn is_distance_sufficiently_shorter(small_dist: u32, big_dist: u32) -> bool {
    small_dist < big_dist / 128
}

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
            let len = input.buffer().get_match_length(rep, avail);
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
            let main = *matches.last().unwrap();

            // If the match is long enough, return it
            if main.len >= self.nice_len as u32 {
                return Some(Match {
                    distance: main.distance + REPS as u32, // TODO: Why are we adding here?
                    len: main.len,
                });
            }

            for m in matches[0..matches.len() - 1].iter().rev() {
                // I'm not sure why this is here, but it's copied from the reference code
                if main_len != m.len + 1 {
                    break;
                }

                if !is_distance_sufficiently_shorter(m.distance as u32, main_dist as u32) {
                    break;
                }

                main_len = m.len;
                main_dist = m.distance;
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

        if main_len == 0 {
            return None;
        }

        // Get the next match. Test if it is better than the current match.
        // If so, encode the current byte as a literal.
        input.increment_pos();
        let matches = input.calc_matches();
        if matches.len() > 0 {
            let next_match = matches.last().unwrap();

            // TODO: Break up this monstrosity, and figure out what it's doing
            // I copied it from the reference code, but I don't understand it
            if (next_match.len >= main_len && next_match.distance < main_dist)
                || (next_match.len == main_len + 1
                    && !is_distance_sufficiently_shorter(main_dist as _, next_match.distance as _))
                || next_match.len > main_len + 1
                || (next_match.len + 1 >= main_len
                    && main_len >= MATCH_LEN_MIN as u32 + 1
                    && is_distance_sufficiently_shorter(next_match.distance as _, main_dist as _))
            {
                return None;
            }
        }

        let limit = (main_len - 1).max(MATCH_LEN_MIN as _);
        for rep in state.reps {
            // TODO: We are calling get_match_length twice on the same values, can we cache them above?
            // There's only a constant amount of them.
            if input.buffer().get_match_length(rep, limit) == limit {
                return None;
            }
        }

        return Some(Match {
            distance: main_dist + REPS as u32, // TODO: Why are we adding here?
            len: main_len,
        });
    }
}
