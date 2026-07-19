//! The first Bianchi identity, `R_{a[bcd]} = 0`, i.e.
//! `R[a,b,c,d] + R[a,c,d,b] + R[a,d,b,c] = 0`. Asserted directly rather
//! than declared through any general "user-defined multi-term identity"
//! mechanism -- Marco 4 doesn't have one (see DESIGN-M4.md, D4.2).
//! `apply_bianchi` scans the e-graph for bare, all-free-index monomials
//! of the given Riemann head and, for each, injects its own instance of
//! the identity.

use crate::egraph::{EGraph, ENode};
use oderom_core::{AbstractIndex, Factor, HeadId, Matching, Monomial, Registry, SlotId};
use smallvec::{smallvec, SmallVec};

/// For every e-class holding a bare (uncontracted, fully free-indexed)
/// monomial of `riemann_head`, injects the first Bianchi identity: that
/// monomial's e-class, plus the e-classes of its two cyclic siblings
/// (permuting slots 1,2,3 -- "b,c,d" -- and fixing slot 0 -- "a"), sums
/// to zero.
pub fn apply_bianchi(egraph: &mut EGraph, registry: &Registry, riemann_head: HeadId) {
    let candidates: Vec<Monomial> = egraph
        .classes()
        .flat_map(|(_, nodes)| nodes.iter())
        .filter_map(|node| match node {
            ENode::Term(m) if is_bare_riemann(m, riemann_head) => Some(m.clone()),
            _ => None,
        })
        .collect();

    for m in candidates {
        let labels: Vec<AbstractIndex> = m.free().iter().map(|(_, l)| l.clone()).collect();
        let cyclic1 = permute_free_indices(&m, registry, &labels, [0, 2, 3, 1]);
        let cyclic2 = permute_free_indices(&m, registry, &labels, [0, 3, 1, 2]);

        let id0 = egraph.add_monomial(registry, &m);
        let id1 = egraph.add_monomial(registry, &cyclic1);
        let id2 = egraph.add_monomial(registry, &cyclic2);

        let sum: SmallVec<[_; 4]> = smallvec![id0, id1, id2];
        let sum_class = egraph.add(ENode::Sum(sum));
        let zero = egraph.zero();
        egraph.union(sum_class, zero);
    }
    egraph.rebuild();
}

fn is_bare_riemann(m: &Monomial, riemann_head: HeadId) -> bool {
    m.factors().len() == 1
        && m.factors()[0].head == riemann_head
        && m.free().len() == 4
        && m.contractions().is_empty()
}

/// Rebuilds `m` with its free labels reassigned to slots according to
/// `order` (`order[i]` says which of `m`'s current labels, by position,
/// ends up at slot `i`) -- same head, same arity, so this cannot fail.
fn permute_free_indices(
    m: &Monomial,
    registry: &Registry,
    labels: &[AbstractIndex],
    order: [usize; 4],
) -> Monomial {
    let factors: SmallVec<[Factor; 4]> = m.factors().iter().copied().collect();
    let free = (0..4)
        .map(|i| (SlotId { factor: 0, slot: i as u8 }, labels[order[i]].clone()))
        .collect();
    Monomial::try_new(m.coeff(), factors, Matching::default(), free, registry)
        .expect("same head/arity/free-label-set as `m`, which already validated")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract;
    use oderom_core::{Perm, Scalar, SignedPerm, SlotSig, Variance};

    fn riemann_registry() -> (Registry, HeadId) {
        let mut reg = Registry::new();
        let m = reg.declare_manifold("M", 4).unwrap();
        let tm = reg.declare_bundle("TM", m, 4).unwrap();
        let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };
        let slots: SmallVec<[SlotSig; 4]> = smallvec![co, co, co, co];
        let pair_swap = SignedPerm::new(Perm::try_from_images(&[2, 3, 0, 1]).unwrap(), 1);
        let gens = vec![
            SignedPerm::new(Perm::transposition(4, 0, 1), -1),
            SignedPerm::new(Perm::transposition(4, 2, 3), -1),
            pair_swap,
        ];
        let r = reg.declare_head("R", slots, gens).unwrap();
        (reg, r)
    }

    fn riemann_free(head: HeadId, registry: &Registry, order: [&str; 4]) -> Monomial {
        let factors: SmallVec<[Factor; 4]> = smallvec![Factor { head }];
        let free = (0..4)
            .map(|i| (SlotId { factor: 0, slot: i as u8 }, AbstractIndex::new(order[i])))
            .collect();
        Monomial::try_new(Scalar::ONE, factors, Matching::default(), free, registry).unwrap()
    }

    #[test]
    fn bianchi_sum_extracts_to_zero_once_asserted() {
        let (registry, r) = riemann_registry();
        let m1 = riemann_free(r, &registry, ["a", "b", "c", "d"]);
        let m2 = riemann_free(r, &registry, ["a", "c", "d", "b"]);
        let m3 = riemann_free(r, &registry, ["a", "d", "b", "c"]);

        let mut eg = EGraph::new();
        let id1 = eg.add_monomial(&registry, &m1);
        let id2 = eg.add_monomial(&registry, &m2);
        let id3 = eg.add_monomial(&registry, &m3);
        let sum = eg.add(ENode::Sum(smallvec![id1, id2, id3]));

        apply_bianchi(&mut eg, &registry, r);

        let result = extract(&mut eg, sum);
        assert!(result.terms.is_empty(), "expected zero, got {:?} terms", result.terms.len());
    }

    #[test]
    fn without_bianchi_the_same_sum_does_not_reduce() {
        let (registry, r) = riemann_registry();
        let m1 = riemann_free(r, &registry, ["a", "b", "c", "d"]);
        let m2 = riemann_free(r, &registry, ["a", "c", "d", "b"]);
        let m3 = riemann_free(r, &registry, ["a", "d", "b", "c"]);

        let mut eg = EGraph::new();
        let id1 = eg.add_monomial(&registry, &m1);
        let id2 = eg.add_monomial(&registry, &m2);
        let id3 = eg.add_monomial(&registry, &m3);
        let sum = eg.add(ENode::Sum(smallvec![id1, id2, id3]));

        let result = extract(&mut eg, sum);
        assert_eq!(result.terms.len(), 3);
    }

    #[test]
    fn bianchi_does_not_touch_unrelated_monomials() {
        // A monomial that isn't a bare Riemann term (e.g. one with a
        // contraction) must be left alone: no spurious relation.
        let (registry, r) = riemann_registry();
        let contracted = {
            let factors: SmallVec<[Factor; 4]> = smallvec![Factor { head: r }];
            let contractions = Matching::try_new([
                (SlotId { factor: 0, slot: 0 }, SlotId { factor: 0, slot: 2 }),
                (SlotId { factor: 0, slot: 1 }, SlotId { factor: 0, slot: 3 }),
            ])
            .unwrap();
            Monomial::try_new(Scalar::ONE, factors, contractions, vec![], &registry).unwrap()
        };
        let mut eg = EGraph::new();
        let id = eg.add_monomial(&registry, &contracted);
        apply_bianchi(&mut eg, &registry, r);
        let result = extract(&mut eg, id);
        assert_eq!(result.terms.len(), 1);
    }
}
