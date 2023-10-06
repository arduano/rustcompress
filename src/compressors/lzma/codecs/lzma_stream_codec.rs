pub mod data_buffers;
pub mod encoders;
mod state;

use std::io::{self, Read, Write};

use self::{
    data_buffers::DecoderDataBuffer,
    encoders::{
        match_finding::{Match, MatchFinder},
        EncodeInstruction, LZMAEncoderInput, LZMAInstructionPicker,
    },
    state::State,
};

use super::{
    length_codec::{LengthCodecDecoder, LengthCodecEncoder, LengthValueCodec},
    literals_codec::{LiteralCodecDecoder, LiteralCodecEncoder},
    range_codec::{RangeDecoder, RangeEncProbability, RangeEncoder},
};

const POS_STATES_MAX: usize = 1 << 4;
const MATCH_LEN_MIN: usize = 2;

const DIST_STATES: usize = 4;
const DIST_SLOTS: usize = 1 << 6;
const DIST_MODEL_START: u32 = 4;
const DIST_MODEL_END: u32 = 14;

const ALIGN_BITS: usize = 4;
const ALIGN_SIZE: usize = 1 << ALIGN_BITS;
const ALIGN_MASK: usize = ALIGN_SIZE - 1;

const REPS: usize = 4;

fn coder_get_dict_size(len: usize) -> usize {
    if len < DIST_STATES + MATCH_LEN_MIN {
        len - MATCH_LEN_MIN
    } else {
        DIST_STATES - 1
    }
}

fn get_dist_slot(dist: u32) -> u32 {
    if dist <= DIST_MODEL_START as u32 {
        return dist;
    }
    let mut n = dist;
    let mut i = 31;

    if (n & 0xFFFF0000) == 0 {
        n <<= 16;
        i = 15;
    }

    if (n & 0xFF000000) == 0 {
        n <<= 8;
        i -= 8;
    }

    if (n & 0xF0000000) == 0 {
        n <<= 4;
        i -= 4;
    }

    if (n & 0xC0000000) == 0 {
        n <<= 2;
        i -= 2;
    }

    if (n & 0x80000000) == 0 {
        i -= 1;
    }

    return (i << 1) + ((dist >> (i - 1)) & 1);
}

pub struct LZMACodec {
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

