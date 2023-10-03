mod state;

use std::io::{self, Read};

use crate::compressors::lzma::data_buffers::DecoderDataBuffer;

use self::state::State;

use super::{
    length_codec::LengthCodecDecoder,
    literals_codec::LiteralCodecDecoder,
    range_codec::{RangeDecoder, RangeEncProbability},
};

const POS_STATES_MAX: usize = 1 << 4;
const MATCH_LEN_MIN: usize = 2;

const DIST_STATES: usize = 4;
const DIST_SLOTS: usize = 1 << 6;
const DIST_MODEL_START: usize = 4;
const DIST_MODEL_END: usize = 14;
const FULL_DISTANCES: usize = 1 << (DIST_MODEL_END / 2);

const ALIGN_BITS: usize = 4;
const ALIGN_SIZE: usize = 1 << ALIGN_BITS;
const ALIGN_MASK: usize = ALIGN_SIZE - 1;

const REPS: usize = 4;

const SHIFT_BITS: u32 = 8;
const TOP_MASK: u32 = 0xFF000000;
const BIT_MODEL_TOTAL_BITS: u32 = 11;
const BIT_MODEL_TOTAL: u32 = 1 << BIT_MODEL_TOTAL_BITS;
const PROB_INIT: u16 = (BIT_MODEL_TOTAL / 2) as u16;
const MOVE_BITS: u32 = 5;
const DIST_SPECIAL_INDEX: [usize; 10] = [0, 2, 4, 8, 12, 20, 28, 44, 60, 92];
const DIST_SPECIAL_END: [usize; 10] = [2, 4, 8, 12, 20, 28, 44, 60, 92, 124];

struct LZMACodec {
    pos_mask: u32,
    state: State,

    reps: [u32; REPS],

    is_match_probs: [[RangeEncProbability; POS_STATES_MAX]; state::STATES],
    is_rep_probs: [RangeEncProbability; state::STATES],
    is_rep0_probs: [RangeEncProbability; state::STATES],
    is_rep1_probs: [RangeEncProbability; state::STATES],
    is_rep2_probs: [RangeEncProbability; state::STATES],
    is_rep0_long_probs: [[RangeEncProbability; POS_STATES_MAX]; state::STATES],

    // TODO: Check these constants
    dist_slot_probs: [[RangeEncProbability; DIST_SLOTS]; DIST_STATES],
    dist_special_probs: [RangeEncProbability; 124],
    dist_align_probs: [RangeEncProbability; ALIGN_SIZE],
}

impl LZMACodec {
    pub fn new(pb: u32) -> Self {
        Self {
            pos_mask: (1 << pb) - 1,

            state: State::new(),

            reps: [0; REPS],

            is_match_probs: [[RangeEncProbability::new(); POS_STATES_MAX]; state::STATES],
            is_rep_probs: [RangeEncProbability::new(); state::STATES],
            is_rep0_probs: [RangeEncProbability::new(); state::STATES],
            is_rep1_probs: [RangeEncProbability::new(); state::STATES],
            is_rep2_probs: [RangeEncProbability::new(); state::STATES],
            is_rep0_long_probs: [[RangeEncProbability::new(); POS_STATES_MAX]; state::STATES],

            dist_slot_probs: [[RangeEncProbability::new(); DIST_SLOTS]; DIST_STATES],
            dist_special_probs: [RangeEncProbability::new(); 124],
            dist_align_probs: [RangeEncProbability::new(); ALIGN_SIZE],
        }
    }
}

pub struct LZMACodecDecoder {
    codec: LZMACodec,

    literal_decoder: LiteralCodecDecoder,
    match_len_decoder: LengthCodecDecoder,
    rep_len_decoder: LengthCodecDecoder,
}

