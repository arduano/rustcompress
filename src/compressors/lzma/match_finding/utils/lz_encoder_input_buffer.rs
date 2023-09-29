pub struct LZEncoderData {
    pub(crate) keep_size_before: u32,
    pub(crate) keep_size_after: u32,
    pub(crate) match_len_max: u32,
    pub(crate) nice_len: u32,
    pub(crate) buf: Vec<u8>,
    pub(crate) buf_size: u32,
    pub(crate) read_pos: i32,
    pub(crate) read_limit: i32,
    pub(crate) finishing: bool,
    pub(crate) write_pos: i32,
    pub(crate) pending_size: u32,
}

impl LZEncoderData {
    pub fn is_started(&self) -> bool {
        self.read_pos != -1
    }

    pub(super) fn buf_mut(&mut self) -> &mut [u8] {
        &mut self.buf[self.read_pos as usize..]
    }

    // fn set_preset_dict(
    //     &mut self,
    //     dict_size: u32,
    //     preset_dict: &[u8],
    //     match_finder: &mut dyn MatchFind,
    // ) {
    //     assert!(!self.is_started());
    //     assert!(self.write_pos == 0);
    //     let copy_size = preset_dict.len().min(dict_size as usize);
    //     let offset = preset_dict.len() - copy_size;
    //     self.buf[0..copy_size].copy_from_slice(&preset_dict[offset..(offset + copy_size)]);
    //     self.write_pos += copy_size as i32;
    //     match_finder.skip(self, copy_size);
    // }

    // fn move_window(&mut self) {
    //     let move_offset = (self.read_pos + 1 - self.keep_size_before as i32) & !15;
    //     let move_size = self.write_pos as i32 - move_offset;
    //     assert!(move_size >= 0);
    //     assert!(move_offset >= 0);
    //     let move_size = move_size as usize;
    //     let offset = move_offset as usize;
    //     let end = offset + move_size;
    //     unsafe {
    //         std::ptr::copy_nonoverlapping(
    //             self.buf[offset..end].as_ptr(),
    //             self.buf[0..].as_mut_ptr(),
    //             move_size,
    //         );
    //     }
    //     self.read_pos -= move_offset;
    //     self.read_limit -= move_offset;
    //     self.write_pos -= move_offset;
    // }

    // fn fill_window(&mut self, input: &[u8], match_finder: &mut dyn MatchFind) -> usize {
    //     assert!(!self.finishing);
    //     if self.read_pos >= (self.buf_size as i32 - self.keep_size_after as i32) {
    //         self.move_window();
    //     }
    //     let len = if input.len() as i32 > self.buf_size as i32 - self.write_pos {
    //         (self.buf_size as i32 - self.write_pos) as usize
    //     } else {
    //         input.len()
    //     };
    //     let d_start = self.write_pos as usize;
    //     let d_end = d_start + len;
    //     self.buf[d_start..d_end].copy_from_slice(&input[..len]);
    //     self.write_pos += len as i32;
    //     if self.write_pos >= self.keep_size_after as i32 {
    //         self.read_limit = self.write_pos - self.keep_size_after as i32;
    //     }
    //     self.process_pending_bytes(match_finder);
    //     len
    // }

    // fn process_pending_bytes(&mut self, match_finder: &mut dyn MatchFind) {
    //     if self.pending_size > 0 && self.read_pos < self.read_limit {
    //         self.read_pos -= self.pending_size as i32;
    //         let old_pending = self.pending_size;
    //         self.pending_size = 0;
    //         match_finder.skip(self, old_pending as _);
    //         assert!(self.pending_size < old_pending)
    //     }
    // }

    // fn set_flushing(&mut self, match_finder: &mut dyn MatchFind) {
    //     self.read_limit = self.write_pos - 1;
    //     self.process_pending_bytes(match_finder);
    // }
    // fn set_finishing(&mut self, match_finder: &mut dyn MatchFind) {
    //     self.read_limit = self.write_pos - 1;
    //     self.finishing = true;
    //     self.process_pending_bytes(match_finder);
    // }

    // pub fn has_enough_data(&self, already_read_len: i32) -> bool {
    //     self.read_pos - already_read_len < self.read_limit
    // }
    // pub fn copy_uncompressed<W: Write>(
    //     &self,
    //     out: &mut W,
    //     backward: i32,
    //     len: usize,
    // ) -> Result<()> {
    //     let start = (self.read_pos + 1 - backward) as usize;
    //     out.write_all(&self.buf[start..(start + len)])
    // }

    pub fn get_avail(&self) -> i32 {
        assert_ne!(self.read_pos, -1);
        self.write_pos - self.read_pos
    }

    // pub fn get_pos(&self) -> i32 {
    //     self.read_pos
    // }

    pub fn get_byte(&self, offset: i32) -> u8 {
        let start = self.read_pos + offset;
        self.buf[start as usize]
    }

    pub fn get_byte_backward(&self, backward: i32) -> u8 {
        self.buf[(self.read_pos - backward) as usize]
    }

    pub fn get_current_byte(&self) -> u8 {
        self.buf[self.read_pos as usize]
    }

    pub fn get_match_len(&self, dist: i32, len_limit: i32) -> usize {
        let back_pos = self.read_pos - dist - 1;
        let mut len = 0;

        while len < len_limit
            && self.buf[(self.read_pos + len) as usize] == self.buf[(back_pos + len) as usize]
        {
            len += 1;
        }

        len as usize
    }

    pub fn get_match_len2(&self, forward: i32, dist: i32, len_limit: i32) -> u32 {
        let cur_pos = (self.read_pos + forward) as usize;
        let back_pos = cur_pos - dist as usize - 1;
        let mut len = 0;

        while len < len_limit
            && self.buf[cur_pos + len as usize] == self.buf[back_pos + len as usize]
        {
            len += 1;
        }
        return len as _;
    }

    // fn verify_matches(&self, matches: &Matches) -> bool {
    //     let len_limit = self.get_avail().min(self.match_len_max as i32);
    //     for i in 0..matches.count as usize {
    //         if self.get_match_len(matches.dist[i] as i32, len_limit) != matches.len[i] as _ {
    //             return false;
    //         }
    //     }
    //     true
    // }

    pub(super) fn move_pos(
        &mut self,
        required_for_flushing: i32,
        required_for_finishing: i32,
    ) -> u32 {
        assert!(required_for_flushing >= required_for_finishing);
        self.read_pos += 1;
        let mut avail = self.get_avail();
        if avail < required_for_flushing {
            if avail < required_for_finishing || !self.finishing {
                self.pending_size += 1;
                avail = 0;
            }
        }

        debug_assert!(avail >= 0);

        avail as u32
    }
}
