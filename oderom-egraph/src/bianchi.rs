//! The two Bianchi identities:
//!
//! - first (algebraic), `R_{a[bcd]} = 0`, i.e.
//!   `R[a,b,c,d] + R[a,c,d,b] + R[a,d,b,c] = 0`;
//! - second (differential), `R_{ab[cd;e]} = 0`, i.e.
//!   `R[a,b,c,d;e] + R[a,b,d,e;c] + R[a,b,e,c;d] = 0`.
//!
//! Both are asserted directly rather than declared through any general
//! "user-defined multi-term identity" mechanism -- Marco 4 doesn't have
//! one (see DESIGN-M4.md, D4.2). Each scans the e-graph for bare,
//! all-free-index monomials of the given Riemann head and, for each,
//! injects its own instance of the identity.
//!
//! Structurally they are one rule: a three-term cyclic sum over a
//! window of slots, with the rest held fixed. They differ only in which
//! monomials qualify and where the window sits -- hence the shared
//! `inject_cyclic_sum_is_zero` below.

use crate::egraph::{EGraph, ENode};
use oderom_core::{AbstractIndex, Factor, HeadId, Matching, Monomial, Registry, SlotId};
use smallvec::{smallvec, SmallVec};

/// For every e-class holding a bare (uncontracted, fully free-indexed)
/// monomial of `riemann_head`, injects the first Bianchi identity: that
/// monomial's e-class, plus the e-classes of its two cyclic siblings
/// (permuting slots 1,2,3 -- "b,c,d" -- and fixing slot 0 -- "a"), sums
/// to zero.
pub fn apply_bianchi(egraph: &mut EGraph, registry: &Registry, riemann_head: HeadId) {
    let candidates = collect(egraph, |m| is_bare_riemann(m, riemann_head));
    // Fix slot 0 ("a"), cycle slots 1,2,3 ("b,c,d").
    inject_cyclic_sum_is_zero(egraph, registry, candidates, &[0, 2, 3, 1], &[0, 3, 1, 2]);
}

/// The second (differential) Bianchi identity, `R_{ab[cd;e]} = 0`, i.e.
/// `R[a,b,c,d;e] + R[a,b,d,e;c] + R[a,b,e,c;d] = 0`.
///
/// Same shape as the first, one fixed slot further along. `Registry`
/// synthesises the once-differentiated head with the derivative index
/// *last* (`registry.rs`, `derivative_head`), so on the rank-5 head
/// `R;1` the identity fixes slots 0,1 and cycles slots 2,3,4 -- the
/// last of which is the derivative index itself. That is what makes
/// this the differential identity rather than a second algebraic one:
/// the derivative slot participates in the cycle.
///
/// Takes the *base* Riemann head and finds its derivative head
/// structurally, so a caller never has to name `R;1`.
pub fn apply_second_bianchi(egraph: &mut EGraph, registry: &Registry, riemann_head: HeadId) {
    let candidates = collect(egraph, |m| is_bare_differentiated_riemann(m, registry, riemann_head));
    inject_cyclic_sum_is_zero(egraph, registry, candidates, &[0, 1, 3, 4, 2], &[0, 1, 4, 2, 3]);
}

fn collect(egraph: &EGraph, mut keep: impl FnMut(&Monomial) -> bool) -> Vec<Monomial> {
    egraph
        .classes()
        .flat_map(|(_, nodes)| nodes.iter())
        .filter_map(|node| match node {
            ENode::Term(m) if keep(m) => Some(m.clone()),
            _ => None,
        })
        .collect()
}

