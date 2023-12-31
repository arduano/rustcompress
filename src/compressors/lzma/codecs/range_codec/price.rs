use std::ops::*;

use super::{RangeEncProbability, BIT_MODEL_TOTAL};

const MOVE_REDUCING_BITS: usize = 4;
const BIT_PRICE_SHIFT_BITS: usize = 4;

const INFINITY_PRICE: u32 = 1 << 30;

const PRICES: [u32; 128] = [
    0x80, 0x67, 0x5b, 0x54, 0x4e, 0x49, 0x45, 0x42, 0x3f, 0x3d, 0x3a, 0x38, 0x36, 0x34, 0x33, 0x31,
    0x30, 0x2e, 0x2d, 0x2c, 0x2b, 0x2a, 0x29, 0x28, 0x27, 0x26, 0x25, 0x24, 0x23, 0x22, 0x22, 0x21,
    0x20, 0x1f, 0x1f, 0x1e, 0x1d, 0x1d, 0x1c, 0x1c, 0x1b, 0x1a, 0x1a, 0x19, 0x19, 0x18, 0x18, 0x17,
    0x17, 0x16, 0x16, 0x16, 0x15, 0x15, 0x14, 0x14, 0x13, 0x13, 0x13, 0x12, 0x12, 0x11, 0x11, 0x11,
    0x10, 0x10, 0x10, 0xf, 0xf, 0xf, 0xe, 0xe, 0xe, 0xd, 0xd, 0xd, 0xc, 0xc, 0xc, 0xb, 0xb, 0xb,
    0xb, 0xa, 0xa, 0xa, 0xa, 0x9, 0x9, 0x9, 0x9, 0x8, 0x8, 0x8, 0x8, 0x7, 0x7, 0x7, 0x7, 0x6, 0x6,
    0x6, 0x6, 0x5, 0x5, 0x5, 0x5, 0x5, 0x4, 0x4, 0x4, 0x4, 0x3, 0x3, 0x3, 0x3, 0x3, 0x2, 0x2, 0x2,
    0x2, 0x2, 0x2, 0x1, 0x1, 0x1, 0x1, 0x1,
];

/// The price of encoding a bit or set of bits with a given probability.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct RangeEncPrice(u32);

impl RangeEncPrice {
    #[inline(always)]
    pub fn zero() -> RangeEncPrice {
        RangeEncPrice(0)
    }

    #[inline(always)]
    pub fn infinity() -> RangeEncPrice {
        RangeEncPrice(INFINITY_PRICE)
    }

    #[inline(always)]
    pub fn get_bit_price(prob: RangeEncProbability, bit: u32) -> RangeEncPrice {
        let bit = bit as i32;
        debug_assert!(bit == 0 || bit == 1);
        let i = ((prob.0 as u32) ^ ((-bit) as u32 & (BIT_MODEL_TOTAL - 1))) >> MOVE_REDUCING_BITS;

        #[cfg(all(feature = "unsafe", not(debug_assertions)))]
        // TODO: See if it's possible to optimize the bounds check away automatically, instead of manually doing it here the unsafe way.
        if i >= PRICES.len() as u32 {
            unsafe {
                core::hint::unreachable_unchecked();
            }
        }

        RangeEncPrice(PRICES[i as usize])
    }

    #[inline(always)]
    pub fn get_direct_bits_price(count: u32) -> RangeEncPrice {
        RangeEncPrice(count << BIT_PRICE_SHIFT_BITS)
    }
}

impl Add for RangeEncPrice {
    type Output = Self;

    #[inline(always)]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for RangeEncPrice {
    #[inline(always)]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Mul<u32> for RangeEncPrice {
    type Output = Self;

    #[inline(always)]
    fn mul(self, rhs: u32) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl MulAssign<u32> for RangeEncPrice {
    #[inline(always)]
    fn mul_assign(&mut self, rhs: u32) {
        self.0 *= rhs;
    }
}
