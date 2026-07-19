//! A coordinate chart: Marco 2 has exactly one per manifold (no atlas,
//! no transition functions -- that is Marco 3), just enough to give
//! tensor components a coordinate basis to be expressed in.

/// A coordinate chart: an ordered list of coordinate names, one per
/// dimension. Component index `i` (0-based) is the coefficient along
/// `coords[i]`.
#[derive(Clone, Debug)]
pub struct Chart {
    pub coords: Vec<String>,
}

impl Chart {
    pub fn new(coords: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Chart { coords: coords.into_iter().map(Into::into).collect() }
    }

    /// The manifold's dimension: number of coordinates.
    pub fn dim(&self) -> usize {
        self.coords.len()
    }

    /// The name of coordinate `i` (0-based).
    pub fn coord(&self, i: u8) -> &str {
        &self.coords[i as usize]
    }
}