/// Asserts, for each candidate, that it plus its two `order`-permuted
/// siblings sums to zero. Shared by both Bianchi identities: they differ
/// only in which monomials qualify and which slots the cycle moves.
fn inject_cyclic_sum_is_zero(
    egraph: &mut EGraph,
    registry: &Registry,
    candidates: Vec<Monomial>,
    order1: &[usize],
    order2: &[usize],
) {
    for m in candidates {
        let labels: Vec<AbstractIndex> = m.free().iter().map(|(_, l)| l.clone()).collect();
        let cyclic1 = permute_free_indices(&m, registry, &labels, order1);
        let cyclic2 = permute_free_indices(&m, registry, &labels, order2);

        let id0 = egraph.add_monomial(registry, &m).0;
        let id1 = egraph.add_monomial(registry, &cyclic1).0;
        let id2 = egraph.add_monomial(registry, &cyclic2).0;

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

/// A lone, fully-free `R[...;e]`: exactly one factor, that factor a
/// once-differentiated Riemann, all five indices free and uncontracted.
///
/// `derivative_count() == 1` and not `>= 1`: the identity relates
/// first derivatives, and a second-derivative head has rank 6, for
/// which the slot cycle below would be the wrong permutation.
fn is_bare_differentiated_riemann(m: &Monomial, registry: &Registry, riemann_head: HeadId) -> bool {
    if m.factors().len() != 1 || m.free().len() != 5 || !m.contractions().is_empty() {
        return false;
    }
    let head = registry.head(m.factors()[0].head);
    head.derivative_count() == 1 && head.base_head() == riemann_head
}

/// Rebuilds `m` with its free labels reassigned to slots according to
/// `order` (`order[i]` says which of `m`'s current labels, by position,
/// ends up at slot `i`) -- same head, same arity, so this cannot fail.
///
/// Reading `m.free()` positionally is sound because `Monomial::try_new`
/// sorts `free` by `SlotId`, so position *is* slot order.
fn permute_free_indices(
    m: &Monomial,
    registry: &Registry,
    labels: &[AbstractIndex],
    order: &[usize],
) -> Monomial {
    let factors: SmallVec<[Factor; 4]> = m.factors().iter().copied().collect();
    let free = (0..order.len())
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
        let id1 = eg.add_monomial(&registry, &m1).0;
        let id2 = eg.add_monomial(&registry, &m2).0;
        let id3 = eg.add_monomial(&registry, &m3).0;
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
        let id1 = eg.add_monomial(&registry, &m1).0;
        let id2 = eg.add_monomial(&registry, &m2).0;
        let id3 = eg.add_monomial(&registry, &m3).0;
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
        let id = eg.add_monomial(&registry, &contracted).0;
        apply_bianchi(&mut eg, &registry, r);
        let result = extract(&mut eg, id);
        assert_eq!(result.terms.len(), 1);
    }

    /// `R[a,b,c,d;e]` with the labels placed at slots `order`, where the
    /// last entry is the derivative index.
    fn dr_free(base: HeadId, registry: &mut Registry, order: [&str; 5]) -> Monomial {
        let head = registry.derivative_head(base, 1).unwrap();
        let factors: SmallVec<[Factor; 4]> = smallvec![Factor { head }];
        let free = (0..5)
            .map(|i| (SlotId { factor: 0, slot: i as u8 }, AbstractIndex::new(order[i])))
            .collect();
        Monomial::try_new(Scalar::ONE, factors, Matching::default(), free, registry).unwrap()
    }

    #[test]
    fn second_bianchi_sum_extracts_to_zero_once_asserted() {
        let (mut registry, r) = riemann_registry();
        let m1 = dr_free(r, &mut registry, ["a", "b", "c", "d", "e"]);
        let m2 = dr_free(r, &mut registry, ["a", "b", "d", "e", "c"]);
        let m3 = dr_free(r, &mut registry, ["a", "b", "e", "c", "d"]);

        let mut eg = EGraph::new();
        let id1 = eg.add_monomial(&registry, &m1).0;
        let id2 = eg.add_monomial(&registry, &m2).0;
        let id3 = eg.add_monomial(&registry, &m3).0;
        let sum = eg.add(ENode::Sum(smallvec![id1, id2, id3]));

        apply_second_bianchi(&mut eg, &registry, r);

        let result = extract(&mut eg, sum);
        assert!(result.terms.is_empty(), "expected zero, got {} terms", result.terms.len());
    }

    #[test]
    fn without_second_bianchi_the_same_sum_does_not_reduce() {
        let (mut registry, r) = riemann_registry();
        let m1 = dr_free(r, &mut registry, ["a", "b", "c", "d", "e"]);
        let m2 = dr_free(r, &mut registry, ["a", "b", "d", "e", "c"]);
        let m3 = dr_free(r, &mut registry, ["a", "b", "e", "c", "d"]);

        let mut eg = EGraph::new();
        let id1 = eg.add_monomial(&registry, &m1).0;
        let id2 = eg.add_monomial(&registry, &m2).0;
        let id3 = eg.add_monomial(&registry, &m3).0;
        let sum = eg.add(ENode::Sum(smallvec![id1, id2, id3]));

        let result = extract(&mut eg, sum);
        assert_eq!(result.terms.len(), 3);
    }

    /// The two identities are declared separately and must stay
    /// separate: declaring the algebraic one says nothing about
    /// derivatives, and vice versa.
    #[test]
    fn the_two_bianchi_identities_do_not_substitute_for_each_other() {
        let (mut registry, r) = riemann_registry();
        let d1 = dr_free(r, &mut registry, ["a", "b", "c", "d", "e"]);
        let d2 = dr_free(r, &mut registry, ["a", "b", "d", "e", "c"]);
        let d3 = dr_free(r, &mut registry, ["a", "b", "e", "c", "d"]);

        // First identity declared, differential sum offered: no reduction.
        let mut eg = EGraph::new();
        let ids: SmallVec<[_; 4]> = smallvec![
            eg.add_monomial(&registry, &d1).0,
            eg.add_monomial(&registry, &d2).0,
            eg.add_monomial(&registry, &d3).0,
        ];
        let sum = eg.add(ENode::Sum(ids));
        apply_bianchi(&mut eg, &registry, r);
        assert_eq!(extract(&mut eg, sum).terms.len(), 3, "first Bianchi must not reduce a differential sum");

        // Second identity declared, algebraic sum offered: no reduction.
        let a1 = riemann_free(r, &registry, ["a", "b", "c", "d"]);
        let a2 = riemann_free(r, &registry, ["a", "c", "d", "b"]);
        let a3 = riemann_free(r, &registry, ["a", "d", "b", "c"]);
        let mut eg = EGraph::new();
        let ids: SmallVec<[_; 4]> = smallvec![
            eg.add_monomial(&registry, &a1).0,
            eg.add_monomial(&registry, &a2).0,
            eg.add_monomial(&registry, &a3).0,
        ];
        let sum = eg.add(ENode::Sum(ids));
        apply_second_bianchi(&mut eg, &registry, r);
        assert_eq!(extract(&mut eg, sum).terms.len(), 3, "second Bianchi must not reduce an algebraic sum");
    }

    /// A *second* derivative has rank 6, so the rank-5 slot cycle would
    /// be the wrong permutation for it. It must be left alone rather
    /// than related by a mis-shaped identity.
    #[test]
    fn second_bianchi_ignores_a_twice_differentiated_riemann() {
        let (mut registry, r) = riemann_registry();
        let head = registry.derivative_head(r, 2).unwrap();
        let factors: SmallVec<[Factor; 4]> = smallvec![Factor { head }];
        let free = (0..6)
            .map(|i| {
                let name = ["a", "b", "c", "d", "e", "f"][i];
                (SlotId { factor: 0, slot: i as u8 }, AbstractIndex::new(name))
            })
            .collect();
        let m = Monomial::try_new(Scalar::ONE, factors, Matching::default(), free, &registry).unwrap();

        let mut eg = EGraph::new();
        let id = eg.add_monomial(&registry, &m).0;
        apply_second_bianchi(&mut eg, &registry, r);
        assert_eq!(extract(&mut eg, id).terms.len(), 1);
    }
}
