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
use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

type IndexTuple = SmallVec<[u8; 4]>;

enum Orbit {
    /// The symmetry group forces every component in this orbit to equal
    /// its own negative.
    Zero,
    Representative(IndexTuple, i8),
}

fn canonical_indices(bsgs: &Bsgs, indices: &[u8]) -> Orbit {
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
}

impl ComponentTensor {
    /// An all-zero tensor of `head`'s declared shape and symmetry.
    pub fn new(head: HeadId) -> Self {
        ComponentTensor { head, independent: FxHashMap::default() }
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
}
