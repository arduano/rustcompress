mod state;

use std::io::{self, Read};

use crate::compressors::lzma::{data_buffers::DecoderDataBuffer, match_finding::Match};

use self::state::State;

use super::{
    length_codec::{LengthCodecDecoder, LengthValueCodec},
    literals_codec::LiteralCodecDecoder,
    range_codec::{RangeDecoder, RangeEncProbability},
};

const POS_STATES_MAX: usize = 1 << 4;
const MATCH_LEN_MIN: usize = 2;

const DIST_STATES: usize = 4;
const DIST_SLOTS: usize = 1 << 6;
const DIST_MODEL_START: u32 = 4;
const DIST_MODEL_END: u32 = 14;

const ALIGN_BITS: usize = 4;
const ALIGN_SIZE: usize = 1 << ALIGN_BITS;

const REPS: usize = 4;

fn coder_get_dict_size(len: usize) -> usize {
    if len < DIST_STATES + MATCH_LEN_MIN {
        len - MATCH_LEN_MIN
    } else {
        DIST_STATES - 1
    }
}

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
    dist_slot_probs: [LengthValueCodec<DIST_SLOTS>; DIST_STATES],
    // I'm not sure if doing this the static way or the array way is faster
    // But I didn't know that this would be necessary when I initially implemented the const generic
    // TODO: Test the performance of this. Theoretically, static should be faster as it can do loop unrolling.
    dist_special_probs: (
        LengthValueCodec<2>,
        LengthValueCodec<2>,
        LengthValueCodec<4>,
        LengthValueCodec<4>,
        LengthValueCodec<8>,
        LengthValueCodec<8>,
        LengthValueCodec<16>,
        LengthValueCodec<16>,
        LengthValueCodec<32>,
        LengthValueCodec<32>,
    ),
    dist_align_probs: LengthValueCodec<ALIGN_SIZE>,
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

            dist_slot_probs: [LengthValueCodec::new(); DIST_STATES],
            dist_special_probs: (
                LengthValueCodec::new(),
                LengthValueCodec::new(),
                LengthValueCodec::new(),
                LengthValueCodec::new(),
                LengthValueCodec::new(),
                LengthValueCodec::new(),
                LengthValueCodec::new(),
                LengthValueCodec::new(),
                LengthValueCodec::new(),
                LengthValueCodec::new(),
            ),
            dist_align_probs: LengthValueCodec::new(),
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
            let byte = if self.codec.state.is_literal() {
                self.literal_decoder.decode_normal(
                    rc,
                    output.get_byte(0),
                    output.position() as usize,
                )?
            } else {
                self.literal_decoder.decode_matched(
                    rc,
                    output.get_byte(0),
                    output.position() as usize,
                    output.get_byte(self.codec.reps[0]),
                )?
            };

            output.append_byte(byte);
            self.codec.state.update_literal();
        } else {
            let index = self.codec.state.get() as usize; // TODO: Do we need to fetch again?
            let prob = &mut self.codec.is_rep_probs[index];

            let match_ = if rc.decode_bit(prob)? == 0 {
                self.decode_match(pos_state, rc)?
            } else {
                self.decode_rep_match(pos_state, rc)?
            };

            output.append_match(match_.distance, match_.len);
        }

        rc.normalize()?; // TODO: Do we need this?

        Ok(())
    }

    fn decode_match<R: Read>(
        &mut self,
        pos_state: u32,
        rc: &mut RangeDecoder<R>,
    ) -> io::Result<Match> {
        self.codec.state.update_match();

        self.codec.reps[3] = self.codec.reps[2];
        self.codec.reps[2] = self.codec.reps[1];
        self.codec.reps[1] = self.codec.reps[0];

        let len = self.match_len_decoder.decode(rc, pos_state)?;
        let slot_decoder = &mut self.codec.dist_slot_probs[coder_get_dict_size(len as usize)];
        let dist_slot = slot_decoder.decode_bit_tree(rc)?;

        if dist_slot < DIST_MODEL_START {
            self.codec.reps[0] = dist_slot as _;
        } else {
            let limit = (dist_slot >> 1) - 1;
            let mut rep0 = (2 | (dist_slot & 1)) << limit;

            if  dist_slot < DIST_MODEL_END {
                let dist_slots_index = (dist_slot - DIST_MODEL_START) as usize;
                rep0 |= self.decode_special_dist_slot(rc, dist_slots_index)?;
            } else {
                rep0 |= rc.decode_direct_bits(limit as u32 - ALIGN_BITS as u32)? << ALIGN_BITS;
                rep0 |= self.codec.dist_align_probs.decode_reverse_bit_tree(rc)?;
            }

            self.codec.reps[0] = rep0;
        }

        Ok(Match {
            len,
            distance: self.codec.reps[0],
        })
    }

    fn decode_special_dist_slot(
        &mut self,
        rc: &mut RangeDecoder<impl Read>,
        index: usize,
    ) -> io::Result<u32> {
        let probs = &mut self.codec.dist_special_probs;
        Ok(match index {
            0 => probs.0.decode_reverse_bit_tree(rc)?,
            1 => probs.1.decode_reverse_bit_tree(rc)?,
            2 => probs.2.decode_reverse_bit_tree(rc)?,
            3 => probs.3.decode_reverse_bit_tree(rc)?,
            4 => probs.4.decode_reverse_bit_tree(rc)?,
            5 => probs.5.decode_reverse_bit_tree(rc)?,
            6 => probs.6.decode_reverse_bit_tree(rc)?,
            7 => probs.7.decode_reverse_bit_tree(rc)?,
            8 => probs.8.decode_reverse_bit_tree(rc)?,
            9 => probs.9.decode_reverse_bit_tree(rc)?,
            _ => unreachable!(),
        })
    }

    fn decode_rep_match(
        &mut self,
        pos_state: u32,
        rc: &mut RangeDecoder<impl Read>,
    ) -> io::Result<Match> {
        let index = self.codec.state.get() as usize;

        let prob = &mut self.codec.is_rep0_probs[index];
        if rc.decode_bit(prob)? == 0 {
            let index: usize = self.codec.state.get() as usize; // TODO: Do we need to fetch again?
            let prob = &mut self.codec.is_rep0_long_probs[index][pos_state as usize];
            if rc.decode_bit(prob)? == 0 {
                self.codec.state.update_short_rep();
                return Ok(Match {
                    len: 1,
                    distance: self.codec.reps[0],
                });
            }
        } else {
            let tmp;
            let s = self.codec.state.get() as usize; // TODO: Do we need to fetch again?
            let prob = &mut self.codec.is_rep1_probs[s];
            if rc.decode_bit(prob)? == 0 {
                tmp = self.codec.reps[1];
            } else {
                let prob = &mut self.codec.is_rep2_probs[s];
                if rc.decode_bit(prob)? == 0 {
                    tmp = self.codec.reps[2];
                } else {
                    tmp = self.codec.reps[3];
                    self.codec.reps[3] = self.codec.reps[2];
                }
                self.codec.reps[2] = self.codec.reps[1];
            }
            self.codec.reps[1] = self.codec.reps[0];
            self.codec.reps[0] = tmp;
        }

        self.codec.state.update_long_rep();
        let len = self.rep_len_decoder.decode(rc, pos_state as _)?;

        Ok(Match {
            len,
            distance: self.codec.reps[0],
        })
    }
}
