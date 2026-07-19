//! Marco 4 acceptance test (DESIGN-M4.md, section 4): the first Bianchi
//! identity is a genuine multi-term relation among Riemann monomials
//! that no pure permutation-symmetry canonicalization (oderom-canon,
//! Marco 1) can capture -- Bianchi's cyclic permutation of three of
//! Riemann's four slots has order 3, and Riemann's own slot-symmetry
//! group has order 8, so by Lagrange's theorem it cannot be one of
//! Riemann's declared symmetries. Once the identity is registered with
//! the e-graph as an independent fact, `R[a,b,c,d] + R[a,c,d,b] +
//! R[a,d,b,c]` extracts to zero; without it, the same sum does not
//! reduce (three unrelated terms, none of them equal or opposite to any
//! other under Marco 1's canonicalizer).

use oderom_core::{
    AbstractIndex, Factor, HeadId, Matching, Monomial, Perm, Registry, Scalar, SignedPerm, SlotId,
    SlotSig, Variance,
};
use oderom_egraph::{apply_bianchi, extract, EGraph, ENode};
use smallvec::SmallVec;

fn riemann_registry() -> (Registry, HeadId) {
    let mut reg = Registry::new();
    let m = reg.declare_manifold("M", 4).unwrap();
    let tm = reg.declare_bundle("TM", m, 4).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };
    let slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co, co, co];
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
    let factors: SmallVec<[Factor; 4]> = smallvec::smallvec![Factor { head }];
    let free = (0..4)
        .map(|i| (SlotId { factor: 0, slot: i as u8 }, AbstractIndex::new(order[i])))
        .collect();
    Monomial::try_new(Scalar::ONE, factors, Matching::default(), free, registry).unwrap()
}

#[test]
fn first_bianchi_identity_zeroes_the_cyclic_sum() {
    let (registry, r) = riemann_registry();
    let m1 = riemann_free(r, &registry, ["a", "b", "c", "d"]);
    let m2 = riemann_free(r, &registry, ["a", "c", "d", "b"]);
    let m3 = riemann_free(r, &registry, ["a", "d", "b", "c"]);

    let mut with_bianchi = EGraph::new();
    let id1 = with_bianchi.add_monomial(&registry, &m1);
    let id2 = with_bianchi.add_monomial(&registry, &m2);
    let id3 = with_bianchi.add_monomial(&registry, &m3);
    let sum = with_bianchi.add(ENode::Sum(smallvec::smallvec![id1, id2, id3]));
    apply_bianchi(&mut with_bianchi, &registry, r);
    let reduced = extract(&mut with_bianchi, sum);
    assert!(reduced.terms.is_empty(), "with Bianchi: expected zero, got {} terms", reduced.terms.len());

    let mut without_bianchi = EGraph::new();
    let id1 = without_bianchi.add_monomial(&registry, &m1);
    let id2 = without_bianchi.add_monomial(&registry, &m2);
    let id3 = without_bianchi.add_monomial(&registry, &m3);
    let sum = without_bianchi.add(ENode::Sum(smallvec::smallvec![id1, id2, id3]));
    let unreduced = extract(&mut without_bianchi, sum);
    assert_eq!(unreduced.terms.len(), 3, "without Bianchi: the sum should not reduce");
}
