use super::super::length_codec::MATCH_LEN_MAX;

use self::cyclic_buffer::CyclicBuffer;

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

    pub fn pos(&self) -> u64 {
        self.compress_pos
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

    /// Index of the offset for the underlying cyclical buffer
    pub fn get_byte_index(&self, offset: i32) -> usize {
        (self.forwards_bytes() as i32 - offset - 1) as usize
    }

    /// Get the byte with offset relative to the compress reader head. 0 is the next unread byte.
    pub fn get_byte(&self, offset: i32) -> u8 {
        self.buf.get_relative(self.get_byte_index(offset))
    }

    /// Check if bytes ahead match bytes backwards at a certain delta.
    ///
    /// TODO: Check if we need to use modulo of the delta/len for cases when len is bigger than delta.
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

        let front = self.get_byte(len as i32);
        let back = self.get_byte(len as i32 - delta as i32 - 1);

        front == back
    }

    pub fn is_match_at_least_longer_than(&self, delta: u32, len: u32) -> bool {
        let src_index = self.get_byte_index(0);
        let src = self.buf.as_slices_after(src_index + 1);
        let dst_index = self.get_byte_index(-(delta as i32) - 1);
        let dst = self.buf.as_slices_after(dst_index + 1);

        let (src, dst) = align_slices(src, dst);

        let mut passed = 0;

        for i in 0..3 {
            let src = src[i];
            let dst = dst[i];

            let max = (len - passed).min(src.len() as u32).min(dst.len() as u32);
            let max = max as usize;

            if src[..max] != dst[..max] {
                return false;
            }

            passed += max as u32;
            if passed >= len {
                break;
            }
        }

        true
    }

    pub fn get_match_length(&self, start_len: u32, delta: u32, max_len: u32) -> u32 {
        // The below code is equivalent to
        //
        // ```
        // let mut len = 0;
        // while len < max_len && self.do_bytes_match_at(delta, len) {
        //     len += 1;
        // }
        // len
        // ```
        //
        // Except it loops over slices directly, making this much more auto-SIMD friendly

        let mut len = start_len;

        let src_index = self.get_byte_index(start_len as i32);
        let src = self.buf.as_slices_after(src_index + 1);
        let dst_index = self.get_byte_index(-(delta as i32) + (start_len as i32) - 1);
        let dst = self.buf.as_slices_after(dst_index + 1);

        let (src, dst) = align_slices(src, dst);

        'outer: for i in 0..3 {
            let src = src[i];
            let dst = dst[i];

            let mut j = 0;
            let max = (max_len - len).min(src.len() as u32).min(dst.len() as u32);

            while j < max as usize {
                if src[j] != dst[j] {
                    len += j as u32;
                    break 'outer;
                }

                j += 1;
            }

            len += j as u32;
        }

        len
    }
}

/// Given two pairs of slices, split and align them both into [&[T]; 3] each so that
/// the first two slices are the same length and the last slice is the remainder.
///
/// This is useful for quickly checking matches on contiguous bytes in memory.
fn align_slices<'a, T>(
    mut left: (&'a [T], &'a [T]),
    mut right: (&'a [T], &'a [T]),
) -> ([&'a [T]; 3], [&'a [T]; 3]) {
    // Let's assume that the left one is always smaller for the below code to work
    if left.0.len() > right.0.len() {
        std::mem::swap(&mut left, &mut right);
    }

    let length_diff = right.0.len() - left.0.len();
    let length_diff = length_diff.min(left.1.len());

    let left_1 = left.0;
    let right_1 = &right.0[..left_1.len()];

    let left_2 = &left.1[..length_diff];
    let right_2 = &right.0[left_1.len()..];

    let left_3 = &left.1[length_diff..];
    let right_3 = right.1;

    let left = [left_1, left_2, left_3];
    let right = [right_1, right_2, right_3];

    (left, right)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_align_slices() {
        let left = (&[1, 2, 3][..], &[4, 5, 6, 7][..]);
        let right = (&[8, 9, 10, 11][..], &[12, 13, 14][..]);

        let (left, right) = align_slices(left, right);

        assert_eq!(left[0], &[1, 2, 3][..]);
        assert_eq!(left[1], &[4][..]);
        assert_eq!(left[2], &[5, 6, 7][..]);

        assert_eq!(right[0], &[8, 9, 10][..]);
        assert_eq!(right[1], &[11][..]);
        assert_eq!(right[2], &[12, 13, 14][..]);
    }

    #[test]
    fn test_align_slices_left_longer() {
        let left = (&[1, 2, 3][..], &[4, 5, 6, 7, 8, 9][..]);
        let right = (&[8, 9, 10, 11][..], &[12][..]);

        let (left, right) = align_slices(right, left);

        assert_eq!(left[0], &[1, 2, 3][..]);
        assert_eq!(left[1], &[4][..]);
        assert_eq!(left[2], &[5, 6, 7, 8, 9][..]);

        assert_eq!(right[0], &[8, 9, 10][..]);
        assert_eq!(right[1], &[11][..]);
        assert_eq!(right[2], &[12][..]);
    }

    #[test]
    fn test_align_slices_right_longer() {
        let left = (&[1, 2, 3][..], &[4][..]);
        let right = (&[8, 9, 10, 11][..], &[12, 13, 14, 15][..]);

        let (left, right) = align_slices(right, left);

        assert_eq!(left[0], &[1, 2, 3][..]);
        assert_eq!(left[1], &[4][..]);
        assert_eq!(left[2], &[][..]);

        assert_eq!(right[0], &[8, 9, 10][..]);
        assert_eq!(right[1], &[11][..]);
        assert_eq!(right[2], &[12, 13, 14, 15][..]);
    }

    #[test]
    fn test_align_slices_left_empty() {
        let left = (&[][..], &[][..]);
        let right = (&[1, 2][..], &[3, 4][..]);

        let (left, right) = align_slices(right, left);

        assert_eq!(left[0], &[][..]);
        assert_eq!(left[1], &[][..]);
        assert_eq!(left[2], &[][..]);

        assert_eq!(right[0], &[][..]);
        assert_eq!(right[1], &[1, 2][..]);
        assert_eq!(right[2], &[3, 4][..]);
    }
}
