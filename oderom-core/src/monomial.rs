//! The contraction graph: [`Monomial`] and its pieces.
//!
//! A dummy index is **not a name** -- it is an edge of [`Matching`] between
//! two [`SlotId`]s. There is deliberately no operation to rename a dummy
//! index anywhere in this module: if one seems necessary, the
//! representation is being used incorrectly. Only *free* indices carry a
//! name ([`AbstractIndex`]), because they must be matched across every term
//! of a [`Polynomial`].

use crate::error::CoreError;
use crate::head::HeadId;
use crate::registry::Registry;
use crate::scalar::Scalar;
use smallvec::SmallVec;
use std::collections::HashMap;

/// A slot is identified by which factor it belongs to (position within the
/// `Monomial`'s factor list, not a global id) and which slot of that
/// factor's head it is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SlotId {
    pub factor: u16,
    pub slot: u8,
}

/// One occurrence of a [`crate::head::TensorHead`] within a [`Monomial`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Factor {
    pub head: HeadId,
}

/// A perfect matching on a subset of slots: the graph of dummy-index
/// contractions. Stored with each pair oriented by [`SlotId`]'s `Ord` and
/// the pair list sorted, so that two `Matching`s describing the same set of
/// unordered contractions compare equal -- there is no meaningful
/// distinction between "edge (u,v)" and "edge (v,u)" at this layer.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Matching {
    pairs: Vec<(SlotId, SlotId)>,
}

impl Matching {
    /// Builds a `Matching` from an arbitrary list of pairs, normalizing
    /// orientation and order. Rejects a slot contracted with itself.
    pub fn try_new(pairs: impl IntoIterator<Item = (SlotId, SlotId)>) -> Result<Self, CoreError> {
        let mut normalized: Vec<(SlotId, SlotId)> = pairs
            .into_iter()
            .map(|(a, b)| {
                if a == b {
                    Err(CoreError::SlotUsedTwice(a))
                } else if a < b {
                    Ok((a, b))
                } else {
                    Ok((b, a))
                }
            })
            .collect::<Result<_, _>>()?;
        normalized.sort();
        Ok(Matching { pairs: normalized })
    }

    /// The contracted pairs, each oriented `(min, max)` by [`SlotId`]'s
    /// `Ord` and the whole list sorted -- see the struct docs.
    pub fn pairs(&self) -> &[(SlotId, SlotId)] {
        &self.pairs
    }

    /// Number of contracted pairs.
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}

/// The user-facing name of a *free* index. Dummy indices never get one --
/// see the module docs.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct AbstractIndex(String);

impl AbstractIndex {
    pub fn new(name: impl Into<String>) -> Self {
        AbstractIndex(name.into())
    }

    pub fn name(&self) -> &str {
        &self.0
    }
}

/// A single term: a coefficient, a list of tensor-head factors, the
/// contraction graph among their slots, and the free (uncontracted) slots
/// with the labels the user gave them.
#[derive(Clone, Debug)]
pub struct Monomial {
    coeff: Scalar,
    factors: SmallVec<[Factor; 4]>,
    contractions: Matching,
    free: Vec<(SlotId, AbstractIndex)>,
}

impl Monomial {
    /// Validates and builds a `Monomial`:
    /// - every `SlotId` referenced (in `contractions` or `free`) names an
    ///   existing factor and a slot within that factor head's arity;
    /// - every slot of every factor is used exactly once, either in a
    ///   contraction or as a free index;
    /// - no two free slots share the same label.
    ///
    /// This is purely structural: it does not check bundle/variance
    /// compatibility of contracted slots (that is a type judgment, made in
    /// `oderom-types`, which alone knows what "compatible" means).
    pub fn try_new(
        coeff: Scalar,
        factors: SmallVec<[Factor; 4]>,
        contractions: Matching,
        free: Vec<(SlotId, AbstractIndex)>,
        registry: &Registry,
    ) -> Result<Self, CoreError> {
        let arities: Vec<usize> =
            factors.iter().map(|f| registry.head(f.head).arity()).collect();
        let mut used: Vec<Vec<bool>> = arities.iter().map(|&a| vec![false; a]).collect();

        let mark = |slot: SlotId, used: &mut Vec<Vec<bool>>| -> Result<(), CoreError> {
            let f = slot.factor as usize;
            let head_arity = *arities.get(f).ok_or(CoreError::SlotOutOfRange {
                factor: f,
                head: "<out of range>".to_string(),
                arity: 0,
                slot: slot.slot as usize,
            })?;
            if slot.slot as usize >= head_arity {
                return Err(CoreError::SlotOutOfRange {
                    factor: f,
                    head: registry.head(factors[f].head).name.clone(),
                    arity: head_arity,
                    slot: slot.slot as usize,
                });
            }
            if used[f][slot.slot as usize] {
                return Err(CoreError::SlotUsedTwice(slot));
            }
            used[f][slot.slot as usize] = true;
            Ok(())
        };

        for &(a, b) in contractions.pairs() {
            mark(a, &mut used)?;
            mark(b, &mut used)?;
        }

        let mut seen_labels: HashMap<&str, SlotId> = HashMap::new();
        for (slot, label) in &free {
            mark(*slot, &mut used)?;
            if let Some(&prior) = seen_labels.get(label.name()) {
                return Err(CoreError::DuplicateFreeLabel(prior, *slot));
            }
            seen_labels.insert(label.name(), *slot);
        }

        for (f, slots) in used.iter().enumerate() {
            for (s, &is_used) in slots.iter().enumerate() {
                if !is_used {
                    return Err(CoreError::UnmatchedSlot(SlotId { factor: f as u16, slot: s as u8 }));
                }
            }
        }

        Ok(Monomial { coeff, factors, contractions, free })
    }

