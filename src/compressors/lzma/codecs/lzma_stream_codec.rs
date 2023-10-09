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
    range_codec::{RangeDecoder, RangeEncPrice, RangeEncProbability, RangeEncoder},
};

// TODO: Clean up all these constants
const POS_STATES_MAX: usize = 1 << 4;
const MATCH_LEN_MIN: usize = 2;

const DIST_STATES: usize = 4;
const DIST_SLOTS: usize = 1 << 6;
const DIST_MODEL_START: u32 = 4;
const DIST_MODEL_END: u32 = 14;
const FULL_DISTANCES: usize = 1 << (DIST_MODEL_END / 2);

const ALIGN_BITS: usize = 4;
const ALIGN_SIZE: usize = 1 << ALIGN_BITS;
const ALIGN_MASK: usize = ALIGN_SIZE - 1;

const DIST_PRICE_UPDATE_INTERVAL: u32 = FULL_DISTANCES as u32;
const ALIGN_PRICE_UPDATE_INTERVAL: u32 = ALIGN_SIZE as u32;

const REPS: usize = 4;

fn get_dist_state(len: u32) -> usize {
    if len < DIST_STATES as u32 + MATCH_LEN_MIN as u32 {
        len as usize - MATCH_LEN_MIN
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

    dist_slot_probs: [LengthValueCodec<DIST_SLOTS>; DIST_STATES],
    // I'm not sure if doing this the static way or the array way is faster
    // But I didn't know that this would be necessary when I initially implemented the const generic
    // TODO: Test the performance of this, as opposed to array indexing and passing around probability array slices.
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

pub struct LZMACodecEncoder<Mode: LZMAInstructionPicker> {
    codec: LZMACodec,
    position: u64,
    dict_size: u32,

    literal_encoder: LiteralCodecEncoder,
    match_len_encoder: LengthCodecEncoder,
    rep_len_encoder: LengthCodecEncoder,

    data: LZMAEncoderData,

    picker: Mode,
}

impl<Mode: LZMAInstructionPicker> LZMACodecEncoder<Mode> {
    // TODO: Simplify these args by using structs
    pub fn new(dict_size: u32, lc: u32, lp: u32, pb: u32, nice_len: u32, picker: Mode) -> Self {
        Self {
            codec: LZMACodec::new(pb),
            position: dict_size as u64,
            dict_size,

            literal_encoder: LiteralCodecEncoder::new(lc, lp),
            match_len_encoder: LengthCodecEncoder::new(pb, nice_len),
            rep_len_encoder: LengthCodecEncoder::new(pb, nice_len),

            data: LZMAEncoderData::new(dict_size),

            picker,
        }
    }

    pub fn position(&self) -> u64 {
        self.position - self.dict_size as u64
    }

    /// Get the next instruction, progressing the input buffer forwards by the according ammount
    fn get_next_instruction(
        &mut self,
        input: &mut LZMAEncoderInput<impl MatchFinder>,
    ) -> EncodeInstruction {
        let mut price_calc = EncoderPriceCalc {
            data: &mut self.data,
            codec: &self.codec,
            literal_encoder: &mut self.literal_encoder,
            match_len_encoder: &mut self.match_len_encoder,
            rep_len_encoder: &mut self.rep_len_encoder,
        };

        let instruction = self
            .picker
            .get_next_symbol(input, &mut price_calc, &self.codec);

        dbg!(instruction);

        let bytes_to_encode = instruction.length();
        self.position += bytes_to_encode as u64;

        // Catch up to the position if necessary
        if input.pos() < self.position {
            // Progress the buffer ourselves
            input.skip((self.position - input.pos()) as u32);
        }

        instruction
    }

    pub fn encode_one_packet(
        &mut self,
        rc: &mut RangeEncoder<impl Write>,
        input: &mut LZMAEncoderInput<impl MatchFinder>,
    ) -> io::Result<u32> {
        let pos = input.pos() - input.dict_size() as u64;

        let pos_state = pos as u32 & self.codec.pos_mask;
        let state_idx = self.codec.state.get() as usize;

        let instruction = self.get_next_instruction(input);

        let is_match_prob = &mut self.codec.is_match_probs[state_idx][pos_state as usize];

        match instruction {
            EncodeInstruction::Literal => {
                rc.encode_bit(is_match_prob, 0)?;

                self.encode_literal(rc, input, pos as u32)?;
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

        Ok(instruction.length())
    }

    fn encode_literal(
        &mut self,
        rc: &mut RangeEncoder<impl Write>,
        input: &mut LZMAEncoderInput<impl MatchFinder>,
        pos: u32,
    ) -> io::Result<()> {
        // The length of the literal is always 1
        let length = 1;

        // If the input has been read ahead, we need to make sure we have the right offset
        let pos_diff = input.pos() - self.position;

        // TODO: Ensure that whenever the input buffer is appeneded to *when input is ahead of this encoder*, it doesn't overwrite the data that we're reading here
        let symbol = input.buffer().get_byte(0 - length - pos_diff as i32);
        let prev_byte = input.buffer().get_byte(-1 - length - pos_diff as i32);

        if self.codec.state.is_literal() {
            self.literal_encoder
                .encode_normal(rc, symbol, prev_byte, pos)?
        } else {
            let match_byte = input
                .buffer()
                .get_byte(-(self.codec.reps[0] as i32) - 1 - length);
            self.literal_encoder
                .encode_matched(rc, symbol, prev_byte, pos, match_byte)?
        }

        self.codec.state.update_literal();

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

        let slot_encoder = &mut self.codec.dist_slot_probs[get_dist_state(match_.len)];
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

                self.data.align_price_count -= 1;
            }
        }

        self.codec.reps[3] = self.codec.reps[2];
        self.codec.reps[2] = self.codec.reps[1];
        self.codec.reps[1] = self.codec.reps[0];
        self.codec.reps[0] = match_.distance;

        self.data.dist_price_count -= 1;

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

        // Store the value as we're swapping array elements around
        let rep_value = self.codec.reps[rep as usize];

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
            self.codec.reps[0] = rep_value;
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

struct LZMAEncoderData {
    dist_price_count: i32,
    align_price_count: i32,
    dist_slot_prices_size: u32,
    dist_slot_prices: Vec<Vec<RangeEncPrice>>, // TODO: Flatten this array
    full_dist_prices: [[RangeEncPrice; FULL_DISTANCES]; DIST_STATES],
    align_prices: [RangeEncPrice; ALIGN_SIZE],
}

impl LZMAEncoderData {
    pub fn new(dict_size: u32) -> Self {
        let dist_slot_prices_size = get_dist_slot(dict_size - 1) + 1;
        let dist_slot_prices =
            vec![vec![RangeEncPrice::zero(); dist_slot_prices_size as usize]; DIST_STATES];

        Self {
            dist_price_count: 0,
            align_price_count: 0,
            dist_slot_prices_size,
            dist_slot_prices,
            full_dist_prices: [[RangeEncPrice::zero(); FULL_DISTANCES]; DIST_STATES],
            align_prices: [RangeEncPrice::zero(); ALIGN_SIZE],
        }
    }
}

pub struct EncoderPriceCalc<'a> {
    data: &'a mut LZMAEncoderData,
    codec: &'a LZMACodec,
    literal_encoder: &'a mut LiteralCodecEncoder,
    match_len_encoder: &'a mut LengthCodecEncoder,
    rep_len_encoder: &'a mut LengthCodecEncoder,
}

impl<'a> EncoderPriceCalc<'a> {
    pub fn update_prices(&mut self) {
        if self.data.dist_price_count <= 0 {
            self.update_dist_prices();
        }

        if self.data.align_price_count <= 0 {
            self.update_align_prices();
        }
        self.match_len_encoder.update_prices();
        self.rep_len_encoder.update_prices();
    }

    pub fn update_dist_prices(&mut self) {
        self.data.dist_price_count = DIST_PRICE_UPDATE_INTERVAL as _;

        for dist_state in 0..DIST_STATES {
            for dist_slot in 0..self.data.dist_slot_prices_size as usize {
                self.data.dist_slot_prices[dist_state][dist_slot] =
                    self.codec.dist_slot_probs[dist_state].get_bit_tree_price(dist_slot as u32);
            }

            for dist_slot in DIST_MODEL_END as u32..self.data.dist_slot_prices_size {
                let count = (dist_slot >> 1) - 1 - ALIGN_BITS as u32;
                self.data.dist_slot_prices[dist_state][dist_slot as usize] +=
                    RangeEncPrice::get_direct_bits_price(count);
            }

            for dist in 0..DIST_MODEL_START {
                self.data.full_dist_prices[dist_state][dist as usize] =
                    self.data.dist_slot_prices[dist_state][dist as usize];
            }
        }

        let mut dist = DIST_MODEL_START;
        for dist_slot in DIST_MODEL_START..DIST_MODEL_END {
            let footer_bits = (dist_slot >> 1) - 1;
            let base = (2 | (dist_slot & 1)) << footer_bits;

            let index = dist_slot - DIST_MODEL_START;
            let limit = match index {
                // TODO: Clean this
                0 => 2,
                1 => 2,
                2 => 4,
                3 => 4,
                4 => 8,
                5 => 8,
                6 => 16,
                7 => 16,
                8 => 32,
                9 => 32,
                _ => unreachable!(),
            };
            for _i in 0..limit {
                let dist_reduced = dist - base;

                // let price = RangeEncoder::get_reverse_bit_tree_price(
                //     self.get_dist_special(dist_slot - DIST_MODEL_START),
                //     dist_reduced as u32,
                // );

                let probs = &self.codec.dist_special_probs;
                let price = match index {
                    0 => probs.0.get_reverse_bit_tree_price(dist_reduced),
                    1 => probs.1.get_reverse_bit_tree_price(dist_reduced),
                    2 => probs.2.get_reverse_bit_tree_price(dist_reduced),
                    3 => probs.3.get_reverse_bit_tree_price(dist_reduced),
                    4 => probs.4.get_reverse_bit_tree_price(dist_reduced),
                    5 => probs.5.get_reverse_bit_tree_price(dist_reduced),
                    6 => probs.6.get_reverse_bit_tree_price(dist_reduced),
                    7 => probs.7.get_reverse_bit_tree_price(dist_reduced),
                    8 => probs.8.get_reverse_bit_tree_price(dist_reduced),
                    9 => probs.9.get_reverse_bit_tree_price(dist_reduced),
                    _ => unreachable!(),
                };

                for dist_state in 0..DIST_STATES {
                    self.data.full_dist_prices[dist_state][dist as usize] =
                        self.data.dist_slot_prices[dist_state][dist_slot as usize] + price;
                }
                dist += 1;
            }
        }

        assert!(dist == FULL_DISTANCES as u32);
    }

    fn update_align_prices(&mut self) {
        self.data.align_price_count = ALIGN_PRICE_UPDATE_INTERVAL as i32;

        for i in 0..ALIGN_SIZE {
            self.data.align_prices[i] = self
                .codec
                .dist_align_probs
                .get_reverse_bit_tree_price(i as u32);
        }
    }

    pub fn get_literal_price(
        &self,
        cur_byte: u8,
        match_byte: u8,
        prev_byte: u8,
        pos: u32,
        state: &State,
    ) -> RangeEncPrice {
        let pos_state = pos & self.codec.pos_mask;
        let prob = &self.codec.is_match_probs[state.get() as usize][pos_state as usize];
        let packet_price = prob.get_bit_price(0);

        let value_price = if state.is_literal() {
            self.literal_encoder
                .get_normal_price(cur_byte, match_byte, prev_byte, pos)
        } else {
            self.literal_encoder
                .get_matched_price(cur_byte, match_byte, prev_byte, pos)
        };

        packet_price + value_price
    }

    pub fn get_rep_len_price(&self, len: u32, pos_state: u32) -> RangeEncPrice {
        self.rep_len_encoder
            .get_price(len as usize, pos_state as usize)
    }

    // TODO: Rename this function to "get_match_packet_price" and any relevant variables that use it
    pub fn get_any_match_price(&self, state: &State, pos_state: u32) -> RangeEncPrice {
        let prob = &self.codec.is_match_probs[state.get() as usize][pos_state as usize];
        prob.get_bit_price(1)
    }

    pub fn get_normal_match_price(
        &self,
        any_match_price: RangeEncPrice,
        state: &State,
    ) -> RangeEncPrice {
        let is_rep_price = &self.codec.is_rep_probs[state.get() as usize];
        any_match_price + is_rep_price.get_bit_price(0)
    }

    pub fn get_any_rep_price(
        &self,
        any_match_price: RangeEncPrice,
        state: &State,
    ) -> RangeEncPrice {
        let is_rep_price = &self.codec.is_rep_probs[state.get() as usize];
        any_match_price + is_rep_price.get_bit_price(1)
    }

    pub fn get_short_rep_price(
        &self,
        any_rep_price: RangeEncPrice,
        state: &State,
        pos_state: u32,
    ) -> RangeEncPrice {
        let is_rep0_price = &self.codec.is_rep0_probs[state.get() as usize];
        let is_rep0_long_price =
            &self.codec.is_rep0_long_probs[state.get() as usize][pos_state as usize];

        any_rep_price + is_rep0_price.get_bit_price(0) + is_rep0_long_price.get_bit_price(0)
    }

    pub fn get_long_rep_price(
        &self,
        any_rep_price: RangeEncPrice,
        rep: u32,
        state: &State,
        pos_state: u32,
    ) -> RangeEncPrice {
        let is_rep0_price = &self.codec.is_rep0_probs[state.get() as usize];
        let is_rep0_long_price =
            &self.codec.is_rep0_long_probs[state.get() as usize][pos_state as usize];
        let is_rep1_price = &self.codec.is_rep1_probs[state.get() as usize];

        let mut price = any_rep_price;

        if rep == 0 {
            price += is_rep0_price.get_bit_price(0) + is_rep0_long_price.get_bit_price(1);
        } else {
            price += is_rep0_price.get_bit_price(1);

            if rep == 1 {
                price += is_rep1_price.get_bit_price(0);
            } else {
                let is_rep2_price = &self.codec.is_rep2_probs[state.get() as usize];
                price += is_rep1_price.get_bit_price(1) + is_rep2_price.get_bit_price(rep - 2);
            }
        }

        price
    }

    pub fn get_long_rep_and_len_price(
        &self,
        rep: u32,
        len: u32,
        state: &State,
        pos_state: u32,
    ) -> RangeEncPrice {
        let any_match_price = self.get_any_match_price(state, pos_state);
        let any_rep_price = self.get_any_rep_price(any_match_price, state);
        let long_rep_price = self.get_long_rep_price(any_rep_price, rep, state, pos_state);
        return long_rep_price + self.rep_len_encoder.get_price(len as _, pos_state as _);
    }

    pub fn get_match_and_len_price(
        &self,
        normal_match_price: RangeEncPrice,
        dist: u32,
        len: u32,
        pos_state: u32,
    ) -> RangeEncPrice {
        let mut price =
            normal_match_price + self.match_len_encoder.get_price(len as _, pos_state as _);
        let dist_state = get_dist_state(len);

        if dist < FULL_DISTANCES as u32 {
            price += self.data.full_dist_prices[dist_state as usize][dist as usize];
        } else {
            // Note that distSlotPrices includes also
            // the price of direct bits.
            let dist_slot = get_dist_slot(dist);
            price += self.data.dist_slot_prices[dist_state as usize][dist_slot as usize]
                + self.data.align_prices[(dist & ALIGN_MASK as u32) as usize];
        }

        return price;
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
            self.decode_literal(rc, output)?;
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

    fn decode_literal(
        &mut self,
        rc: &mut RangeDecoder<impl Read>,
        output: &mut DecoderDataBuffer,
    ) -> io::Result<()> {
        let last_byte = if output.is_empty() {
            0
        } else {
            output.get_byte(0)
        };

        let byte = if self.codec.state.is_literal() {
            self.literal_decoder
                .decode_normal(rc, last_byte, output.position() as usize)?
        } else {
            let prev_match_byte = output.get_byte(self.codec.reps[0]);
            self.literal_decoder.decode_matched(
                rc,
                last_byte,
                output.position() as usize,
                prev_match_byte,
            )?
        };

        output.append_byte(byte);
        self.codec.state.update_literal();

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
        let slot_decoder = &mut self.codec.dist_slot_probs[get_dist_state(len)];
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
