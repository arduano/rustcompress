use super::{
    encoder_data_buffer::EncoderDataBuffer,
    match_finding::{Match, MatchFinder},
    LZMACoderState,
};

mod instructions_fast;

pub trait LZMAInstructionPicker {
    fn get_next_symbol(
        &mut self,
        data: &mut LZMAEncoderInput<impl MatchFinder>,
        state: &LZMACoderState,
    ) -> Option<Match>;
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

            // We allow up to 1/4th of the dict to be kept as redundant data before we shift
            // the buffer (which is expensive).
            buffer: EncoderDataBuffer::new(dict_size / 4),

            dict_size,
        }
    }

    pub fn append_data(&mut self, data: &[u8]) {
        self.buffer.append_data(data, self.dict_size);
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
