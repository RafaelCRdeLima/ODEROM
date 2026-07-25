//! A plain, uncompressed component array, indexed by a tuple of
//! coordinate indices. Used for intermediate quantities that either have
//! no clean tensor symmetry (Christoffel symbols are only symmetric in
//! their last two indices, and aren't a tensor at all) or whose symmetry
//! isn't worth exploiting for a computation that touches every component
//! anyway (the mixed-index Riemann tensor `R^a_{bcd}`, on its way to
//! being index-lowered into the fully covariant `R_{abcd}` that *does*
//! get stored compressed, in a [`crate::tensor::ComponentTensor`]).

use oderom_expr::Expr;
use rustc_hash::{FxHashMap, FxHasher};
use smallvec::SmallVec;
use std::hash::{Hash, Hasher};

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

    /// A hash of `self`'s content, stable across two `Grid`s built by
    /// `set`-ting the same components in a different order (DESIGN-UI-
    /// SESSION.md's `DefFingerprint`/compute-cache key needs exactly
    /// this property) -- `#[derive(Hash)]` is not an option here at all
    /// (`FxHashMap` doesn't implement `Hash`), and hashing the map's own
    /// iteration order would silently violate the property being claimed
    /// (that order tracks insertion, not content). Sorts by index first,
    /// so the hash is a function of *what's stored*, never *in what
    /// order it was stored*. `dim`/`rank` are included so a `Grid` and a
    /// differently-shaped one holding coincidentally-identical values
    /// don't collide; no internal id of any kind goes into this (nothing
    /// here is stable across two separate `Registry`s to begin with).
    pub fn canonical_hash(&self) -> u64 {
        let mut entries: Vec<(&SmallVec<[u8; 4]>, &Expr)> = self.values.iter().collect();
        entries.sort_by(|(a, _), (b, _)| a.as_slice().cmp(b.as_slice()));
        let mut hasher = FxHasher::default();
        self.dim.hash(&mut hasher);
        self.rank.hash(&mut hasher);
        for (idx, expr) in entries {
            idx.as_slice().hash(&mut hasher);
            expr.hash(&mut hasher);
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property `canonical_hash` exists for: two `Grid`s holding
    /// identical content, built by calling `set` in a different order,
    /// must hash identically. Not a given -- `FxHashMap`'s own iteration
    /// order depends on insertion order (confirmed by the second half of
    /// this test: hashing the map's *raw* iteration order directly, the
    /// naive thing `#[derive(Hash)]` would have to do if it could be
    /// derived at all, does NOT agree between the two -- exactly the bug
    /// `canonical_hash`'s sort step exists to avoid).
    #[test]
    fn canonical_hash_is_independent_of_insertion_order() {
        let mut forward = Grid::new(3, 2);
        let mut backward = Grid::new(3, 2);
        let components: Vec<(Vec<u8>, Expr)> = vec![
            (vec![0, 0], Expr::int(1)),
            (vec![0, 1], Expr::int(2)),
            (vec![1, 0], Expr::int(3)),
            (vec![2, 2], Expr::int(4)),
        ];
        for (idx, val) in &components {
            forward.set(idx, val.clone());
        }
        for (idx, val) in components.iter().rev() {
            backward.set(idx, val.clone());
        }
        assert_eq!(forward.canonical_hash(), backward.canonical_hash());

        // The naive alternative -- hash the FxHashMap's own iteration
        // order directly, no sort -- is NOT insertion-order-independent
        // (this is *why* canonical_hash sorts first, not a redundant
        // check): demonstrated by actually observing the two orders
        // differ for at least one of these two maps.
        let forward_order: Vec<_> = forward.values.keys().cloned().collect();
        let backward_order: Vec<_> = backward.values.keys().cloned().collect();
        assert_ne!(
            forward_order, backward_order,
            "test assumption violated: FxHashMap iteration order happened to match insertion \
             order here, so this run can't demonstrate why the sort step is necessary -- rerun, \
             or pick different indices"
        );
    }

    #[test]
    fn canonical_hash_differs_for_different_content() {
        let mut a = Grid::new(2, 1);
        a.set(&[0], Expr::int(1));
        let mut b = Grid::new(2, 1);
        b.set(&[0], Expr::int(2));
        assert_ne!(a.canonical_hash(), b.canonical_hash());
    }

    #[test]
    fn canonical_hash_differs_for_different_shape_same_values() {
        // dim/rank are part of the hash, not inferred from content alone.
        let mut a = Grid::new(2, 1);
        a.set(&[0], Expr::int(1));
        let mut b = Grid::new(4, 1);
        b.set(&[0], Expr::int(1));
        assert_ne!(a.canonical_hash(), b.canonical_hash());
    }
}
