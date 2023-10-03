// TODO: For both of these, test if Vec or VecDeque is faster

pub struct EncoderDataBuffer {
    read_pos: u32,
    buf: Vec<u8>,

    /// The interval at which we should trim the buffer. We can't trim it too often because that
    /// would be slow, but we also can't trim it too rarely because that would waste memory.
    trim_interval: u32,
}

impl EncoderDataBuffer {
    pub fn new(trim_interval: u32) -> Self {
        Self {
            read_pos: 0,
            buf: Vec::new(),
            trim_interval,
        }
    }

    pub fn append_data(&mut self, input: &[u8], backward_bytes_to_keep: u32) {
        self.buf.extend_from_slice(input);

        if self.read_pos > backward_bytes_to_keep {
            // We need to try trimming the buffer if we have more than backward_bytes_to_keep bytes
            let data_to_trim = self.read_pos - backward_bytes_to_keep;

            if data_to_trim > self.trim_interval {
                // Get the next multiple of shift_interval
                let data_to_trim = data_to_trim - (data_to_trim % self.trim_interval);

                self.buf.drain(0..data_to_trim as usize);
                self.read_pos -= data_to_trim;
            }
        }
    }

    pub fn available_bytes_forward(&self) -> u32 {
        self.buf.len() as u32 - self.read_pos
    }

    pub fn available_bytes_back(&self) -> u32 {
        self.buf.len() as u32
    }

    pub fn skip(&mut self, len: u32) {
        self.read_pos += len;
    }

    pub fn increment_pos(&mut self) {
        self.read_pos += 1;
    }

    pub fn get_byte(&self, offset: i32) -> u8 {
        debug_assert!(
            (offset + self.read_pos as i32) >= 0,
            "offset: {}, read_pos: {}",
            offset,
            self.read_pos
        );

        self.buf[(self.read_pos as i32 + offset) as usize]
    }

    pub fn do_bytes_match(&self, delta: u32, len: u32) -> bool {
        debug_assert!(
            (self.read_pos as i32 - delta as i32) as i32 >= 0,
            "delta: {}, read_pos: {}",
            delta,
            self.read_pos
        );

        self.get_byte(len as i32 - delta as i32) == self.get_byte(len as i32)
    }

    pub fn get_match_length(&self, delta: u32, max_len: u32) -> u32 {
        let mut len = 0;
        while len < max_len && self.do_bytes_match(delta, len) {
            len += 1;
        }

        len
    }
}

pub struct DecoderDataBuffer {
    flushed_pos: u32,
    dict_size: u32,
    buf: Vec<u8>,

    /// The interval at which we should trim the buffer. We can't trim it too often because that
    /// would be slow, but we also can't trim it too rarely because that would waste memory.
    trim_interval: u32,

    /// The position of the first byte in the buffer relative to the overall file.
    buffer_start_pos: u64,

    /// The length of the overall output stream
    total_file_length: u64,
}

impl DecoderDataBuffer {
    pub fn new(dict_size: u32, total_file_length: u64) -> Self {
        Self {
            buf: Vec::new(),
            dict_size,
            flushed_pos: 0,

            trim_interval: dict_size / 4, // TODO: I guessed this value, should be tested

            buffer_start_pos: 0,
            total_file_length,
        }
    }

    pub fn position(&self) -> u64 {
        self.buffer_start_pos + self.buf.len() as u64
    }

    pub fn append_byte(&mut self, byte: u8) {
        self.buf.push(byte);
    }

    pub fn append_match(&mut self, dist: u32, len: u32) {
        debug_assert!(
            dist < self.buf.len() as u32,
            "dist: {}, buf.len(): {}",
            dist,
            self.buf.len()
        );
        debug_assert!(dist > len, "dist: {}, len: {}", dist, len);

        let start = self.buf.len() - 1 - dist as usize;
        let end = start + len as usize;

        // The below code is equivalent to the commented out code
        //
        // let mut i = start;
        // while i < end {
        //     self.buf.push(self.buf[i]);
        //     i += 1;
        // }

        self.buf.reserve(self.buf.len() + len as usize);
        unsafe {
            self.buf.set_len(self.buf.len() + len as usize);
            std::ptr::copy_nonoverlapping(
                self.buf.as_ptr().add(start),
                self.buf.as_mut_ptr().add(start + len as usize),
                len as usize,
            );
        }
    }

    pub fn available_bytes_back(&self) -> u32 {
        self.buf.len() as u32
    }

    pub fn get_byte(&self, dist: u32) -> u8 {
        debug_assert!(
            dist < self.buf.len() as u32,
            "dist: {}, buf.len(): {}",
            dist,
            self.buf.len()
        );

        self.buf[self.buf.len() - dist as usize - 1]
    }

    /// The number of bytes that we can flush from the buffer.
    pub fn flushable_bytes(&self) -> u32 {
        self.buf.len() as u32 - self.flushed_pos
    }

    /// The number of bytes remaining in the file that we haven't flushed yet.
    pub fn remaining_file_bytes(&self) -> u64 {
        let flushed_pos = self.buffer_start_pos - self.flushed_pos as u64;
        self.total_file_length - flushed_pos
    }

    pub fn flush(&mut self, buf: &mut [u8]) -> usize {
        let bytes_to_flush = buf.len().min(self.flushable_bytes() as usize);
        buf[..bytes_to_flush].copy_from_slice(&self.buf[self.flushed_pos as usize..bytes_to_flush]);

        self.flushed_pos += bytes_to_flush as u32;
        self.try_trim();
        bytes_to_flush
    }

    fn try_trim(&mut self) {
        let min_tail_len = self.dict_size as usize;
        if self.buf.len() > min_tail_len {
            let trimmable = self.buf.len() - min_tail_len;
            let trimmable = trimmable.min(self.flushed_pos as usize);

            if trimmable > self.trim_interval as usize {
                let trim_rounded =
                    trimmable / self.trim_interval as usize * self.trim_interval as usize;

                self.buf.drain(0..trim_rounded);

                self.buffer_start_pos += trim_rounded as u64;
                self.flushed_pos -= trim_rounded as u32;
            }
        }
    }
}
