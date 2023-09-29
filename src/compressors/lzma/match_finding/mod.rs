pub mod brute_force;
pub mod hc4;
pub mod utils;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct Match {
    pub distance: u32,
    pub len: u32,
}

impl PartialOrd for Match {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.distance.cmp(&other.distance))
    }
}

impl Ord for Match {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.distance.cmp(&other.distance)
    }
}

pub trait MatchFinder {
    fn find_and_write_matches(
        &mut self,
        buffer: &impl MatchInputBuffer,
        output_matches_vec: &mut Vec<Match>,
    );
    fn skip_byte(&mut self, buffer: &impl MatchInputBuffer);
}

pub trait MatchInputBuffer {
    fn available_bytes(&self) -> u32;
    fn tail_bytes(&self) -> u32;
    fn get_byte(&self, offset: i32) -> u8;
}
