use super::{price::RangeEncPrice, BIT_MODEL_TOTAL, MOVE_BITS};

const PROB_INIT: u16 = (BIT_MODEL_TOTAL / 2) as u16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeEncProbability(pub u16);

impl Default for RangeEncProbability {
    fn default() -> Self {
        Self::new()
    }
}

impl RangeEncProbability {
    pub fn new() -> Self {
        Self(PROB_INIT)
    }

    pub fn increment(&mut self) {
        let mut prob = self.0 as u32;
        prob += (BIT_MODEL_TOTAL - prob) >> MOVE_BITS;
        self.0 = prob as u16;
    }

    pub fn decrement(&mut self) {
        let mut prob = self.0 as u32;
        prob -= prob >> MOVE_BITS;
        self.0 = prob as u16;
    }

    pub fn get_bit_price(self, bit: u32) -> RangeEncPrice {
        RangeEncPrice::get_bit_price(self, bit)
    }
}
