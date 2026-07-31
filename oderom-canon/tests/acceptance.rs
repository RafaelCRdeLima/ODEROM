//! Marco 1 acceptance table, verbatim from DESIGN.md / the project brief.
//! Each `#[test]` here is named after the row it checks. The "Tipos" rows
//! are also exercised directly by `oderom-types`'s own suite (they only
//! need `oderom-core` + `oderom-types`); repeated here so this file is a
//! single, traceable checklist against the table.

use oderom_canon::{canonicalize, CanonResult};
use oderom_core::{
    AbstractIndex, Factor, Matching, Monomial, Perm, Registry, Scalar, SignedPerm, SlotId,
    SlotSig, Variance,
};
use oderom_types::{typecheck_monomial, typecheck_polynomial, TypeError};
use smallvec::SmallVec;

struct Prelude {
    registry: Registry,
    r: oderom_core::HeadId,
    eps: oderom_core::HeadId,
}

fn prelude() -> Prelude {
    let mut reg = Registry::new();
    let m = reg.declare_manifold("M", 4).unwrap();
    let tm = reg.declare_bundle("TM", m, 4).unwrap();
    let co = |dim| SlotSig { bundle: tm, variance: Variance::Co, dim };

    let r_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co(4), co(4), co(4), co(4)];
    let pair_swap = SignedPerm::new(Perm::try_from_images(&[2, 3, 0, 1]).unwrap(), 1);
    let r_gens = vec![
        SignedPerm::new(Perm::transposition(4, 0, 1), -1),
        SignedPerm::new(Perm::transposition(4, 2, 3), -1),
        pair_swap,
    ];
    let r = reg.declare_head("R", r_slots, r_gens).unwrap();

    let g_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co(4), co(4)];
    let g_gens = vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)];
    reg.declare_head("g", g_slots, g_gens).unwrap();

    let eps_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co(3), co(3), co(3)];
    let eps = reg.declare_head("eps", eps_slots, oderom_core::totally_antisymmetric_generators(3)).unwrap();

    Prelude { registry: reg, r, eps }
}

fn free(factor: u16, slot: u8, name: &str) -> (SlotId, AbstractIndex) {
    (SlotId { factor, slot }, AbstractIndex::new(name))
}

fn riemann(p: &Prelude, labels: [&str; 4]) -> Monomial {
    let factors = smallvec::smallvec![Factor { head: p.r }];
    let free_idx =
        vec![free(0, 0, labels[0]), free(0, 1, labels[1]), free(0, 2, labels[2]), free(0, 3, labels[3])];
    Monomial::try_new(Scalar::ONE, factors, Matching::default(), free_idx, &p.registry).unwrap()
}

fn expect_value(r: CanonResult) -> oderom_canon::Canonical {
    match r {
        CanonResult::Value(c) => c,
        CanonResult::Zero => panic!("expected a nonzero canonical form"),
    }
}

fn free_layout(m: &Monomial) -> Vec<(SlotId, AbstractIndex)> {
    let mut v: Vec<_> = m.free().iter().map(|(s, l)| (*s, l.clone())).collect();
    v.sort_by_key(|(s, _)| *s);
    v
}

#[test]
fn r_abcd_and_r_cdab_same_canonical_form_sign_plus_one() {
    let p = prelude();
    let a = expect_value(canonicalize(&riemann(&p, ["a", "b", "c", "d"]), &p.registry).unwrap());
    let b = expect_value(canonicalize(&riemann(&p, ["c", "d", "a", "b"]), &p.registry).unwrap());
    assert_eq!(free_layout(&a.monomial), free_layout(&b.monomial));
    assert_eq!(a.sign, 1);
    assert_eq!(b.sign, 1);
    assert_eq!(a.monomial.coeff(), b.monomial.coeff());
}

#[test]
fn r_abcd_and_r_bacd_same_canonical_form_opposite_sign() {
    let p = prelude();
    let a = expect_value(canonicalize(&riemann(&p, ["a", "b", "c", "d"]), &p.registry).unwrap());
    let b = expect_value(canonicalize(&riemann(&p, ["b", "a", "c", "d"]), &p.registry).unwrap());
    assert_eq!(free_layout(&a.monomial), free_layout(&b.monomial));
    assert_eq!(a.sign, -b.sign);
    assert_eq!(a.monomial.coeff(), -b.monomial.coeff());
}

