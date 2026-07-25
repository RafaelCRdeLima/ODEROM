//! Storage of only a tensor's independent components, keyed by orbit
//! representative under its head's declared symmetry group -- the same
//! [`oderom_core::Bsgs`] built once at `TensorHead` declaration (Marco 1)
//! and, via [`oderom_core::Bsgs::for_each_element`], the same enumeration
//! primitive `oderom-canon` uses to canonicalize abstract dummy/free
//! index structure. Here the group acts on a *concrete* tuple of
//! coordinate indices instead: for a rank-`n` head, this is a search for
//! the lexicographically minimal image of `indices` over every group
//! element, with the accompanying sign, using the exact algorithm
//! `oderom-canon::coset::search_minimal` uses for words -- just scored by
//! the index tuple itself instead of a dummy/free descriptor.

use crate::error::ComponentError;
use oderom_core::{Bsgs, HeadId, Registry, SignedPerm};
use oderom_expr::Expr;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use smallvec::SmallVec;
use std::hash::{Hash, Hasher};

pub(crate) type IndexTuple = SmallVec<[u8; 4]>;

pub(crate) enum Orbit {
    /// The symmetry group forces every component in this orbit to equal
    /// its own negative.
    Zero,
    Representative(IndexTuple, i8),
}

pub(crate) fn canonical_indices(bsgs: &Bsgs, indices: &[u8]) -> Orbit {
    let mut best: Option<IndexTuple> = None;
    let mut best_sign = 1i8;
    let mut signs_at_best: FxHashSet<i8> = FxHashSet::default();

    bsgs.for_each_element(|g: &SignedPerm| {
        let mut candidate: IndexTuple = smallvec::smallvec![0; indices.len()];
        for (i, &value) in indices.iter().enumerate() {
            candidate[g.perm.image(i as u16) as usize] = value;
        }
        match &best {
            None => {
                best_sign = g.sign;
                signs_at_best.clear();
                signs_at_best.insert(g.sign);
                best = Some(candidate);
            }
            Some(b) if candidate < *b => {
                best_sign = g.sign;
                signs_at_best.clear();
                signs_at_best.insert(g.sign);
                best = Some(candidate);
            }
            Some(b) if candidate == *b => {
                signs_at_best.insert(g.sign);
            }
            _ => {}
        }
    });

    if signs_at_best.len() > 1 {
        return Orbit::Zero;
    }
    Orbit::Representative(best.expect("group always has at least the identity"), best_sign)
}

/// A tensor's components in one [`crate::chart::Chart`], stored one
/// [`Expr`] per symmetry orbit rather than one per raw index tuple.
#[derive(Clone, Debug)]
pub struct ComponentTensor {
    head: HeadId,
    independent: FxHashMap<IndexTuple, Expr>,
    /// Per orbit representative, the raw indices and raw (unscaled)
    /// value of the *first* [`Self::set_declared`] call that landed
    /// there -- used only to name both sides of a
    /// [`ComponentError::ConflictingComponent`]. Never populated by the
    /// plain [`Self::set`] (`grid_to_component_tensor` and every other
    /// in-crate caller), so it stays empty -- and free -- outside of
    /// parsing a user declaration.
    declared_from: FxHashMap<IndexTuple, (IndexTuple, Expr)>,
}

impl ComponentTensor {
    /// An all-zero tensor of `head`'s declared shape and symmetry.
    pub fn new(head: HeadId) -> Self {
        ComponentTensor { head, independent: FxHashMap::default(), declared_from: FxHashMap::default() }
    }

    pub fn head(&self) -> HeadId {
        self.head
    }

    fn check_arity(&self, registry: &Registry, indices: &[u8]) -> Result<(), ComponentError> {
        let expected = registry.head(self.head).arity();
        if indices.len() != expected {
            return Err(ComponentError::ArityMismatch { expected, found: indices.len() });
        }
        Ok(())
    }

    /// Sets the component at `indices`; every other component in its
    /// symmetry orbit is implied. `value` is `T(indices)`, not the value
    /// stored for the orbit's representative -- those differ by the
    /// orbit's sign, accounted for here.
    pub fn set(
        &mut self,
        registry: &Registry,
        indices: &[u8],
        value: Expr,
    ) -> Result<(), ComponentError> {
        self.check_arity(registry, indices)?;
        let bsgs = &registry.head(self.head).symmetry;
        match canonical_indices(bsgs, indices) {
            Orbit::Zero => Ok(()), // T(indices) is forced to 0 regardless of `value`.
            Orbit::Representative(rep, sign) => {
                // T(indices) = sign * T(rep)  =>  T(rep) = sign * T(indices).
                let scaled = if sign < 0 { -value } else { value };
                self.independent.insert(rep, scaled);
                Ok(())
            }
        }
    }

