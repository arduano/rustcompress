const REPS: usize = 4;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct State {
    reps: [u32; REPS],
    state: u8,
}
pub const STATES: usize = 12;

const LIT_STATES: u8 = 7;
const LIT_LIT: u8 = 0;
const _MATCH_LIT_LIT: u8 = 1;
const _REP_LIT_LIT: u8 = 2;
const SHORTREP_LIT_LIT: u8 = 3;
const _MATCH_LIT: u8 = 4;
const _REP_LIT: u8 = 5;
const _SHORTREP_LIT: u8 = 6;
const LIT_MATCH: u8 = 7;
const LIT_LONGREP: u8 = 8;
const LIT_SHORTREP: u8 = 9;
const NONLIT_MATCH: u8 = 10;
const NONLIT_REP: u8 = 11;

impl State {
    pub fn new() -> Self {
        Self {
            state: 0,
            reps: [0; REPS],
        }
    }

    // pub fn reset(&mut self) {
    //     self.state = LIT_LIT;
    // }

    // pub fn set(&mut self, other: State) {
    //     self.state = other.state;
    // }

    pub fn get_idx(&self) -> u8 {
        self.state
    }

    pub fn get_rep(&self, index: usize) -> u32 {
        self.reps[index]
    }

    pub fn reps(&self) -> &[u32; REPS] {
        &self.reps
    }

    pub fn update_literal(&mut self) {
        if self.state <= SHORTREP_LIT_LIT {
            self.state = LIT_LIT;
        } else if self.state <= LIT_SHORTREP {
            self.state -= 3;
        } else {
            self.state -= 6;
        }
    }

    #[inline(always)]
    pub fn update_match(&mut self, distance: u32) {
        self.reps[3] = self.reps[2];
        self.reps[2] = self.reps[1];
        self.reps[1] = self.reps[0];
        self.reps[0] = distance;

        self.state = if self.state < LIT_STATES {
            LIT_MATCH
        } else {
            NONLIT_MATCH
        };
    }

    #[inline(always)]
    pub fn update_long_rep(&mut self, rep: usize) -> u32 {
        let rep_value = self.reps[rep];

        if rep != 0 {
            if rep != 1 {
                if rep == 3 {
                    self.reps[3] = self.reps[2];
                }
                self.reps[2] = self.reps[1];
            }

            self.reps[1] = self.reps[0];
            self.reps[0] = rep_value;
        }

        self.state = if self.state < LIT_STATES {
            LIT_LONGREP
        } else {
            NONLIT_REP
        };

        rep_value
    }

    #[inline(always)]
    pub fn update_short_rep(&mut self) {
        self.state = if self.state < LIT_STATES {
            LIT_SHORTREP
        } else {
            NONLIT_REP
        };
    }

    pub fn is_literal(&self) -> bool {
        return self.state < LIT_STATES;
    }
}
