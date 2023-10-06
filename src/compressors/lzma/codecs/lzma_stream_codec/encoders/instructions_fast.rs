use crate::compressors::lzma::codecs::{
    length_codec::{MATCH_LEN_MAX, MATCH_LEN_MIN},
    lzma_stream_codec::LZMACodec,
};

use super::{
    match_finding::{Match, MatchFinder},
    EncodeInstruction, LZMAEncoderInput, LZMAInstructionPicker,
};

pub struct LZMAFastInstructionPicker {
    nice_len: u32,
}

impl LZMAFastInstructionPicker {
    pub fn new(nice_len: u32) -> Self {
        Self { nice_len }
    }
}

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
        state: &LZMACodec,
    ) -> EncodeInstruction {
        let avail = input.buffer().forwards_bytes().min(MATCH_LEN_MAX);

        // If there aren't enough bytes to encode a match, just return None
        if avail < MATCH_LEN_MIN {
            return EncodeInstruction::Literal;
        }

        // Cache the lengths as they're used multiple times
        let rep_lens = state
            .reps
            .map(|rep| input.buffer().get_match_length(rep, avail as u32));

        let mut best_rep_len = 0;
        let mut best_rep_index = 0;
        for i in 0..state.reps.len() {
            let len = rep_lens[i];
            if len < MATCH_LEN_MIN as u32 {
                continue;
            }

            if len >= self.nice_len {
                return EncodeInstruction::Rep {
                    rep_index: i,
                    len: len,
                };
            }

            if len > best_rep_len {
                best_rep_index = i;
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
                return EncodeInstruction::Match(main);
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
                return EncodeInstruction::Rep {
                    rep_index: best_rep_index,
                    len: best_rep_len,
                };
            }
        }

        if main_len == 0 {
            return EncodeInstruction::Literal;
        }

        // Get the next match. Test if it is better than the current match.
        // If so, encode the current byte as a literal.
        input.increment_pos();
        let matches = input.calc_matches();
        if matches.len() > 0 {
            let next_match = matches.last().unwrap();

            // TODO: Break up this monstrosity, and figure out what it's doing
            // I copied it from the reference LZMA C code, but I don't understand it
            if (next_match.len >= main_len && next_match.distance < main_dist)
                || (next_match.len == main_len + 1
                    && !is_distance_sufficiently_shorter(main_dist as _, next_match.distance as _))
                || next_match.len > main_len + 1
                || (next_match.len + 1 >= main_len
                    && main_len >= MATCH_LEN_MIN as u32 + 1
                    && is_distance_sufficiently_shorter(next_match.distance as _, main_dist as _))
            {
                return EncodeInstruction::Literal;
            }
        }

        // TODO: Reference this file:
        // https://github.com/chemfiles/lzma/blob/chemfiles/liblzma/lzma/lzma_encoder_optimum_fast.c
        // to make sure the algorithm actually matches. There's some confusing conditions here.
        let limit = (main_len - 1).max(MATCH_LEN_MIN as _);
        if rep_lens.into_iter().any(|r| r >= limit) {
            return EncodeInstruction::Literal;
        }

        return EncodeInstruction::Match(Match {
            len: main_len,
            distance: main_dist,
        });
    }
}
