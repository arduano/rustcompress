use crate::compressors::lzma::codecs::{
    length_codec::{MATCH_LEN_MAX, MATCH_LEN_MIN},
    lzma_stream_codec::{state::State, EncoderPriceCalc, LZMACodec},
    range_codec::RangeEncPrice,
};

use super::{
    match_finding::{Match, MatchFinder},
    EncodeInstruction, LZMAEncoderInput, LZMAInstructionPicker,
};

pub struct LZMANormalInstructionPicker {
    nice_len: u32,
    pos_mask: u32,
}

impl LZMANormalInstructionPicker {
    const OPTS: u32 = 4096;

    pub fn new(nice_len: u32, pb: u32) -> Self {
        Self {
            nice_len,
            pos_mask: (1 << pb) - 1,
        }
    }
}

impl LZMAInstructionPicker for LZMANormalInstructionPicker {
    fn get_next_symbol(
        &mut self,
        input: &mut LZMAEncoderInput<impl MatchFinder>,
        price_calc: &mut EncoderPriceCalc,
        state: &LZMACodec,
    ) -> EncodeInstruction {
        todo!()
    }
}

const REPS: usize = 4;

#[derive(Debug, Default, Clone)]
struct Optimum {
    state: State,
    reps: [u32; REPS],

    price: RangeEncPrice,
    opt_prev: usize,
    back_prev: i32,
    prev1_is_literal: bool,

    has_prev2: bool,
    opt_prev2: usize,
    back_prev2: i32,
}

impl Optimum {
    fn reset(&mut self) {
        // TODO: Refactor this to be more atomic
        self.price = RangeEncPrice::infinity();
    }

    fn set1(&mut self, new_price: RangeEncPrice, opt_cur: usize, back: i32) {
        self.price = new_price;
        self.opt_prev = opt_cur;
        self.back_prev = back;
        self.prev1_is_literal = false;
    }

    fn set2(&mut self, new_price: RangeEncPrice, opt_cur: usize, back: i32) {
        self.price = new_price;
        self.opt_prev = opt_cur + 1;
        self.back_prev = back;
        self.prev1_is_literal = true;
        self.has_prev2 = false;
    }

    fn set3(
        &mut self,
        new_price: RangeEncPrice,
        opt_cur: usize,
        back2: i32,
        len2: usize,
        back: i32,
    ) {
        self.price = new_price;
        self.opt_prev = opt_cur + len2 + 1;
        self.back_prev = back;
        self.prev1_is_literal = true;
        self.has_prev2 = true;
        self.opt_prev2 = opt_cur;
        self.back_prev2 = back2;
    }
}
