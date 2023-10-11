use crate::compressors::lzma::codecs::{
    length_codec::LengthCodecEncoder,
    literals_codec::LiteralCodecEncoder,
    lzma_stream_codec::{
        ALIGN_BITS, DIST_MODEL_END, DIST_MODEL_START, DIST_PRICE_UPDATE_INTERVAL, DIST_STATES,
        FULL_DISTANCES,
    },
    range_codec::RangeEncPrice,
};

use super::{
    get_dist_slot, get_dist_state, state::State, LZMACodec, LZMAEncoderData, ALIGN_MASK,
    ALIGN_PRICE_UPDATE_INTERVAL, ALIGN_SIZE,
};

pub struct EncoderPriceCalc<'a> {
    pub(super) data: &'a mut LZMAEncoderData,
    pub(super) codec: &'a LZMACodec,
    pub(super) literal_encoder: &'a mut LiteralCodecEncoder,
    pub(super) match_len_encoder: &'a mut LengthCodecEncoder,
    pub(super) rep_len_encoder: &'a mut LengthCodecEncoder,
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
        let prob = &self.codec.is_match_probs[state.get_idx() as usize][pos_state as usize];
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
    pub fn get_any_match_price(&self, state: &State, pos_state: u32) -> AnyMatchPrice {
        let prob = &self.codec.is_match_probs[state.get_idx() as usize][pos_state as usize];
        AnyMatchPrice {
            price_calc: self,
            any_match_price: prob.get_bit_price(1),
            state_idx: state.get_idx() as usize,
            pos_state: pos_state as usize,
        }
    }

    // pub fn get_normal_match_price(
    //     &self,
    //     any_match_price: RangeEncPrice,
    //     state: &State,
    // ) -> RangeEncPrice {
    //     let is_rep_price = &self.codec.is_rep_probs[state.get_idx() as usize];
    //     any_match_price + is_rep_price.get_bit_price(0)
    // }

    // pub fn get_any_rep_price(
    //     &self,
    //     any_match_price: RangeEncPrice,
    //     state: &State,
    // ) -> RangeEncPrice {
    //     let is_rep_price = &self.codec.is_rep_probs[state.get_idx() as usize];
    //     any_match_price + is_rep_price.get_bit_price(1)
    // }

    // pub fn get_short_rep_price(
    //     &self,
    //     any_rep_price: RangeEncPrice,
    //     state: &State,
    //     pos_state: u32,
    // ) -> RangeEncPrice {
    //     let is_rep0_price = &self.codec.is_rep0_probs[state.get_idx() as usize];
    //     let is_rep0_long_price =
    //         &self.codec.is_rep0_long_probs[state.get_idx() as usize][pos_state as usize];

    //     any_rep_price + is_rep0_price.get_bit_price(0) + is_rep0_long_price.get_bit_price(0)
    // }

    // pub fn get_long_rep_price(
    //     &self,
    //     any_rep_price: RangeEncPrice,
    //     rep: u32,
    //     state: &State,
    //     pos_state: u32,
    // ) -> RangeEncPrice {
    //     let is_rep0_price = &self.codec.is_rep0_probs[state.get_idx() as usize];
    //     let is_rep0_long_price =
    //         &self.codec.is_rep0_long_probs[state.get_idx() as usize][pos_state as usize];
    //     let is_rep1_price = &self.codec.is_rep1_probs[state.get_idx() as usize];

    //     let mut price = any_rep_price;

    //     if rep == 0 {
    //         price += is_rep0_price.get_bit_price(0) + is_rep0_long_price.get_bit_price(1);
    //     } else {
    //         price += is_rep0_price.get_bit_price(1);

    //         if rep == 1 {
    //             price += is_rep1_price.get_bit_price(0);
    //         } else {
    //             let is_rep2_price = &self.codec.is_rep2_probs[state.get_idx() as usize];
    //             price += is_rep1_price.get_bit_price(1) + is_rep2_price.get_bit_price(rep - 2);
    //         }
    //     }

    //     price
    // }

    // pub fn get_long_rep_and_len_price(
    //     &self,
    //     rep: u32,
    //     len: u32,
    //     state: &State,
    //     pos_state: u32,
    // ) -> RangeEncPrice {
    //     let any_match_price = self.get_any_match_price(state, pos_state);
    //     let any_rep_price = self.get_any_rep_price(any_match_price, state);
    //     let long_rep_price = self.get_long_rep_price(any_rep_price, rep, state, pos_state);
    //     return long_rep_price + self.rep_len_encoder.get_price(len as _, pos_state as _);
    // }

    // pub fn get_match_and_len_price(
    //     &self,
    //     dist: u32,
    //     len: u32,
    //     state: &State,
    //     pos_state: u32,
    // ) -> RangeEncPrice {
    //     let any_match_price = self.get_any_match_price(state, pos_state);
    //     let normal_match_price = self.get_normal_match_price(any_match_price, state);

    //     let mut price =
    //         normal_match_price + self.match_len_encoder.get_price(len as _, pos_state as _);
    //     let dist_state = get_dist_state(len);

    //     if dist < FULL_DISTANCES as u32 {
    //         price += self.data.full_dist_prices[dist_state as usize][dist as usize];
    //     } else {
    //         // Note that distSlotPrices includes also
    //         // the price of direct bits.
    //         let dist_slot = get_dist_slot(dist);
    //         price += self.data.dist_slot_prices[dist_state as usize][dist_slot as usize]
    //             + self.data.align_prices[(dist & ALIGN_MASK as u32) as usize];
    //     }

    //     return price;
    // }
}