#[test]
fn r_abab_and_r_cdcd_are_identical_dummies_are_edges_not_names() {
    let p = prelude();
    let build = |c: [(u8, u8); 2]| {
        let factors = smallvec::smallvec![Factor { head: p.r }];
        let contractions = Matching::try_new([
            (SlotId { factor: 0, slot: c[0].0 }, SlotId { factor: 0, slot: c[0].1 }),
            (SlotId { factor: 0, slot: c[1].0 }, SlotId { factor: 0, slot: c[1].1 }),
        ])
        .unwrap();
        Monomial::try_new(Scalar::ONE, factors, contractions, vec![], &p.registry).unwrap()
    };
    // R[a,b,a,b]: (slot0,slot2) and (slot1,slot3). R[c,d,c,d] has the exact
    // same slot-pairing -- there is no "c" or "d" anywhere in the graph.
    let m1 = build([(0, 2), (1, 3)]);
    let m2 = build([(0, 2), (1, 3)]);
    let a = expect_value(canonicalize(&m1, &p.registry).unwrap());
    let b = expect_value(canonicalize(&m2, &p.registry).unwrap());
    assert_eq!(a.monomial.contractions(), b.monomial.contractions());
    assert_eq!(a.monomial.coeff(), b.monomial.coeff());
}

/// `R[a,b,c,d] g[a,c] g[b,d]` reduces to `R[a,b,a,b]` only by substituting
/// through the metric (index lowering), which is explicit-metric algebra:
/// not a permutation symmetry a coset search can find, since it changes
/// the term's factor count. Left `#[ignore]`d through Marco 1 for exactly
/// that reason, and no longer ignored: `Monomial::eliminate_metric`
/// performs the substitution as contraction-graph surgery *before*
/// canonicalization, so the coset search still only ever does what it
/// can do. The metric head is declared by the caller, never inferred
/// from a rank-2 symmetric shape.
#[test]
fn riemann_contracted_through_explicit_metric_reduces_like_direct_contraction() {
    let p = prelude();
    let g = p.registry.lookup_head("g").unwrap();

    // R[a,b,c,d] g[a,c] g[b,d]: three factors, with every one of the
    // metrics' slots contracted into Riemann.
    let factors: SmallVec<[Factor; 4]> =
        smallvec::smallvec![Factor { head: p.r }, Factor { head: g }, Factor { head: g }];
    let contractions = Matching::try_new([
        (SlotId { factor: 0, slot: 0 }, SlotId { factor: 1, slot: 0 }), // a
        (SlotId { factor: 0, slot: 2 }, SlotId { factor: 1, slot: 1 }), // c
        (SlotId { factor: 0, slot: 1 }, SlotId { factor: 2, slot: 0 }), // b
        (SlotId { factor: 0, slot: 3 }, SlotId { factor: 2, slot: 1 }), // d
    ])
    .unwrap();
    let through_metric =
        Monomial::try_new(Scalar::ONE, factors, contractions, vec![], &p.registry).unwrap();

    // R[a,b,a,b]: the same tensor written with the contractions taken
    // directly, no metric factor at all.
    let direct_factors: SmallVec<[Factor; 4]> = smallvec::smallvec![Factor { head: p.r }];
    let direct_contractions = Matching::try_new([
        (SlotId { factor: 0, slot: 0 }, SlotId { factor: 0, slot: 2 }),
        (SlotId { factor: 0, slot: 1 }, SlotId { factor: 0, slot: 3 }),
    ])
    .unwrap();
    let direct =
        Monomial::try_new(Scalar::ONE, direct_factors, direct_contractions, vec![], &p.registry).unwrap();

    let eliminated = through_metric.eliminate_metric(g, &p.registry).unwrap();
    assert_eq!(eliminated.factors().len(), 1, "both metric factors should be gone");

    let a = expect_value(canonicalize(&eliminated, &p.registry).unwrap());
    let b = expect_value(canonicalize(&direct, &p.registry).unwrap());
    assert_eq!(a.monomial.contractions(), b.monomial.contractions());
    assert_eq!(a.monomial.coeff(), b.monomial.coeff());
    assert_eq!(a.sign, b.sign);
}

