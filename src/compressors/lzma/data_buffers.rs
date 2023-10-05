use self::cyclic_buffer::CyclicBuffer;

use super::codecs::length_codec::MATCH_LEN_MAX;

mod cyclic_buffer;

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
    flushed_pos: u64,
    buf: CyclicBuffer<u8>,

    /// The length of the overall output stream
    total_file_length: u64,
}

impl DecoderDataBuffer {
    pub fn new(dict_size: u32, total_file_length: u64) -> Self {
        Self {
            buf: CyclicBuffer::new(dict_size as usize),
            flushed_pos: 0,
            total_file_length,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buf.pos() == 0
    }

    pub fn position(&self) -> u64 {
        self.buf.pos()
    }

    pub fn append_byte(&mut self, byte: u8) {
        self.buf.push(byte);
    }

    pub fn append_match(&mut self, dist: u32, len: u32) {
        debug_assert!(
            dist < self.buf.capacity() as u32,
            "dist: {}, buf.capacity(): {}",
            dist,
            self.buf.capacity()
        );

        if len > dist {
            for _ in 0..len {
                let byte = self.buf.get_relative(dist as usize);
                self.buf.push(byte);
            }
        } else {
            let dist = dist as usize;
            let len = len as usize;
            let backwards_offset_range = (dist - len)..dist;
            self.buf.append_past_data(backwards_offset_range)
        }
    }

    pub fn available_bytes_back(&self) -> u32 {
        self.buf.capacity() as u32
    }

    pub fn get_byte(&self, dist: u32) -> u8 {
        debug_assert!(
            dist < self.buf.capacity() as u32,
            "dist: {}, buf.capacity(): {}",
            dist,
            self.buf.capacity()
        );

        self.buf.get_relative(dist as usize)
    }

    /// The number of bytes that we can flush from the buffer.
    pub fn flushable_bytes(&self) -> u32 {
        (self.buf.pos() - self.flushed_pos) as u32
    }

    /// An important condition for the encoder is that it must flush the buffer before it gets full.
    /// I didn't want to add protection for this when actually appending data because it would be slow.
    pub fn must_flush_now_or_data_will_be_lost(&self) -> bool {
        let safe_bytes = self.buf.capacity() as u32 - self.flushable_bytes();
        safe_bytes < MATCH_LEN_MAX as u32 - 1
    }

    /// The number of bytes remaining in the file that we haven't flushed yet.
    pub fn remaining_file_bytes(&self) -> u64 {
        self.total_file_length - self.flushed_pos
    }

    pub fn flush(&mut self, buf: &mut [u8]) -> usize {
        let flushable = self.flushable_bytes() as usize;
        let bytes_to_flush = buf.len().min(flushable);

        let backwards_offset_range = (flushable - bytes_to_flush)..flushable;
        let (left, right) = self.buf.as_slices_between(backwards_offset_range);

        let buf = &mut buf[..bytes_to_flush];
        buf[..left.len()].copy_from_slice(left);
        buf[left.len()..].copy_from_slice(right);

        self.flushed_pos += bytes_to_flush as u64;

        bytes_to_flush
    }
}