#[derive(Copy, Clone)]
pub struct AnyMatchPrice<'a> {
    price_calc: &'a EncoderPriceCalc<'a>,
    any_match_price: RangeEncPrice,
    state_idx: usize,
    pos_state: usize,
}

impl<'a> AnyMatchPrice<'a> {
    pub fn get_normal_match_price(self) -> NormalMatchPrice<'a> {
        let is_rep_price = &self.price_calc.codec.is_rep_probs[self.state_idx];
        NormalMatchPrice {
            price_calc: self.price_calc,
            normal_match_price: self.any_match_price + is_rep_price.get_bit_price(0),
            pos_state: self.pos_state,
        }
    }

    pub fn get_any_rep_price(self) -> AnyRepPrice<'a> {
        let is_rep_price = &self.price_calc.codec.is_rep_probs[self.state_idx];
        AnyRepPrice {
            price_calc: self.price_calc,
            any_rep_price: self.any_match_price + is_rep_price.get_bit_price(1),
            state_idx: self.state_idx,
            pos_state: self.pos_state,
        }
    }
}

#[derive(Copy, Clone)]
pub struct NormalMatchPrice<'a> {
    price_calc: &'a EncoderPriceCalc<'a>,
    normal_match_price: RangeEncPrice,
    pos_state: usize,
}

impl<'a> NormalMatchPrice<'a> {
    pub fn get_price_with_dist_len(self, dist: u32, len: u32) -> RangeEncPrice {
        let mut price = self.normal_match_price
            + self
                .price_calc
                .match_len_encoder
                .get_price(len as _, self.pos_state as _);
        let dist_state = get_dist_state(len);

        if dist < FULL_DISTANCES as u32 {
            price += self.price_calc.data.full_dist_prices[dist_state as usize][dist as usize];
        } else {
            // Note that distSlotPrices includes also
            // the price of direct bits.
            let dist_slot = get_dist_slot(dist);
            price += self.price_calc.data.dist_slot_prices[dist_state as usize][dist_slot as usize]
                + self.price_calc.data.align_prices[(dist & ALIGN_MASK as u32) as usize];
        }

        return price;
    }
}

#[derive(Copy, Clone)]
pub struct AnyRepPrice<'a> {
    price_calc: &'a EncoderPriceCalc<'a>,
    any_rep_price: RangeEncPrice,
    state_idx: usize,
    pos_state: usize,
}

impl<'a> AnyRepPrice<'a> {
    pub fn get_short_rep_price(self) -> RangeEncPrice {
        let is_rep0_price = &self.price_calc.codec.is_rep0_probs[self.state_idx];
        let is_rep0_long_price =
            &self.price_calc.codec.is_rep0_long_probs[self.state_idx][self.pos_state as usize];

        self.any_rep_price + is_rep0_price.get_bit_price(0) + is_rep0_long_price.get_bit_price(0)
    }

    pub fn get_long_rep_price(self, rep: u32) -> LongRepPrice<'a> {
        let is_rep0_price = &self.price_calc.codec.is_rep0_probs[self.state_idx];
        let is_rep0_long_price =
            &self.price_calc.codec.is_rep0_long_probs[self.state_idx][self.pos_state];
        let is_rep1_price = &self.price_calc.codec.is_rep1_probs[self.state_idx];

        let mut price = self.any_rep_price;

        if rep == 0 {
            price += is_rep0_price.get_bit_price(0) + is_rep0_long_price.get_bit_price(1);
        } else {
            price += is_rep0_price.get_bit_price(1);

            if rep == 1 {
                price += is_rep1_price.get_bit_price(0);
            } else {
                let is_rep2_price = &self.price_calc.codec.is_rep2_probs[self.state_idx];
                price += is_rep1_price.get_bit_price(1) + is_rep2_price.get_bit_price(rep - 2);
            }
        }

        LongRepPrice {
            price_calc: self.price_calc,
            long_rep_price: price,
            pos_state: self.pos_state,
        }
    }
}

#[derive(Copy, Clone)]
pub struct LongRepPrice<'a> {
    price_calc: &'a EncoderPriceCalc<'a>,
    long_rep_price: RangeEncPrice,
    pos_state: usize,
}

impl<'a> LongRepPrice<'a> {
    pub fn get_price_with_len(&self, len: u32) -> RangeEncPrice {
        self.long_rep_price
            + self
                .price_calc
                .rep_len_encoder
                .get_price(len as _, self.pos_state)
    }
}
