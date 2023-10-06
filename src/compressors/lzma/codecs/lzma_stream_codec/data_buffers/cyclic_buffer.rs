use std::ops::Range;

/// A constant size cyclic buffer structure that allows appending and reading data,
/// but doesn't delete it, just lets the writer overwrite it.
pub struct CyclicBuffer<T: Copy + Default> {
    buf: Vec<T>,

    /// The wrapping position in the buffer. The buffer index is calculated as `pos % buf.len()`.
    pos: u64,
}

// TODO: Improve the panics in this file to be more descriptive

impl<T: Copy + Default> CyclicBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: vec![T::default(); capacity],
            pos: 0,
        }
    }

    pub fn pos(&self) -> u64 {
        self.pos
    }

    pub fn capacity(&self) -> usize {
        self.pos.min(self.buf.len() as u64) as usize
    }

    pub fn max_capacity(&self) -> usize {
        self.buf.len()
    }

    pub fn get(&self, pos: u64) -> T {
        if pos >= self.pos {
            panic!("pos: {}, self.pos: {}", pos, self.pos);
        }

        if self.pos - pos > self.buf.len() as u64 {
            panic!(
                "pos: {}, self.pos: {}, self.buf.len(): {}",
                pos,
                self.pos,
                self.buf.len()
            );
        }

        self.buf[(pos % self.buf.len() as u64) as usize]
    }

    pub fn get_relative(&self, backwards_offset: usize) -> T {
        if backwards_offset > self.buf.len() {
            panic!(
                "backwards_offset: {}, self.buf.len(): {}",
                backwards_offset,
                self.buf.len()
            );
        }

        if self.pos <= backwards_offset as u64 {
            panic!(
                "self.pos: {}, backwards_offset: {}",
                self.pos, backwards_offset
            );
        }

        self.buf[((self.pos - backwards_offset as u64 - 1) % self.buf.len() as u64) as usize]
    }

    pub fn get_last(&self) -> T {
        if self.pos == 0 {
            panic!("Tried to get last element on an empty buffer");
        }

        self.buf[((self.pos - 1) % self.buf.len() as u64) as usize]
    }

    /// Return the array as 2 slices that are contiguous in memory
    pub fn as_slices(&self) -> (&[T], &[T]) {
        let buf = &self.buf;

        if self.pos < buf.len() as u64 {
            // Make sure the first is the empty one, as that's relevant in future functions
            (&[], &buf[..self.pos as usize])
        } else {
            let index = (self.pos % buf.len() as u64) as usize;
            (&buf[index..], &buf[..index])
        }
    }

    /// Return the array as 2 slices that are contiguous in memory after the specified offset
    pub fn as_slices_after(&self, backwards_offset: usize) -> (&[T], &[T]) {
        let buf = &self.buf;
        let index = (self.pos % buf.len() as u64) as usize;

        if backwards_offset > self.pos as usize {
            panic!(
                "backwards_offset: {}, self.pos: {}",
                backwards_offset, self.pos
            );
        }

        if backwards_offset > buf.len() {
            panic!(
                "backwards_offset: {}, buf.len(): {}",
                backwards_offset,
                buf.len()
            );
        }

        if backwards_offset <= index {
            // Make sure the second is the empty one, as that's relevant in future functions
            (&buf[(index - backwards_offset)..index], &[])
        } else {
            (
                &buf[(buf.len() - (backwards_offset - index))..],
                &buf[..index],
            )
        }
    }

    pub fn as_slices_between(&self, backwards_offset_range: Range<usize>) -> (&[T], &[T]) {
        let (left, right) = self.as_slices_after(backwards_offset_range.end);

        let len = backwards_offset_range.end - backwards_offset_range.start;

        // This assumes that the left slice is the one that gets main data, while the right one may be empty
        if left.len() >= len {
            (&left[..len], &[])
        } else {
            (&left[..], &right[..(len - left.len())])
        }
    }

    pub fn push(&mut self, val: T) {
        let index = (self.pos % self.buf.len() as u64) as usize;
        self.buf[index] = val;
        self.pos += 1;
    }

    pub fn push_slice(&mut self, val: &[T]) {
        let index = (self.pos % self.buf.len() as u64) as usize;
        let mut written = 0;
        while written < val.len() {
            let distance_to_end = self.buf.len() - index;
            let to_write = distance_to_end.min(val.len() - written);

            self.buf[index..(index + to_write)]
                .copy_from_slice(&val[written..(written + to_write)]);
            written += to_write;
        }

        self.pos += val.len() as u64;
    }

    /// Append the buffer range from the specified offsets to the end of the buffer.
    /// Technically, the range is reversed as it's backwards.
    ///
    /// The range can't overlap with the destination in the buffer. It will panic if it does.
    ///
    /// This function is significantly faster than doing it manually.
    pub fn append_past_data(&mut self, backwards_offset_range: Range<usize>) {
        let len = backwards_offset_range.end - backwards_offset_range.start;

        // Keep in mind that start and end are reversed, as they're backwards offsets

        if len == 0 {
            return;
        }

        // TODO: Write rigorous unit tests for this function just in case

        if backwards_offset_range.end < len {
            // Can't append overlapping data
            panic!(
                "backwards_offset_range.end: {}, len: {}",
                backwards_offset_range.end, len
            );
        }

        if backwards_offset_range.start > self.buf.len() - len {
            // Can't append overlapping data
            panic!(
                "backwards_offset_range.start: {}, self.buf.len(): {}, len: {}",
                backwards_offset_range.start,
                self.buf.len(),
                len
            );
        }

        if len > self.buf.len() / 2 {
            panic!("len: {}, self.buf.len(): {}", len, self.buf.len());
        }

        let dst_start = (self.pos % self.buf.len() as u64) as usize;
        let mut dst_end = (dst_start + len) % self.buf.len();
        if dst_end == 0 {
            // This condition is ok because we covered the edge case of
            // len being 0 or len being >= self.buf.len()
            dst_end = self.buf.len();
        }

        let src_start = (self.pos - 1 - backwards_offset_range.end as u64) % self.buf.len() as u64;
        let src_start = src_start as usize;
        let mut src_end = (src_start + len) % self.buf.len();
        if src_end == 0 {
            // Same as the above condition
            src_end = self.buf.len();
        }

        // I tried implementing the below in safe rust, just in case, without using raw pointers or type casting

        // 4 conditions:
        // 1. src range is overlapping the buffer end
        // 2. dst range is overlapping the buffer end
        // 3. neither are overlapping the buffer end, src is before dst
        // 4. same as above, but dst is before src

        let buf = &mut self.buf;

        // + is src, - is dst
        if src_end < src_start {
            // The array is like
            // ++++++++++++]...........[------------]......[+++++++++++
            // src_second               dst                 src_first

            let (buf, src_first) = buf.split_at_mut(src_start);
            let (src_second, buf) = buf.split_at_mut(src_end);
            let (_, buf) = buf.split_at_mut(dst_start - src_end);
            let (dst, _) = buf.split_at_mut(dst_end - dst_start);

            dst[..src_first.len()].copy_from_slice(src_first);
            dst[src_first.len()..].copy_from_slice(src_second);
        } else if dst_end < dst_start {
            // The array is like
            // ------------]...........[++++++++++++]......[-----------
            // dst_second               src                 dst_first

            let (buf, dst_first) = buf.split_at_mut(dst_start);
            let (dst_second, buf) = buf.split_at_mut(dst_end);
            let (_, buf) = buf.split_at_mut(src_start - dst_end);
            let (src, _) = buf.split_at_mut(src_end - src_start);

            dst_first.copy_from_slice(&src[..dst_first.len()]);
            dst_second.copy_from_slice(&src[dst_first.len()..]);
        } else if src_start < dst_start {
            // The array is like
            // ....[++++++++++++]......[-----------].....
            //      src                 dst

            let (_, buf) = buf.split_at_mut(src_start);
            let (src, buf) = buf.split_at_mut(src_end - src_start);
            let (_, buf) = buf.split_at_mut(dst_start - src_end);
            let (dst, _) = buf.split_at_mut(dst_end - dst_start);

            dst.copy_from_slice(src);
        } else {
            // The array is like
            // ....[-----------]......[++++++++++++].....
            //      dst                src

            let (_, buf) = buf.split_at_mut(dst_start);
            let (dst, buf) = buf.split_at_mut(dst_end - dst_start);
            let (_, buf) = buf.split_at_mut(src_start - dst_end);
            let (src, _) = buf.split_at_mut(src_end - src_start);

            dst.copy_from_slice(src);
        }

        self.pos += len as u64;
    }

    pub fn iter_after(&self) {
        // let (left, right) = self.as_slices();
        // let parts = [left, right];

        // let flush_start = self.flushed_pos;
        // let flush_end = self.flushed_pos + bytes_to_flush;

        // // The iterator just operates on 2 slices, but it saves me copy pasting those if statements
        // let mut cumulative_pos = 0;
        // let copy_parts = parts
        //     .into_iter()
        //     .map(|p| {
        //         // Include the slice start pos in the iterator
        //         let result = (cumulative_pos, p);
        //         cumulative_pos += p.len();
        //         result
        //     })
        //     .filter_map(|(start, mut slice)| {
        //         let end = start + slice.len();

        //         if start >= flush_end || end <= flush_start {
        //             // Start is outside the range, skip
        //             return None;
        //         }

        //         if start < flush_start {
        //             // Start is behind, trim the slice
        //             slice = &slice[(flush_start - start)..];
        //         }

        //         if end > flush_end {
        //             // End is ahead, trim the slice
        //             slice = &slice[..(flush_end - start)];
        //         }

        //         Some(slice)
        //     });
        todo!();
    }
}
