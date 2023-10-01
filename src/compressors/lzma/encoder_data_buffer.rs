pub struct EncoderDataBuffer {
    read_pos: u32,
    buf: Vec<u8>,

    shift_interval: u32,
}

impl EncoderDataBuffer {
    pub fn new(shift_interval: u32) -> Self {
        Self {
            read_pos: 0,
            buf: Vec::new(),
            shift_interval,
        }
    }

    pub fn append_data(&mut self, input: &[u8], backward_bytes: u32) {
        self.buf.extend_from_slice(input);

        if self.read_pos > backward_bytes {
            let data_to_trim = self.read_pos - backward_bytes;

            if data_to_trim > self.shift_interval {
                // Get the next multiple of shift_interval
                let data_to_trim = data_to_trim - (data_to_trim % self.shift_interval);

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

    pub fn do_bytes_match(&self, delta: i32, len: u32) -> bool {
        debug_assert!(
            (self.read_pos as i32 - delta) as i32 >= 0,
            "delta: {}, read_pos: {}",
            delta,
            self.read_pos
        );

        self.get_byte(len as i32 - delta) == self.get_byte(len as i32)
    }
}