/// The lowering case on its own: `g[a,b] R[b,c,d,e]` is `R[a,c,d,e]` --
/// the metric renames a free index rather than contracting two slots
/// together. Separated from the test above because it exercises the
/// other branch of `eliminate_metric` (one slot contracted, one free).
#[test]
fn a_metric_with_one_free_slot_renames_the_index_it_lowers() {
    let p = prelude();
    let g = p.registry.lookup_head("g").unwrap();

    let factors: SmallVec<[Factor; 4]> = smallvec::smallvec![Factor { head: g }, Factor { head: p.r }];
    let contractions =
        Matching::try_new([(SlotId { factor: 0, slot: 1 }, SlotId { factor: 1, slot: 0 })]).unwrap();
    let free_idx = vec![
        free(0, 0, "a"),
        free(1, 1, "c"),
        free(1, 2, "d"),
        free(1, 3, "e"),
    ];
    let lowered = Monomial::try_new(Scalar::ONE, factors, contractions, free_idx, &p.registry)
        .unwrap()
        .eliminate_metric(g, &p.registry)
        .unwrap();

    assert_eq!(lowered.factors().len(), 1, "the metric factor should be gone");
    assert!(lowered.contractions().is_empty(), "nothing is contracted once the metric is removed");
    let mut labels: Vec<&str> = lowered.free().iter().map(|(_, l)| l.name()).collect();
    labels.sort_unstable();
    assert_eq!(labels, vec!["a", "c", "d", "e"], "the `a` label must have moved onto Riemann's first slot");
}

/// A metric that is not contracted with anything (`g[a,b]` standing
/// alone) has nothing to identify, so it must survive untouched -- the
/// negative control that keeps `eliminate_metric` from being a rule that
/// just deletes metrics.
#[test]
fn a_free_standing_metric_is_left_alone() {
    let p = prelude();
    let g = p.registry.lookup_head("g").unwrap();
    let factors: SmallVec<[Factor; 4]> = smallvec::smallvec![Factor { head: g }];
    let m = Monomial::try_new(
        Scalar::ONE,
        factors,
        Matching::default(),
        vec![free(0, 0, "a"), free(0, 1, "b")],
        &p.registry,
    )
    .unwrap();
    let after = m.eliminate_metric(g, &p.registry).unwrap();
    assert_eq!(after, m, "a bare g[a,b] has nothing to eliminate");
}

#[test]
fn epsilon_dot_symmetric_tensor_is_zero() {
    let p = prelude();
    let t_bundle = p.registry.lookup_bundle("TM").unwrap();
    let mut reg = p.registry;
    let t_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![
        SlotSig { bundle: t_bundle, variance: Variance::Co, dim: 4 },
        SlotSig { bundle: t_bundle, variance: Variance::Co, dim: 4 },
    ];
    let t = reg.declare_head("T", t_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();

    let factors = smallvec::smallvec![Factor { head: p.eps }, Factor { head: t }];
    let contractions = Matching::try_new([
        (SlotId { factor: 0, slot: 0 }, SlotId { factor: 1, slot: 0 }),
        (SlotId { factor: 0, slot: 1 }, SlotId { factor: 1, slot: 1 }),
    ])
    .unwrap();
    let free_idx = vec![free(0, 2, "c")];
    let m = Monomial::try_new(Scalar::ONE, factors, contractions, free_idx, &reg).unwrap();

    assert!(matches!(canonicalize(&m, &reg).unwrap(), CanonResult::Zero));
}

// -- "Tipos" rows (also covered directly in oderom-types) --------------

#[test]
fn contracting_tm_with_tm_is_a_type_error_naming_both_slots() {
    let p = prelude();
    // Two copies of a bare TM-vector head, contracted upper-with-upper.
    let tm = p.registry.lookup_bundle("TM").unwrap();
    let mut reg = p.registry;
    let v_slots: SmallVec<[SlotSig; 4]> =
        smallvec::smallvec![SlotSig { bundle: tm, variance: Variance::Contra, dim: 4 }];
    let v = reg.declare_head("V", v_slots, vec![]).unwrap();
    let factors = smallvec::smallvec![Factor { head: v }, Factor { head: v }];
    let contractions =
        Matching::try_new([(SlotId { factor: 0, slot: 0 }, SlotId { factor: 1, slot: 0 })]).unwrap();
    let m = Monomial::try_new(Scalar::ONE, factors, contractions, vec![], &reg).unwrap();

    let err = typecheck_monomial(0, &m, &reg).unwrap_err();
    assert!(matches!(err, TypeError::IncompatibleContraction { .. }));
}

#[test]
fn summing_terms_with_different_free_indices_is_a_type_error() {
    let p = prelude();
    let term_a = riemann(&p, ["a", "b", "c", "d"]);
    // second term reuses the same slot layout but a different free label set
    let factors = smallvec::smallvec![Factor { head: p.r }];
    let term_b = Monomial::try_new(
        Scalar::ONE,
        factors,
        Matching::default(),
        vec![free(0, 0, "w"), free(0, 1, "x"), free(0, 2, "y"), free(0, 3, "z")],
        &p.registry,
    )
    .unwrap();
    let poly = oderom_core::Polynomial { terms: vec![term_a, term_b] };
    let err = typecheck_polynomial(&poly, &p.registry).unwrap_err();
    assert!(matches!(err, TypeError::FreeIndexMismatch { .. }));
}
