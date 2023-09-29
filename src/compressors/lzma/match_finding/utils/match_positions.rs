/// The relative index of the current "read head". The index isn't absolute as it gets shifted down once it reaches 0x7FFFFFFF.
///
/// However, every LzMatchPos is relative to the current LzReadPos.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MatchReadPos(u32);

impl MatchReadPos {
    // TODO: Test normalization functions correctly by reducing the below value to something small.
    const MAX_SAFE_READ_POS: Self = Self(0x7FFFFFFF);

    pub fn new(pos: u32) -> Self {
        Self(pos)
    }

    pub fn increment(&mut self) -> PosIncrementResult {
        self.0 += 1;

        if self.0 == 0 {
            PosIncrementResult::ShouldNormalize
        } else {
            PosIncrementResult::Ok
        }
    }

    pub fn get_norm_offset(&self, buffer_size: usize) -> u32 {
        assert!(
            self.0 > buffer_size as u32,
            "Normalization was attempted at an unexpected state"
        );

        self.0 - buffer_size as u32
    }

    /// Normalize the position if it gets too high.
    pub fn subtract_offset(&mut self, offset: u32) -> Self {
        assert!(offset <= self.0, "Normalizing offset is too high");

        Self(self.0 - offset)
    }

    pub fn as_match_pos(&self) -> MatchPos {
        MatchPos(self.0)
    }
}

#[must_use]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PosIncrementResult {
    Ok,
    ShouldNormalize,
}

/// The index of a match referenced by a hash table entry. The index may or may not be in the cyclic buffer.
/// It is always relative to the current LzReadPos.
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MatchPos(u32);

impl MatchPos {
    pub fn new() -> Self {
        // The default value is zero, which can give invalid hits, but it's fine.
        // When a hit is invalid, it still checks the bits of the stream to make sure.
        Self(0)
    }

    /// Get the delta relative to the read position.
    /// Return i32 because we get the negative of the delta often
    pub fn get_delta(&self, read: MatchReadPos) -> i32 {
        read - *self
    }

    /// Normalize the position if it gets too high.
    pub fn subtract_offset(&self, offset: u32) -> Self {
        if offset > self.0 {
            Self(0)
        } else {
            Self(self.0 - offset)
        }
    }
}

impl std::fmt::Debug for MatchPos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MatchPos({})", self.0)
    }
}

impl std::ops::Sub<MatchPos> for MatchReadPos {
    // Return i32 because we get the negative of the delta often
    type Output = i32;

    fn sub(self, rhs: MatchPos) -> Self::Output {
        debug_assert!(self.0 >= rhs.0);

        self.0 as i32 - rhs.0 as i32
    }
}