    pub fn coeff(&self) -> Scalar {
        self.coeff
    }

    pub fn factors(&self) -> &[Factor] {
        &self.factors
    }

    /// The dummy-index contraction graph.
    pub fn contractions(&self) -> &Matching {
        &self.contractions
    }

    /// The uncontracted slots and the labels the user gave them.
    pub fn free(&self) -> &[(SlotId, AbstractIndex)] {
        &self.free
    }
}

/// A sum of monomials. `oderom-core` does not enforce that every term has
/// the same free-index signature -- that is a type judgment, checked in
/// `oderom-types`.
#[derive(Clone, Debug)]
pub struct Polynomial {
    pub terms: Vec<Monomial>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::head::{SlotSig, Variance};
    use crate::perm::{Perm, SignedPerm};

    fn riemann_registry() -> (Registry, HeadId) {
        let mut reg = Registry::new();
        let m = reg.declare_manifold("M", 4).unwrap();
        let tm = reg.declare_bundle("TM", m, 4).unwrap();
        let slot = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };
        let slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![slot, slot, slot, slot];
        let pair_swap = SignedPerm::new(Perm::try_from_images(&[2, 3, 0, 1]).unwrap(), 1);
        let gens = vec![
            SignedPerm::new(Perm::transposition(4, 0, 1), -1),
            SignedPerm::new(Perm::transposition(4, 2, 3), -1),
            pair_swap,
        ];
        let head = reg.declare_head("R", slots, gens).unwrap();
        (reg, head)
    }

    #[test]
    fn matching_normalizes_pair_orientation() {
        let a = SlotId { factor: 0, slot: 0 };
        let b = SlotId { factor: 0, slot: 1 };
        let m1 = Matching::try_new([(a, b)]).unwrap();
        let m2 = Matching::try_new([(b, a)]).unwrap();
        assert_eq!(m1, m2);
    }

    #[test]
    fn matching_rejects_self_loop() {
        let a = SlotId { factor: 0, slot: 0 };
        assert!(Matching::try_new([(a, a)]).is_err());
    }

    #[test]
    fn riemann_dd_free_monomial_is_well_formed() {
        // R[a,b,c,d] : all four slots free.
        let (reg, head) = riemann_registry();
        let factors: SmallVec<[Factor; 4]> = smallvec::smallvec![Factor { head }];
        let free = vec![
            (SlotId { factor: 0, slot: 0 }, AbstractIndex::new("a")),
            (SlotId { factor: 0, slot: 1 }, AbstractIndex::new("b")),
            (SlotId { factor: 0, slot: 2 }, AbstractIndex::new("c")),
            (SlotId { factor: 0, slot: 3 }, AbstractIndex::new("d")),
        ];
        let m = Monomial::try_new(Scalar::ONE, factors, Matching::default(), free, &reg).unwrap();
        assert_eq!(m.free().len(), 4);
    }

    #[test]
    fn unmatched_slot_is_rejected() {
        let (reg, head) = riemann_registry();
        let factors: SmallVec<[Factor; 4]> = smallvec::smallvec![Factor { head }];
        // only 3 of 4 slots covered
        let free = vec![
            (SlotId { factor: 0, slot: 0 }, AbstractIndex::new("a")),
            (SlotId { factor: 0, slot: 1 }, AbstractIndex::new("b")),
            (SlotId { factor: 0, slot: 2 }, AbstractIndex::new("c")),
        ];
        let err = Monomial::try_new(Scalar::ONE, factors, Matching::default(), free, &reg)
            .unwrap_err();
        assert!(matches!(err, CoreError::UnmatchedSlot(_)));
    }

    #[test]
    fn slot_out_of_range_is_rejected() {
        let (reg, head) = riemann_registry();
        let factors: SmallVec<[Factor; 4]> = smallvec::smallvec![Factor { head }];
        let free = vec![(SlotId { factor: 0, slot: 9 }, AbstractIndex::new("a"))];
        let err = Monomial::try_new(Scalar::ONE, factors, Matching::default(), free, &reg)
            .unwrap_err();
        assert!(matches!(err, CoreError::SlotOutOfRange { .. }));
    }

    #[test]
    fn duplicate_free_label_is_rejected() {
        let (reg, head) = riemann_registry();
        let factors: SmallVec<[Factor; 4]> = smallvec::smallvec![Factor { head }];
        let free = vec![
            (SlotId { factor: 0, slot: 0 }, AbstractIndex::new("a")),
            (SlotId { factor: 0, slot: 1 }, AbstractIndex::new("a")),
            (SlotId { factor: 0, slot: 2 }, AbstractIndex::new("c")),
            (SlotId { factor: 0, slot: 3 }, AbstractIndex::new("d")),
        ];
        let err = Monomial::try_new(Scalar::ONE, factors, Matching::default(), free, &reg)
            .unwrap_err();
        assert!(matches!(err, CoreError::DuplicateFreeLabel(_, _)));
    }

    #[test]
    fn riemann_aba_b_contracted_monomial_is_well_formed() {
        // R[a,b,a,b]: slots 0-2 and 1-3 contracted (both mudos).
        let (reg, head) = riemann_registry();
        let factors: SmallVec<[Factor; 4]> = smallvec::smallvec![Factor { head }];
        let contractions = Matching::try_new([
            (SlotId { factor: 0, slot: 0 }, SlotId { factor: 0, slot: 2 }),
            (SlotId { factor: 0, slot: 1 }, SlotId { factor: 0, slot: 3 }),
        ])
        .unwrap();
        let m = Monomial::try_new(Scalar::ONE, factors, contractions, vec![], &reg).unwrap();
        assert_eq!(m.contractions().len(), 2);
        assert!(m.free().is_empty());
    }
}
