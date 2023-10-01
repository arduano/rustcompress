pub struct EncoderDataBuffer {
    read_pos: u32,
    buf: Vec<u8>,

    trim_interval: u32,
}

impl EncoderDataBuffer {
    pub fn new(shift_interval: u32) -> Self {
        Self {
            read_pos: 0,
            buf: Vec::new(),
            trim_interval: shift_interval,
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
