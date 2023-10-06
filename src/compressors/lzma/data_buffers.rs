use self::cyclic_buffer::CyclicBuffer;

use super::codecs::length_codec::MATCH_LEN_MAX;

mod cyclic_buffer;

// TODO: Port EncoderDataBuffer to use CyclicBuffer

pub struct EncoderDataBuffer {
    compress_pos: u64,
    max_forwards_bytes: u32,
    dict_size: u32,
    buf: CyclicBuffer<u8>,
}

impl EncoderDataBuffer {
    pub fn new(dict_size: u32, max_forwards_bytes: u32) -> Self {
        // max_forwards_bytes is the number of extra bytes that should be stored on top of the dict_size.
        // It is used to find matches, so the minimum forward_bytes should be the maximum match size.
        // However, bigger max_forwards_bytes means less data copying is needed so it may be faster.
        Self {
            buf: CyclicBuffer::new((dict_size + max_forwards_bytes) as usize),
            compress_pos: 0 as u64,
            max_forwards_bytes,
            dict_size,
        }
    }

    /// The number of bytes ahead that are currently in the buffer
    pub fn forwards_bytes(&self) -> usize {
        (self.buf.pos() - self.compress_pos) as usize
    }

    /// The number of dictionary bytes. Can be higher than dict_size if fowards_bytes() is smaller than max_forwards_bytes.
    pub fn backwards_bytes(&self) -> usize {
        self.buf.capacity() - self.forwards_bytes()
    }

    /// The number of free bytes that could safely be appended without overwriting the dictionary
    pub fn available_append_bytes(&self) -> usize {
        self.max_forwards_bytes as usize - self.forwards_bytes()
    }

    /// Appends bytes to the end of the buffer. The length of the slice MUST be smaller or equal to self.available_append_bytes().
    pub fn append_data(&mut self, input: &[u8]) {
        if input.len() > self.available_append_bytes() {
            panic!(
                "input.len(): {}, available_append_bytes(): {}",
                input.len(),
                self.available_append_bytes()
            );
        }

        self.buf.push_slice(input)
    }

    pub fn skip(&mut self, len: u32) {
        debug_assert!(
            len <= self.forwards_bytes() as u32,
            "len: {}, forwards_bytes(): {}",
            len,
            self.forwards_bytes()
        );

        self.compress_pos += len as u64;
    }

    pub fn increment_pos(&mut self) {
        debug_assert!(
            self.forwards_bytes() > 0,
            "forwards_bytes(): {}",
            self.forwards_bytes()
        );

        self.compress_pos += 1;
    }

    pub fn get_byte(&self, offset: i32) -> u8 {
        self.buf
            .get_relative((self.forwards_bytes() as i32 - offset) as usize)
    }

    pub fn do_bytes_match_at(&self, delta: u32, len: u32) -> bool {
        debug_assert!(
            delta as usize <= self.backwards_bytes(),
            "delta: {}, backwards_bytes(): {}",
            delta,
            self.backwards_bytes()
        );

        debug_assert!(
            (len as usize) < self.forwards_bytes(),
            "len: {}, forwards_bytes(): {}",
            len,
            self.forwards_bytes()
        );

        let zero_offset = self.forwards_bytes();
        let front_pos = zero_offset - len as usize;
        let back_pos = front_pos + delta as usize;

        let front = self.buf.get_relative(front_pos);
        let back = self.buf.get_relative(back_pos);

        front == back
    }

    pub fn get_match_length(&self, delta: u32, max_len: u32) -> u32 {
        let mut len = 0;
        // TODO: Optimize this with fetching entire slices at once and comparing
        while len < max_len && self.do_bytes_match_at(delta, len) {
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

        let overlaps_head = len > dist;
        let overlaps_tail = self.buf.max_capacity() as u32 - dist > len;

        if overlaps_head || overlaps_tail {
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
