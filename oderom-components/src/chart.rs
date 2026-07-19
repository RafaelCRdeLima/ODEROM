//! A coordinate chart, and (Marco 3) an atlas of several charts related
//! by transition maps.

use oderom_types::Domain;

/// A coordinate chart: an ordered list of coordinate names, one per
/// dimension, and the domain (see [`oderom_types::Domain`]) it is valid
/// on -- `Everywhere` unless restricted via [`Chart::with_domain`].
/// Component index `i` (0-based) is the coefficient along `coords[i]`.
#[derive(Clone, Debug)]
pub struct Chart {
    pub coords: Vec<String>,
    pub domain: Domain,
}

impl Chart {
    pub fn new(coords: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Chart { coords: coords.into_iter().map(Into::into).collect(), domain: Domain::Everywhere }
    }

    pub fn with_domain(mut self, domain: Domain) -> Self {
        self.domain = domain;
        self
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
