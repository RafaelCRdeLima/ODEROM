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
/// Marco 2 per DESIGN.md, not a permutation symmetry a coset search can
/// find (it would need to change the term's factor count). Confirmed with
/// the user 2026-07-19; left `#[ignore]`d rather than special-cased.
#[test]
#[ignore = "requires metric-contraction substitution, out of Marco 1 scope per DESIGN.md; confirmed with user"]
fn riemann_contracted_through_explicit_metric_reduces_like_direct_contraction() {
    unimplemented!()
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
