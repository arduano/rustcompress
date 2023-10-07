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
    opts: Vec<Optimum>,
    opt_cur: usize,
    opt_end: usize,
    pos_mask: u32,

    matches_cache: Vec<Match>,
}

impl LZMANormalInstructionPicker {
    const OPTS: u32 = 4096;
    pub const EXTRA_SIZE_BEFORE: u32 = Self::OPTS;
    pub const EXTRA_SIZE_AFTER: u32 = Self::OPTS;

    pub fn new(nice_len: u32, pb: u32) -> Self {
        Self {
            nice_len,
            opts: vec![Optimum::default(); Self::OPTS as usize],
            opt_cur: 0,
            opt_end: 0,
            pos_mask: (1 << pb) - 1,
            matches_cache: Vec::new(),
        }
    }

    fn convert_opts(&mut self, back: &mut i32) -> usize {
        self.opt_end = self.opt_cur;

        let mut opt_prev = self.opts[self.opt_cur].opt_prev;

        loop {
            let opt_index = self.opt_cur;

            if self.opts[opt_index].prev1_is_literal {
                self.opts[opt_prev].opt_prev = self.opt_cur;
                self.opts[opt_prev].back_prev = -1;
                self.opt_cur = opt_prev;
                opt_prev -= 1;

                if self.opts[opt_index].has_prev2 {
                    self.opts[opt_prev].opt_prev = opt_prev + 1;
                    self.opts[opt_prev].back_prev = self.opts[opt_index].back_prev2;
                    self.opt_cur = opt_prev;
                    opt_prev = self.opts[opt_index].opt_prev2;
                }
            }

            let temp = self.opts[opt_prev].opt_prev;
            self.opts[opt_prev].opt_prev = self.opt_cur;
            self.opt_cur = opt_prev;
            opt_prev = temp;
            if self.opt_cur <= 0 {
                break;
            }
        }

        self.opt_cur = self.opts[0].opt_prev;
        *back = self.opts[self.opt_cur].back_prev;
        return self.opt_cur;
    }

