use crate::utils::cache_cell::{CacheCell, CacheCellGuard};

use super::{
    encoder_data_buffer::{EncoderDataBuffer, EncoderDataBufferProjection},
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

    /// The cache of the buffer's matches for the current position.
    ///
    /// This is lazily calculated and invalidated when the buffer location is changed.
    /// We use a CacheCell so that this struct can be passed around as immutable, while
    /// still allowing the matches to be lazily calculated and cached.
    matches: CacheCell<MatchCache<M>>,
}

impl<M: MatchFinder> LZMAEncoderInput<M> {
    pub fn new(match_finder: M, dict_size: u32) -> Self {
        Self {
            matches: CacheCell::new(MatchCache {
                matches: Vec::new(),
                match_finder,
                calculated_at_pos: None,
            }),

            // We allow up to 1/4th of the dict to be kept as redundant data before we shift
            // the buffer (which is expensive).
            buffer: EncoderDataBuffer::new(dict_size / 4),

            dict_size,
        }
    }

    pub fn append_data(&mut self, data: &[u8]) {
        self.buffer.append_data(data, self.dict_size);
        self.matches.update(|v| v.calculated_at_pos = None);
    }

    pub fn buffer(&self) -> EncoderDataBufferProjection {
        self.buffer.projection()
    }

    pub fn update_cache_for_offset(&self, offset: u32) {
        let bytes = self.buffer.available_bytes_forward();
        debug_assert!(offset <= bytes);

        self.matches.update(|v| {
            if v.calculated_at_pos != Some(bytes - offset) {
                v.calculated_at_pos = Some(bytes - offset);

                v.matches.clear();
                v.match_finder
                    .find_and_write_matches(&self.buffer.offset_projection(offset), &mut v.matches);
            }
        })
    }

    pub fn increment_pos(&mut self) {
        self.buffer.increment_pos();
    }

    pub fn skip(&mut self, len: u32) {
        self.buffer.skip(len);
    }

    #[inline(always)]
    pub fn calc_matches(&self) -> CacheCellGuard<[Match]> {
        self.update_cache_for_offset(0);
        self.matches.get().map_inner(|v| v.matches.as_slice())
    }

    #[inline(always)]
    pub fn calc_matches_at_offset(&mut self, offset: u32) -> CacheCellGuard<[Match]> {
        self.update_cache_for_offset(offset);
        self.matches.get().map_inner(|v| v.matches.as_slice())
    }
}

struct MatchCache<M: MatchFinder> {
    matches: Vec<Match>,
    match_finder: M,
    calculated_at_pos: Option<u32>,
}
