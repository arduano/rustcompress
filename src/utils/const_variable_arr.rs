/// A constant length array with a variable length exposed slice at runtime.
///
/// This is mainly used for performance, as it allows us to have a stack allocated array
/// with a variable length slice, without having to allocate a new Vec.
#[derive(Debug, Clone)]
pub struct ConstVariableArr<T, const MAX_LEN: usize> {
    arr: [T; MAX_LEN],
    len: usize,
}

impl<T, const MAX_LEN: usize> ConstVariableArr<T, MAX_LEN> {
    pub fn new(val: T, len: usize) -> Self
    where
        T: Clone,
    {
        Self {
            // We use clone not copy because some places we use this have non-copy types
            arr: array_macro::array![val; MAX_LEN],
            len,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn as_slice(&self) -> &[T] {
        &self.arr[..self.len]
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.arr[..self.len]
    }
}

impl std::ops::Deref for ConstVariableArr<u8, 32> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl std::ops::DerefMut for ConstVariableArr<u8, 32> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut_slice()
    }
}

impl<T, const MAX_LEN: usize> std::ops::Index<usize> for ConstVariableArr<T, MAX_LEN> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        &(&self.arr)[index]
    }
}

impl<T, const MAX_LEN: usize> std::ops::IndexMut<usize> for ConstVariableArr<T, MAX_LEN> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut (&mut self.arr)[index]
    }
}