    fn update_opt_state_and_reps(&mut self) {
        let mut opt_prev = self.opts[self.opt_cur].opt_prev;
        assert!(opt_prev < self.opt_cur);

        if self.opts[self.opt_cur].prev1_is_literal {
            opt_prev -= 1;

            if self.opts[self.opt_cur].has_prev2 {
                let state = self.opts[self.opts[self.opt_cur].opt_prev2].state;
                self.opts[self.opt_cur].state.set(state);
                if self.opts[self.opt_cur].back_prev2 < REPS as i32 {
                    self.opts[self.opt_cur].state.update_long_rep();
                } else {
                    self.opts[self.opt_cur].state.update_match();
                }
            } else {
                let state = self.opts[opt_prev].state;
                self.opts[self.opt_cur].state.set(state);
            }

            self.opts[self.opt_cur].state.update_literal();
        } else {
            let state = self.opts[opt_prev].state;
            self.opts[self.opt_cur].state.set(state);
        }

        if opt_prev == self.opt_cur - 1 {
            // Must be either a short rep or a literal.
            assert!(
                self.opts[self.opt_cur].back_prev == 0 || self.opts[self.opt_cur].back_prev == -1
            );

            if self.opts[self.opt_cur].back_prev == 0 {
                self.opts[self.opt_cur].state.update_short_rep();
            } else {
                self.opts[self.opt_cur].state.update_literal();
            }

            unsafe {
                std::ptr::copy_nonoverlapping(
                    self.opts[opt_prev].reps.as_ptr(),
                    self.opts[self.opt_cur].reps.as_mut_ptr(),
                    REPS,
                );
            }
        } else {
            let back;
            if self.opts[self.opt_cur].prev1_is_literal && self.opts[self.opt_cur].has_prev2 {
                opt_prev = self.opts[self.opt_cur].opt_prev2;
                back = self.opts[self.opt_cur].back_prev2;
                self.opts[self.opt_cur].state.update_long_rep();
            } else {
                back = self.opts[self.opt_cur].back_prev;
                if back < REPS as i32 {
                    self.opts[self.opt_cur].state.update_long_rep();
                } else {
                    self.opts[self.opt_cur].state.update_match();
                }
            }

            if back < REPS as i32 {
                self.opts[self.opt_cur].reps[0] = self.opts[opt_prev].reps[back as usize];

                for rep in 1..=back as usize {
                    self.opts[self.opt_cur].reps[rep] = self.opts[opt_prev].reps[rep - 1];
                }
                for rep in (back as usize + 1)..REPS {
                    self.opts[self.opt_cur].reps[rep] = self.opts[opt_prev].reps[rep];
                }
            } else {
                self.opts[self.opt_cur].reps[0] = back as u32 - REPS as u32;
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        self.opts[opt_prev].reps.as_ptr(),
                        self.opts[self.opt_cur].reps[1..].as_mut_ptr(),
                        REPS - 1,
                    );
                }
            }
        }
    }

    fn calc1_byte_prices(
        &mut self,
        input: &mut LZMAEncoderInput<impl MatchFinder>,
        price_calc: &EncoderPriceCalc,
        pos: u32,
        pos_state: u32,
        avail: i32,
        any_rep_price: RangeEncPrice,
    ) {
        // This will be set to true if using a literal or a short rep.
        let mut next_is_byte = false;
        let cur_byte = input.buffer().get_byte(0);
        let match_byte = input
            .buffer()
            .get_byte(-(self.opts[self.opt_cur].reps[0] as i32) - 1);

        // Try a literal.
        let literal_price = self.opts[self.opt_cur].price
            + price_calc.get_literal_price(
                cur_byte as _,
                match_byte as _,
                input.buffer().get_byte(-1),
                pos,
                &self.opts[self.opt_cur].state,
            );
        if literal_price < self.opts[self.opt_cur + 1].price {
            self.opts[self.opt_cur + 1].set1(literal_price, self.opt_cur, -1);
            next_is_byte = true;
        }
        let mut next_state = State::new();
        // Try a short rep.
        if match_byte == cur_byte
            && (self.opts[self.opt_cur + 1].opt_prev == self.opt_cur
                || self.opts[self.opt_cur + 1].back_prev != 0)
        {
            let short_rep_price = price_calc.get_short_rep_price(
                any_rep_price,
                &self.opts[self.opt_cur].state,
                pos_state,
            );
            if short_rep_price <= self.opts[self.opt_cur + 1].price {
                self.opts[self.opt_cur + 1].set1(short_rep_price, self.opt_cur, 0);
                next_is_byte = true;
            }
        }

        // If neither a literal nor a short rep was the cheapest choice,
        // try literal + long rep0.
        if !next_is_byte && match_byte != cur_byte && avail > MATCH_LEN_MIN as i32 {
            let len_limit = (self.nice_len as i32).min(avail - 1);
            // TODO: remove i32?
            let len = input.buffer().get_match_length(
                1,
                self.opts[self.opt_cur].reps[0] as u32,
                len_limit as u32,
            );

            if len >= MATCH_LEN_MIN as u32 {
                next_state.set(self.opts[self.opt_cur].state);
                next_state.update_literal();
                let next_pos_state = (pos + 1) & self.pos_mask;
                let price = literal_price
                    + price_calc.get_long_rep_and_len_price(0, len, &next_state, next_pos_state);

                let i = self.opt_cur + 1 + len as usize;
                while self.opt_end < i {
                    self.opt_end += 1;
                    self.opts[self.opt_end].reset();
                }
                if price < self.opts[i].price {
                    self.opts[i].set2(price, self.opt_cur, 0);
                }
            }
        }
    }

    fn calc_long_rep_prices(
        &mut self,
        input: &mut LZMAEncoderInput<impl MatchFinder>,
        price_calc: &EncoderPriceCalc,
        pos: u32,
        pos_state: u32,
        avail: i32,
        any_rep_price: RangeEncPrice,
    ) -> usize {
        let mut start_len = MATCH_LEN_MIN;
        let len_limit = avail.min(self.nice_len as i32);
        let mut next_state = State::new();

        for rep in 0..REPS {
            let len = input.buffer().get_match_length(
                0,
                self.opts[self.opt_cur].reps[rep] as u32,
                len_limit as u32,
            );
            if len < MATCH_LEN_MIN as u32 {
                continue;
            }
            while self.opt_end < self.opt_cur + len as usize {
                self.opt_end += 1;
                self.opts[self.opt_end].reset();
            }
            let long_rep_price = price_calc.get_long_rep_price(
                any_rep_price,
                rep as u32,
                &self.opts[self.opt_cur].state,
                pos_state,
            );

            // i=len;i>=MATCH_LEN_MIN;--i
            for i in (MATCH_LEN_MIN..=len as usize).rev() {
                let price = long_rep_price + price_calc.get_rep_len_price(i as u32, pos_state);
                if price < self.opts[self.opt_cur + i].price {
                    self.opts[self.opt_cur + i].set1(price, self.opt_cur, rep as i32);
                }
            }

            if rep == 0 {
                start_len = len as usize + 1;
            }
            let len2_limit = i32::min(self.nice_len as i32, avail - len as i32 - 1);
            // assert!(
            //     len2_limit >= 0,
            //     "len2_limit>=0, len2_limit={}, avail={}, len={}",
            //     len2_limit,
            //     avail,
            //     len
            // );
            let len2 = input.buffer().get_match_length(
                len + 1,
                self.opts[self.opt_cur].reps[rep] as u32,
                len2_limit as u32,
            );

            if len2 >= MATCH_LEN_MIN as u32 {
                // Rep
                let mut price = long_rep_price + price_calc.get_rep_len_price(len, pos_state as _);
                next_state.set(self.opts[self.opt_cur].state);
                next_state.update_long_rep();

                // Literal
                let cur_byte = input.buffer().get_byte(len as i32);
                let match_byte = input.buffer().get_byte(0);
                let prev_byte = input.buffer().get_byte(len as i32 - 1);
                price += price_calc.get_literal_price(
                    cur_byte,
                    match_byte,
                    prev_byte,
                    pos + len as u32,
                    &next_state,
                );
                next_state.update_literal();

                // Rep0
                let next_pos_state = (pos + len as u32 + 1) & self.pos_mask;
                price +=
                    price_calc.get_long_rep_and_len_price(0, len2, &next_state, next_pos_state);

                let i = self.opt_cur + len as usize + 1 + len2 as usize;
                while self.opt_end < i {
                    self.opt_end += 1;
                    self.opts[self.opt_end].reset();
                }
                if price < self.opts[i].price {
                    self.opts[i].set3(price, self.opt_cur, rep as _, len as usize, 0);
                }
            }
        }

        return start_len;
    }

    fn calc_normal_match_prices(
        &mut self,
        input: &mut LZMAEncoderInput<impl MatchFinder>,
        price_calc: &EncoderPriceCalc,
        pos: u32,
        pos_state: u32,
        avail: i32,
        any_match_price: RangeEncPrice,
        start_len: u32,
    ) {
        let matches = input.calc_matches();
        self.matches_cache.clear();
        self.matches_cache.extend_from_slice(&matches);

        let mut matches = &mut self.matches_cache[..];

        {
            // If the longest match is so long that it would not fit into
            // the opts array, shorten the matches.

            let last = matches.last().unwrap(); // TODO: What if there are 0 matches?
                                                // TODO: Is last.len ever bigger than avail, or does the match finder handle that?

            if last.len as i32 > avail {
                let mut i = 0;
                while (matches[i].len as i32) < avail {
                    i += 1;
                }
                matches[i].len = avail as u32;
                matches = &mut matches[..=i];
            }

            let last = matches.last().unwrap();
            if last.len < start_len {
                return;
            }
            while self.opt_end < self.opt_cur + last.len as usize {
                self.opt_end += 1;
                self.opts[self.opt_end].reset();
            }
        }
        let normal_match_price =
            price_calc.get_normal_match_price(any_match_price, &self.opts[self.opt_cur].state);

        let mut match_ = 0;
        while start_len > matches[match_].len {
            match_ += 1;
        }
        let mut len = start_len;
        let mut next_state = State::new();
        loop {
            let dist = matches[match_].distance;

            // Calculate the price of a match of len bytes from the nearest
            // possible distance.
            let match_and_len_price =
                price_calc.get_match_and_len_price(normal_match_price, dist as _, len, pos_state);
            if match_and_len_price < self.opts[self.opt_cur + len as usize].price {
                self.opts[self.opt_cur + len as usize].set1(
                    match_and_len_price,
                    self.opt_cur,
                    dist as i32 + REPS as i32,
                );
            }
            if len != matches[match_].len {
                len += 1;
                continue;
            }

            // Try match + literal + rep0. First get the length of the rep0.
            let len2_limit = i32::min(self.nice_len as i32, avail - len as i32 - 1);
            let len2 = input
                .buffer()
                .get_match_length(len + 1, dist, len2_limit as u32);

            if len2 >= MATCH_LEN_MIN as _ {
                next_state.set(self.opts[self.opt_cur].state);
                next_state.update_match();

                // Literal
                let cur_byte = input.buffer().get_byte(len as i32);
                let match_byte = input.buffer().get_byte(0);
                let prev_byte = input.buffer().get_byte(len as i32 - 1);
                let mut price = match_and_len_price
                    + price_calc.get_literal_price(
                        cur_byte,
                        match_byte,
                        prev_byte,
                        pos + len,
                        &next_state,
                    );
                next_state.update_literal();

                // Rep0
                let next_pos_state = (pos + len + 1) & self.pos_mask;
                price +=
                    price_calc.get_long_rep_and_len_price(0, len2, &next_state, next_pos_state);

                let i = self.opt_cur + len as usize + 1 + len2 as usize;
                while self.opt_end < i {
                    self.opt_end += 1;
                    self.opts[self.opt_end].reset();
                }
                if price < self.opts[i].price {
                    self.opts[i].set3(
                        price,
                        self.opt_cur,
                        dist as i32 + REPS as i32,
                        len as usize,
                        0,
                    );
                }
            }

            match_ += 1;
            if match_ == matches.len() as usize {
                break;
            }
            len += 1;
        }
    }
}