    pub fn reps(&self) -> &[u32; REPS] {
        &self.reps
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
        let index = self.codec.state.get() as usize;

        let prob = &mut self.codec.is_match_probs[index][pos_state as usize];
        let bit = rc.decode_bit(prob)?;

        if bit == 0 {
            // TODO: Split this out into a function, same with encode
            let byte = if self.codec.state.is_literal() {
                let last_byte = if output.is_empty() {
                    0
                } else {
                    output.get_byte(0)
                };

                self.literal_decoder
                    .decode_normal(rc, last_byte, output.position() as usize)?
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
            let prob = &mut self.codec.is_rep_probs[index];

            let match_ = if rc.decode_bit(prob)? == 0 {
                self.decode_match(pos_state, rc)?
            } else {
                self.decode_rep_match(pos_state, rc)?
            };

            output.append_match(match_.distance, match_.len);
        }

        Ok(())
    }

    fn decode_match(
        &mut self,
        pos_state: u32,
        rc: &mut RangeDecoder<impl Read>,
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

            if dist_slot < DIST_MODEL_END {
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
            let prob = &mut self.codec.is_rep1_probs[index];
            if rc.decode_bit(prob)? == 0 {
                tmp = self.codec.reps[1];
            } else {
                let prob = &mut self.codec.is_rep2_probs[index];
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

pub struct LZMACodecEncoder<Mode: LZMAInstructionPicker> {
    codec: LZMACodec,

    literal_encoder: LiteralCodecEncoder,
    match_len_encoder: LengthCodecEncoder,
    rep_len_encoder: LengthCodecEncoder,

    dist_price_count: i32,
    align_price_count: i32,

    picker: Mode,
}

impl<Mode: LZMAInstructionPicker> LZMACodecEncoder<Mode> {
    // TODO: Simplify these args by using structs
    pub fn new(lc: u32, lp: u32, pb: u32, nice_len: u32, picker: Mode) -> Self {
        Self {
            codec: LZMACodec::new(pb),

            literal_encoder: LiteralCodecEncoder::new(lc, lp),
            match_len_encoder: LengthCodecEncoder::new(pb, nice_len),
            rep_len_encoder: LengthCodecEncoder::new(pb, nice_len),

            dist_price_count: 0,
            align_price_count: 0,

            picker,
        }
    }

    /// Get the next instruction, progressing the input buffer forwards by the according ammount
    fn get_next_instruction(
        &mut self,
        input: &mut LZMAEncoderInput<impl MatchFinder>,
    ) -> EncodeInstruction {
        let pos = input.pos();

        let instruction = self.picker.get_next_symbol(input, &self.codec);

        let bytes_to_encode = instruction.length();

        let new_pos = pos + bytes_to_encode as u64;
        if input.pos() > new_pos {
            // The instruction picker mismanaged the buffer position
            panic!("input.pos(): {}, new_pos: {}", input.pos(), new_pos);
        } else if input.pos() < new_pos {
            // Progress the buffer ourselves
            input.skip((new_pos - input.pos()) as u32);
        }

        instruction
    }

    pub fn encode_one_packet(
        &mut self,
        rc: &mut RangeEncoder<impl Write>,
        input: &mut LZMAEncoderInput<impl MatchFinder>,
    ) -> io::Result<()> {
        let pos = input.pos() - input.dict_size() as u64;

        let pos_state = pos as u32 & self.codec.pos_mask;
        let state_idx = self.codec.state.get() as usize;

        let instruction = self.get_next_instruction(input);

        dbg!(&instruction);

        let is_match_prob = &mut self.codec.is_match_probs[state_idx][pos_state as usize];

        match instruction {
            EncodeInstruction::Literal => {
                let length = instruction.length() as i32;

                rc.encode_bit(is_match_prob, 0)?;

                let symbol = input.buffer().get_byte(0 - length);
                let prev_byte = input.buffer().get_byte(-1 - length);

                if self.codec.state.is_literal() {
                    self.literal_encoder
                        .encode_normal(rc, symbol, prev_byte, pos as u32)?
                } else {
                    let match_byte = input
                        .buffer()
                        .get_byte(-(self.codec.reps[0] as i32) - 1 - length);
                    self.literal_encoder
                        .encode_matched(rc, symbol, prev_byte, pos as u32, match_byte)?
                }
            }
            EncodeInstruction::Match(match_) => {
                rc.encode_bit(is_match_prob, 1)?;

                let is_rep_prob = &mut self.codec.is_rep_probs[state_idx];
                rc.encode_bit(is_rep_prob, 0)?;

                self.encode_match(rc, match_, pos_state)?;
            }
            EncodeInstruction::Rep { rep_index, len } => {
                rc.encode_bit(is_match_prob, 1)?;

                let is_rep_prob = &mut self.codec.is_rep_probs[state_idx];
                rc.encode_bit(is_rep_prob, 1)?;

                self.encode_rep_match(rc, rep_index as _, len, pos_state)?;
            }
        }

        Ok(())
    }

    fn encode_match(
        &mut self,
        rc: &mut RangeEncoder<impl Write>,
        match_: Match,
        pos_state: u32,
    ) -> io::Result<()> {
        self.codec.state.update_match();
        self.match_len_encoder.encode(rc, match_.len, pos_state)?;
        let dist_slot = get_dist_slot(match_.distance);

        let slot_encoder =
            &mut self.codec.dist_slot_probs[coder_get_dict_size(match_.len as usize)];
        slot_encoder.encode_bit_tree(rc, dist_slot)?;

        if dist_slot >= DIST_MODEL_START {
            let footer_bits = (dist_slot >> 1).wrapping_sub(1);
            let base = (2 | (dist_slot & 1)) << footer_bits;
            let dist_reduced = match_.distance - base;

            if dist_slot < DIST_MODEL_END as u32 {
                self.encode_special_dist_slot(rc, dist_slot - DIST_MODEL_START, dist_reduced)?;
            } else {
                rc.encode_direct_bits(dist_reduced >> ALIGN_BITS, footer_bits - ALIGN_BITS as u32)?;

                self.codec
                    .dist_align_probs
                    .encode_reverse_bit_tree(rc, dist_reduced & ALIGN_MASK as u32)?;

                self.align_price_count = self.align_price_count - 1;
            }
        }

        self.codec.reps[3] = self.codec.reps[2];
        self.codec.reps[2] = self.codec.reps[1];
        self.codec.reps[1] = self.codec.reps[0];
        self.codec.reps[0] = match_.distance;

        self.dist_price_count = self.dist_price_count - 1;

        Ok(())
    }

    fn encode_special_dist_slot(
        &mut self,
        rc: &mut RangeEncoder<impl Write>,
        index: u32,
        symbol: u32,
    ) -> io::Result<()> {
        let probs = &mut self.codec.dist_special_probs;
        Ok(match index {
            0 => probs.0.encode_reverse_bit_tree(rc, symbol)?,
            1 => probs.1.encode_reverse_bit_tree(rc, symbol)?,
            2 => probs.2.encode_reverse_bit_tree(rc, symbol)?,
            3 => probs.3.encode_reverse_bit_tree(rc, symbol)?,
            4 => probs.4.encode_reverse_bit_tree(rc, symbol)?,
            5 => probs.5.encode_reverse_bit_tree(rc, symbol)?,
            6 => probs.6.encode_reverse_bit_tree(rc, symbol)?,
            7 => probs.7.encode_reverse_bit_tree(rc, symbol)?,
            8 => probs.8.encode_reverse_bit_tree(rc, symbol)?,
            9 => probs.9.encode_reverse_bit_tree(rc, symbol)?,
            _ => unreachable!(),
        })
    }

    fn encode_rep_match<W: Write>(
        &mut self,
        rc: &mut RangeEncoder<W>,
        rep: u32,
        len: u32,
        pos_state: u32,
    ) -> std::io::Result<()> {
        let state = self.codec.state.get() as usize;

        if rep == 0 {
            let rep0_prob = &mut self.codec.is_rep0_probs[state];
            rc.encode_bit(rep0_prob, 0)?;

            let rep0_long_prob = &mut self.codec.is_rep0_long_probs[state][pos_state as usize];
            rc.encode_bit(rep0_long_prob, if len == 1 { 0 } else { 1 })?;
        } else {
            let rep0_prob = &mut self.codec.is_rep0_probs[state];
            rc.encode_bit(rep0_prob, 1)?;

            if rep == 1 {
                let rep1_prob = &mut self.codec.is_rep1_probs[state];
                rc.encode_bit(rep1_prob, 0)?;
            } else {
                let rep1_prob = &mut self.codec.is_rep1_probs[state];
                rc.encode_bit(rep1_prob, 1)?;
                let rep2_prob = &mut self.codec.is_rep2_probs[state];
                rc.encode_bit(rep2_prob, rep - 2)?;

                if rep == 3 {
                    self.codec.reps[3] = self.codec.reps[2];
                }
                self.codec.reps[2] = self.codec.reps[1];
            }

            self.codec.reps[1] = self.codec.reps[0];
            self.codec.reps[0] = self.codec.reps[rep as usize];
        }

        if len == 1 {
            self.codec.state.update_short_rep();
        } else {
            self.rep_len_encoder.encode(rc, len, pos_state)?;
            self.codec.state.update_long_rep();
        }
        Ok(())
    }
}