impl LZMACodecDecoder {
    pub fn new(lc: u32, lp: u32, pb: u32) -> Self {
        Self {
            codec: LZMACodec::new(pb),

            literal_decoder: LiteralCodecDecoder::new(lc, lp),
            match_len_decoder: LengthCodecDecoder::new(pb),
            rep_len_decoder: LengthCodecDecoder::new(pb),
        }
    }

    pub fn decode_one_packet(
        &mut self,
        rc: &mut RangeDecoder<impl Read>,
        output: &mut DecoderDataBuffer,
    ) -> io::Result<()> {
        let pos_state = output.position() as u32 & self.codec.pos_mask;
        let i = self.codec.state.get() as usize;

        let prob = &mut self.codec.is_match_probs[i][pos_state as usize];
        let bit = rc.decode_bit(prob)?;

        if bit == 0 {
            // TODO: Choose the right literal decoding method
            // self.literal_decoder.decode(&mut self.coder, lz, rc)?;
        } else {
            let index = self.codec.state.get() as usize; // TODO: Do we need to fetch again?
            let prob = &mut self.codec.is_rep_probs[index];

            let len = if rc.decode_bit(prob)? == 0 {
                self.decode_match(pos_state, rc)?
            } else {
                self.decode_rep_match(pos_state, rc)?
            };

            output.append_match(self.codec.reps[0], len);
        }

        rc.normalize()?; // TODO: Do we need this?

        Ok(())
    }

    fn decode_match<R: Read>(
        &mut self,
        pos_state: u32,
        rc: &mut RangeDecoder<R>,
    ) -> io::Result<u32> {
        self.codec.state.update_match();

        self.codec.reps[3] = self.codec.reps[2];
        self.codec.reps[2] = self.codec.reps[1];
        self.codec.reps[1] = self.codec.reps[0];

        let len = self.match_len_decoder.decode(pos_state as _, rc)?;
        let dist_slot = self.match_len_decoder.decode(rc, pos_state);
        let dist_slot = rc.decode_bit_tree(&mut self.dist_slots[coder_get_dict_size(len as _)])?;

        if dist_slot < DIST_MODEL_START as i32 {
            self.reps[0] = dist_slot as _;
        } else {
            let limit = (dist_slot >> 1) - 1;
            self.reps[0] = (2 | (dist_slot & 1)) << limit;
            if dist_slot < DIST_MODEL_END as i32 {
                let probs = self.get_dist_special((dist_slot - DIST_MODEL_START as i32) as usize);
                self.reps[0] |= rc.decode_reverse_bit_tree(probs)?;
            } else {
                let r0 = rc.decode_direct_bits(limit as u32 - ALIGN_BITS as u32)? << ALIGN_BITS;
                self.reps[0] = self.reps[0] | r0;
                self.reps[0] |= rc.decode_reverse_bit_tree(&mut self.dist_align)?;
            }
        }

        Ok(len as _)
    }

    fn decode_rep_match<R: Read>(
        &mut self,
        pos_state: u32,
        rc: &mut RangeDecoder<R>,
    ) -> io::Result<u32> {
        let index = self.state.get() as usize;
        if rc.decode_bit(&mut self.is_rep0[index])? == 0 {
            let index: usize = self.state.get() as usize;
            if rc.decode_bit(&mut self.is_rep0_long[index][pos_state as usize])? == 0 {
                self.state.update_short_rep();
                return Ok(1);
            }
        } else {
            let tmp;
            let s = self.state.get() as usize;
            if rc.decode_bit(&mut self.is_rep1[s])? == 0 {
                tmp = self.reps[1];
            } else {
                if rc.decode_bit(&mut self.is_rep2[s])? == 0 {
                    tmp = self.reps[2];
                } else {
                    tmp = self.reps[3];
                    self.reps[3] = self.reps[2];
                }
                self.reps[2] = self.reps[1];
            }
            self.reps[1] = self.reps[0];
            self.reps[0] = tmp;
        }

        self.state.update_long_rep();
        self.rep_len_decoder
            .decode(pos_state as _, rc)
            .map(|i| i as u32)
    }
}
