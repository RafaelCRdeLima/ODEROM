//! A plain, uncompressed component array, indexed by a tuple of
//! coordinate indices. Used for intermediate quantities that either have
//! no clean tensor symmetry (Christoffel symbols are only symmetric in
//! their last two indices, and aren't a tensor at all) or whose symmetry
//! isn't worth exploiting for a computation that touches every component
//! anyway (the mixed-index Riemann tensor `R^a_{bcd}`, on its way to
//! being index-lowered into the fully covariant `R_{abcd}` that *does*
//! get stored compressed, in a [`crate::tensor::ComponentTensor`]).

use oderom_expr::Expr;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

/// An uncompressed `dim^rank`-component array (see the module docs for
/// why this isn't a [`crate::tensor::ComponentTensor`]).
#[derive(Clone, Debug)]
pub struct Grid {
    dim: usize,
    rank: usize,
    values: FxHashMap<SmallVec<[u8; 4]>, Expr>,
}

impl Grid {
    /// An all-zero grid of `dim` coordinates and `rank` indices.
    pub fn new(dim: usize, rank: usize) -> Self {
        Grid { dim, rank, values: FxHashMap::default() }
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Number of indices each component tuple has.
    pub fn rank(&self) -> usize {
        self.rank
    }

    /// Unset components default to zero.
    pub fn get(&self, indices: &[u8]) -> Expr {
        debug_assert_eq!(indices.len(), self.rank);
        self.values.get(indices).cloned().unwrap_or_else(Expr::zero)
    }

    /// Sets the raw component at `indices` (no symmetry applied -- see
    /// [`crate::tensor::ComponentTensor::set`] for that).
    pub fn set(&mut self, indices: &[u8], value: Expr) {
        debug_assert_eq!(indices.len(), self.rank);
        self.values.insert(indices.into(), value);
    }
}
