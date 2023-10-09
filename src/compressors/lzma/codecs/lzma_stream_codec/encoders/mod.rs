use super::{super::length_codec::MATCH_LEN_MAX, EncoderPriceCalc, state::State};

use self::match_finding::{Match, MatchFinder};

use super::{data_buffers::EncoderDataBuffer, LZMACodec};

pub mod instructions_fast;
// pub mod instructions_normal;
// pub mod instructions_normal2;
pub mod match_finding;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeInstruction {
    Literal,
    Rep { rep_index: usize, len: u32 },
    Match(Match),
}

impl EncodeInstruction {
    pub fn length(&self) -> u32 {
        match *self {
            EncodeInstruction::Literal => 1,
            EncodeInstruction::Match(match_) => match_.len,
            EncodeInstruction::Rep { len, .. } => len,
        }
    }
}

pub trait LZMAInstructionPicker {
    /// Returns the next symbol to encode, optionally progressing the data buffer forwards.
    ///
    /// The data buffer can't be progressed more than the returned instruction's length.
    fn get_next_symbol(
        &mut self,
        data: &mut LZMAEncoderInput<impl MatchFinder>,
        price_calc: &mut EncoderPriceCalc,
        state: &State,
    ) -> EncodeInstruction;
}

pub struct LZMAEncoderInput<M: MatchFinder> {
    buffer: EncoderDataBuffer,
    dict_size: u32,

    matches: Vec<Match>,
    match_finder: M,
    matches_calculated: bool,
}

impl<M: MatchFinder> LZMAEncoderInput<M> {
    pub fn new(match_finder: M, dict_size: u32) -> Self {
        Self {
            matches: Vec::new(),
            match_finder,
            matches_calculated: false,

            // TODO: Investigate `MATCH_LEN_MAX * 20`. It means that the maximum forwards bytes would be
            // 10 times the maximum match length, which lets us do less buffer copy operations
            // when feeding input data.
            buffer: EncoderDataBuffer::new(dict_size, MATCH_LEN_MAX as u32 * 20),

            dict_size,
        }
    }

    pub fn pos(&self) -> u64 {
        self.buffer.pos()
    }

    pub fn dict_size(&self) -> u32 {
        self.dict_size
    }

    /// The number of free bytes that could safely be appended without overwriting the dictionary
    pub fn available_append_bytes(&self) -> usize {
        self.buffer.available_append_bytes()
    }

    /// Appends bytes to the end of the buffer. The length of the slice MUST be smaller or equal to self.available_append_bytes().
    pub fn append_data(&mut self, data: &[u8]) {
        self.buffer.append_data(data);
    }

    pub fn forward_bytes(&self) -> usize {
        self.buffer.forwards_bytes()
    }

    pub fn buffer(&self) -> &EncoderDataBuffer {
        &self.buffer
    }

    pub fn increment_pos(&mut self) {
        self.buffer.increment_pos();
        self.matches_calculated = false;
    }

    pub fn skip(&mut self, len: u32) {
        self.buffer.skip(len);
        self.matches_calculated = false;
    }

    #[inline(always)]
    pub fn calc_matches(&mut self) -> &[Match] {
        if self.forward_bytes() < M::MIN_FORWARDS_BYTES as usize {
            return &[];
        }

        if !self.matches_calculated {
            self.matches.clear();
            self.match_finder
                .find_and_write_matches(&self.buffer, &mut self.matches);
            self.matches_calculated = true;
        }

        &self.matches
    }
}
