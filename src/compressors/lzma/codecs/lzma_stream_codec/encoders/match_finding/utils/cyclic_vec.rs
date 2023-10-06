/// A simple constant length cyclic vector, where the position increments by one
/// each time a new element is added, looping around.
///
/// It is slightly faster than a VecDeque, as it does less bounds checking.
pub struct CyclicVec<T: Default> {
    buf: Vec<T>,
    pos: usize,
}

impl<T: Default> CyclicVec<T> {
    pub fn new(size: usize) -> Self {
        let mut buf = Vec::with_capacity(size);
        for _ in 0..size {
            buf.push(Default::default());
        }

        Self { buf, pos: 0 }
    }

    pub fn push(&mut self, value: T) {
        self.buf[self.pos] = value;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
    }

    pub fn replace_current(&mut self, value: T) {
        self.buf[self.pos] = value;
    }

    pub fn get_backwards(&self, index: usize) -> &T {
        debug_assert!(index < self.buf.len());

        if index > self.pos {
            let index = self.pos + self.buf.len() - index;
            &self.buf[index]
        } else {
            &self.buf[self.pos - index]
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        let (a, b) = self.buf.split_at(self.pos);
        a.iter().rev().chain(b.iter().rev())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        let (a, b) = self.buf.split_at_mut(self.pos);
        a.iter_mut().rev().chain(b.iter_mut().rev())
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }
}

impl<T: std::fmt::Debug + Default> std::fmt::Debug for CyclicVec<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_list();
        for byte in self.iter() {
            debug.entry(byte);
        }
        debug.finish()
    }
}