    /// Same as [`Self::set`], but for a component declared directly by a
    /// user (today, exactly one call site: `oderom-cli`'s `metric`
    /// block parser) rather than computed internally: if `indices`'
    /// orbit was already given a *different* value by an earlier
    /// `set_declared` call -- the `[t,phi] = X` / `[phi,t] = Y`,
    /// `X != Y` case -- returns [`ComponentError::ConflictingComponent`]
    /// naming both declarations exactly as written, instead of silently
    /// keeping whichever was declared last. A second declaration that
    /// agrees with the first (same value, accounting for the orbit's
    /// sign) is not an error -- redeclaring the same component is not
    /// the mistake this guards against.
    ///
    /// Not the function every internal caller should switch to:
    /// `grid_to_component_tensor` writes every raw index tuple of an
    /// already-consistent `Grid` (both `[i,j]` and `[j,i]` of a
    /// symmetric grid, deliberately, every time), which would make the
    /// comparison below pure overhead on a hot path -- see that
    /// function's own doc comment. It keeps using the plain [`Self::set`].
    pub fn set_declared(
        &mut self,
        registry: &Registry,
        indices: &[u8],
        value: Expr,
    ) -> Result<(), ComponentError> {
        self.check_arity(registry, indices)?;
        let bsgs = &registry.head(self.head).symmetry;
        match canonical_indices(bsgs, indices) {
            Orbit::Zero => Ok(()),
            Orbit::Representative(rep, sign) => {
                let scaled = if sign < 0 { -value.clone() } else { value.clone() };
                if let Some((prev_indices, prev_value)) = self.declared_from.get(&rep) {
                    let existing = self.independent.get(&rep).cloned().unwrap_or_else(Expr::zero);
                    if scaled != existing {
                        return Err(ComponentError::ConflictingComponent {
                            first_indices: prev_indices.to_vec(),
                            first_value: prev_value.clone(),
                            second_indices: indices.to_vec(),
                            second_value: value,
                        });
                    }
                    return Ok(());
                }
                self.declared_from.insert(rep.clone(), (IndexTuple::from_slice(indices), value));
                self.independent.insert(rep, scaled);
                Ok(())
            }
        }
    }

    /// The component at `indices`; zero if its orbit was never set.
    pub fn get(&self, registry: &Registry, indices: &[u8]) -> Result<Expr, ComponentError> {
        self.check_arity(registry, indices)?;
        let bsgs = &registry.head(self.head).symmetry;
        Ok(match canonical_indices(bsgs, indices) {
            Orbit::Zero => Expr::zero(),
            Orbit::Representative(rep, sign) => {
                let base = self.independent.get(&rep).cloned().unwrap_or_else(Expr::zero);
                if sign < 0 {
                    -base
                } else {
                    base
                }
            }
        })
    }

    /// Number of stored orbit representatives (i.e. of *independent*
    /// components) -- not the number of raw index tuples.
    pub fn independent_len(&self) -> usize {
        self.independent.len()
    }

    /// A hash of `self`'s content, stable regardless of the order its
    /// independent components were `set` in -- same reasoning and same
    /// caveats as [`crate::grid::Grid::canonical_hash`] (sorts by index
    /// first; `FxHashMap` itself has no usable `Hash`). `head` (a
    /// `HeadId`, an index into whichever `Registry` happens to own it,
    /// never stable across two separately-built `Registry`s) is
    /// deliberately excluded -- this hash is meant to answer "does this
    /// tensor's *content* match another's", including across two
    /// `Model`s from two different `evaluate_definitions` calls, where
    /// comparing raw `HeadId`s would be comparing unrelated numbers.
    pub fn canonical_hash(&self) -> u64 {
        let mut entries: Vec<(&IndexTuple, &Expr)> = self.independent.iter().collect();
        entries.sort_by(|(a, _), (b, _)| a.as_slice().cmp(b.as_slice()));
        let mut hasher = FxHasher::default();
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
    use oderom_core::{Perm, SlotSig, Variance};

    /// A symmetric rank-2 head (same shape a `metric` block declares),
    /// deliberately not the full Schwarzschild/RN fixture setup
    /// elsewhere in this crate -- this test only needs somewhere to
    /// `set` two components and doesn't care what they mean physically.
    fn symmetric_rank2_head() -> (Registry, HeadId) {
        let mut registry = Registry::new();
        let manifold = registry.declare_manifold("M", 2).unwrap();
        let tm = registry.declare_bundle("TM", manifold, 2).unwrap();
        let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 2 };
        let slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
        let head = registry.declare_head("g", slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();
        (registry, head)
    }

    #[test]
    fn canonical_hash_is_independent_of_insertion_order() {
        let (registry, head) = symmetric_rank2_head();
        let mut forward = ComponentTensor::new(head);
        forward.set(&registry, &[0, 0], Expr::int(1)).unwrap();
        forward.set(&registry, &[1, 1], Expr::int(2)).unwrap();

        let mut backward = ComponentTensor::new(head);
        backward.set(&registry, &[1, 1], Expr::int(2)).unwrap();
        backward.set(&registry, &[0, 0], Expr::int(1)).unwrap();

        assert_eq!(forward.canonical_hash(), backward.canonical_hash());
    }

    #[test]
    fn canonical_hash_differs_for_different_content() {
        let (registry, head) = symmetric_rank2_head();
        let mut a = ComponentTensor::new(head);
        a.set(&registry, &[0, 0], Expr::int(1)).unwrap();
        let mut b = ComponentTensor::new(head);
        b.set(&registry, &[0, 0], Expr::int(2)).unwrap();
        assert_ne!(a.canonical_hash(), b.canonical_hash());
    }
}
