use super::super::length_codec::MATCH_LEN_MAX;

use self::match_finding::{Match, MatchFinder};

use super::{data_buffers::EncoderDataBuffer, LZMACodec};

mod instructions_fast;
mod match_finding;

pub enum EncodeInstruction {
    Literal,
    Rep { rep_index: usize, len: u32 },
    Match(Match),
}

pub trait LZMAInstructionPicker {
    fn get_next_symbol(
        &mut self,
        data: &mut LZMAEncoderInput<impl MatchFinder>,
        state: &LZMACodec,
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

            // TODO: Investigate `MATCH_LEN_MAX * 10`. It means that the maximum forwards bytes would be
            // 10 times the maximum match length, which lets us do less buffer copy operations
            // when feeding input data.
            buffer: EncoderDataBuffer::new(dict_size, MATCH_LEN_MAX as u32 * 10),

            dict_size,
        }
    }

    /// The number of free bytes that could safely be appended without overwriting the dictionary
    pub fn available_append_bytes(&self) -> usize {
        self.buffer.available_append_bytes()
    }

    /// Appends bytes to the end of the buffer. The length of the slice MUST be smaller or equal to self.available_append_bytes().
    pub fn append_data(&mut self, data: &[u8]) {
        self.buffer.append_data(data);
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
        if !self.matches_calculated {
            self.matches.clear();
            self.match_finder
                .find_and_write_matches(&self.buffer, &mut self.matches);
            self.matches_calculated = true;
        }

        &self.matches
    }
}
