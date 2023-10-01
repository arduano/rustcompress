pub mod encoder_data_buffer;
pub mod encoders;
pub mod length_codec;
pub mod literals_codec;
pub mod match_finding;
pub mod range_codec;

pub enum LzState {}

const STATES: usize = 12;
const POS_STATES_MAX: usize = 1 << 4;
const REPS: usize = 4;
const DIST_STATES: usize = 4;
const DIST_SLOTS: usize = 1 << 6;

const ALIGN_BITS: usize = 4;
const ALIGN_SIZE: usize = 1 << ALIGN_BITS;
const ALIGN_MASK: usize = ALIGN_SIZE - 1;

// TODO: Move this to other places when I figure out the point of this

pub struct LZMACoderState {
    // pub(crate) pos_mask: u32,
    pub reps: [u32; REPS],
    pub state: LzState,
    // pub(crate) is_match: [[u16; POS_STATES_MAX]; STATES],
    // pub(crate) is_rep: [u16; STATES],
    // pub(crate) is_rep0: [u16; STATES],
    // pub(crate) is_rep1: [u16; STATES],
    // pub(crate) is_rep2: [u16; STATES],
    // pub(crate) is_rep0_long: [[u16; POS_STATES_MAX]; STATES],
    // pub(crate) dist_slots: [[u16; DIST_SLOTS]; DIST_STATES],
    // dist_special: [u16; 124],
    // dist_align: [u16; ALIGN_SIZE], // Prices
}