/// Returns true if the distance is shorter than 1/128th of the big distance,
/// which
fn is_distance_sufficiently_shorter(small_dist: u32, big_dist: u32) -> bool {
    small_dist < big_dist / 128
}

impl LZMAInstructionPicker for LZMANormalInstructionPicker {
    // TODO: Ensure that when this function returns, input gets skipped by the match length - 1
    fn get_next_symbol(
        &mut self,
        input: &mut LZMAEncoderInput<impl MatchFinder>,
        price_calc: &mut EncoderPriceCalc,
        state: &LZMACodec,
    ) -> EncodeInstruction {
        let mut back = 0;

        // If there are pending symbols from an earlier call to this
        // function, return those symbols first.
        let pos = input.pos();
        assert!(pos >= 0);
        if self.opt_cur < self.opt_end {
            let len = self.opts[self.opt_cur].opt_prev as i32 - self.opt_cur as i32;
            self.opt_cur = self.opts[self.opt_cur].opt_prev;
            back = self.opts[self.opt_cur].back_prev;
            assert!(len >= 0);

            if back < REPS as i32 {
                return EncodeInstruction::Rep {
                    rep_index: back as usize,
                    len: len as u32,
                };
            } else {
                return EncodeInstruction::Match(Match {
                    distance: back as u32 - REPS as u32,
                    len: len as u32,
                });
            }
        }

        assert_eq!(self.opt_cur, self.opt_end);
        self.opt_cur = 0;
        self.opt_end = 0;
        back = -1;

        // Get the number of bytes available in the dictionary, but
        // not more than the maximum match length. If there aren't
        // enough bytes remaining to encode a match at all, return
        // immediately to encode this byte as a literal.
        let mut avail = i32::min(input.forward_bytes() as i32, MATCH_LEN_MAX as i32);
        if avail < MATCH_LEN_MIN as i32 {
            return EncodeInstruction::Literal;
        }
        // Get the lengths of repeated matches.
        let mut rep_best = 0;
        let mut rep_lens = [0; REPS];
        for rep in 0..REPS {
            rep_lens[rep] = input
                .buffer()
                .get_match_length(0, state.reps[rep], avail as u32)
                as i32;

            if rep_lens[rep] < MATCH_LEN_MIN as i32 {
                rep_lens[rep] = 0;
                continue;
            }

            if rep_lens[rep] > rep_lens[rep_best] {
                rep_best = rep;
            }
        }

        // Return if the best repeated match is at least niceLen bytes long.
        if rep_lens[rep_best] >= self.nice_len as i32 {
            return EncodeInstruction::Rep {
                rep_index: rep_best as usize,
                len: rep_lens[rep_best] as u32,
            };
        }

        // Initialize mainLen and mainDist to the longest match found
        // by the match finder.
        let mut main_len = 0;
        let main_dist;
        let matches = input.calc_matches();
        if matches.len() > 0 {
            let last = matches.last().unwrap();
            main_len = last.len;
            main_dist = last.distance;

            // Return if it is at least niceLen bytes long.
            if main_len >= self.nice_len {
                return EncodeInstruction::Match(Match {
                    distance: main_dist,
                    len: main_len,
                });
            }
        }

        let cur_byte = input.buffer().get_byte(0);
        let match_byte = input.buffer().get_byte(-(state.reps[0] as i32) - 1);

        // If the match finder found no matches and this byte cannot be
        // encoded as a repeated match (short or long), we must be return
        // to have the byte encoded as a literal.
        if main_len < MATCH_LEN_MIN as u32
            && cur_byte != match_byte
            && rep_lens[rep_best] < MATCH_LEN_MIN as i32
        {
            return EncodeInstruction::Literal;
        }

        let mut pos = input.pos() as u32;
        let mut pos_state = pos & self.pos_mask;

        // Calculate the price of encoding the current byte as a literal.
        {
            let prev_byte = input.buffer().get_byte(-1);
            let literal_price =
                price_calc.get_literal_price(cur_byte, match_byte, prev_byte, pos, &state.state);
            self.opts[1].set1(literal_price, 0, -1);
        }

        let mut any_match_price = price_calc.get_any_match_price(&state.state, pos_state);
        let mut any_rep_price = price_calc.get_any_rep_price(any_match_price, &state.state);

        // If it is possible to encode this byte as a short rep, see if
        // it is cheaper than encoding it as a literal.
        if match_byte == cur_byte {
            let short_rep_price =
                price_calc.get_short_rep_price(any_rep_price, &state.state, pos_state);
            if short_rep_price < self.opts[1].price {
                self.opts[1].set1(short_rep_price, 0, 0);
            }
        }

        // Return if there is neither normal nor long repeated match. Use
        // a short match instead of a literal if is is possible and cheaper.
        self.opt_end = usize::max(main_len as usize, rep_lens[rep_best] as usize);
        if self.opt_end < MATCH_LEN_MIN {
            assert_eq!(self.opt_end, 0);
            let rep_index = self.opts[1].back_prev as usize;
            return EncodeInstruction::Rep { rep_index, len: 1 };
        }

        // Update the lookup tables for distances and lengths before using
        // those price calculation functions. (The price function above
        // don't need these tables.)
        price_calc.update_prices();

        // Initialize the state and reps of this position in opts[].
        // updateOptStateAndReps() will need these to get the new
        // state and reps for the next byte.
        self.opts[0].state.set(state.state);
        self.opts[0].reps = state.reps;

        // Initialize the prices for latter opts that will be used below.
        for i in (MATCH_LEN_MIN..=self.opt_end).rev() {
            self.opts[i].reset();
        }

        // Calculate the prices of repeated matches of all lengths.
        for rep in 0..REPS {
            let rep_len = rep_lens[rep];
            if rep_len < MATCH_LEN_MIN as i32 {
                continue;
            }
            let long_rep_price =
                price_calc.get_long_rep_price(any_rep_price, rep as _, &state.state, pos_state);
            let mut rep_len = rep_len as usize;
            loop {
                let price =
                    long_rep_price + price_calc.get_rep_len_price(rep_len as _, pos_state as _);
                if price < self.opts[rep_len].price {
                    self.opts[rep_len].set1(price, 0, rep as _);
                }
                rep_len -= 1;
                if rep_len < MATCH_LEN_MIN {
                    break;
                }
            }
        }

        // Calculate the prices of normal matches that are longer than rep0.
        {
            let matches = input.calc_matches();
            let mut len = i32::max(rep_lens[0] + 1, MATCH_LEN_MIN as i32);
            if len <= main_len as i32 {
                let normal_match_price =
                    price_calc.get_normal_match_price(any_match_price, &state.state);

                // Set i to the index of the shortest match that is
                // at least len bytes long.
                let mut i = 0;
                while len > matches[i].len as i32 {
                    i += 1;
                }

                loop {
                    let dist = matches[i].distance;
                    let price = price_calc.get_match_and_len_price(
                        normal_match_price,
                        dist as _,
                        len as _,
                        pos_state,
                    );
                    if price < self.opts[len as usize].price {
                        self.opts[len as usize].set1(price, 0, dist as i32 + REPS as i32);
                    }
                    if len == matches[i].len as i32 {
                        i += 1;
                        if i == matches.len() {
                            break;
                        }
                    }
                    len += 1;
                }
            }
        }

        avail = i32::min(input.forward_bytes() as i32, Self::OPTS as i32 - 1);

        // Get matches for later bytes and optimize the use of LZMA symbols
        // by calculating the prices and picking the cheapest symbol
        // combinations.
        while {
            self.opt_cur += 1;
            self.opt_cur < self.opt_end
        } {
            input.increment_pos();
            let matches = input.calc_matches();
            if matches.len() > 0 && matches.last().unwrap().len >= self.nice_len as u32 {
                break;
            }

            avail -= 1;
            pos += 1;
            pos_state = pos & self.pos_mask;

            self.update_opt_state_and_reps();
            any_match_price = self.opts[self.opt_cur].price
                + price_calc.get_any_match_price(&self.opts[self.opt_cur].state, pos_state);
            any_rep_price =
                price_calc.get_any_rep_price(any_match_price, &self.opts[self.opt_cur].state);

            self.calc1_byte_prices(
                input,
                &price_calc,
                pos,
                pos_state,
                avail as _,
                any_rep_price,
            );

            if avail >= MATCH_LEN_MIN as i32 {
                let start_len = self.calc_long_rep_prices(
                    input,
                    &price_calc,
                    pos,
                    pos_state,
                    avail as _,
                    any_rep_price,
                );
                let matches = input.calc_matches();
                if matches.len() > 0 {
                    self.calc_normal_match_prices(
                        input,
                        &price_calc,
                        pos,
                        pos_state,
                        avail as _,
                        any_match_price,
                        start_len as _,
                    );
                }
            }
        }

        let len = self.convert_opts(&mut back) as u32;
        if back == -1 {
            return EncodeInstruction::Literal;
        } else if back < REPS as i32 {
            return EncodeInstruction::Rep {
                rep_index: back as usize,
                len,
            };
        } else {
            return EncodeInstruction::Match(Match {
                distance: back as u32 - REPS as u32,
                len,
            });
        }
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
    const INFINITY_PRICE: u32 = 1 << 30;
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
