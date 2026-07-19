//! Property test: for a random monomial `x` and a random element `g` of
//! its own declared symmetry group, `canon(g*x)` must be bit-for-bit
//! identical to `canon(x)`, up to the sign predicted from `g` alone. This
//! is the operational definition of "canonicalization is correct" for
//! Marco 1; everything else in the acceptance table is a corollary of it
//! holding.
//!
//! `g*x` is built by directly relabeling `x`'s slots according to `g`
//! (permutation only, coefficient left untouched) -- *not* by asking
//! `oderom-canon` to do it, since that would make the test check the
//! implementation against itself. The predicted sign is accumulated
//! independently, one declared-generator sign at a time, as each move is
//! applied.

use oderom_canon::{canonicalize, CanonResult};
use oderom_core::{
    AbstractIndex, Factor, HeadId, Matching, Monomial, Perm, Registry, Scalar, SignedPerm, SlotId,
    SlotSig, Variance,
};
use proptest::prelude::*;
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

fn free(factor: u16, slot: u8, name: &str) -> (SlotId, AbstractIndex) {
    (SlotId { factor, slot }, AbstractIndex::new(name))
}

/// Three representative templates: a fully-free factor, a self-contracted
/// factor, and two cross-contracted factors -- exercising `S` alone, `S`
/// with a nontrivial dummy pattern, and `S` combined with `P` (factor
/// exchange) respectively.
fn template(idx: u8, r: HeadId, registry: &Registry) -> (Monomial, u16) {
    match idx % 3 {
        0 => {
            let factors = smallvec::smallvec![Factor { head: r }];
            let free_idx = vec![free(0, 0, "a"), free(0, 1, "b"), free(0, 2, "c"), free(0, 3, "d")];
            (Monomial::try_new(Scalar::ONE, factors, Matching::default(), free_idx, registry).unwrap(), 1)
        }
        1 => {
            let factors = smallvec::smallvec![Factor { head: r }];
            let contractions = Matching::try_new([
                (SlotId { factor: 0, slot: 0 }, SlotId { factor: 0, slot: 2 }),
                (SlotId { factor: 0, slot: 1 }, SlotId { factor: 0, slot: 3 }),
            ])
            .unwrap();
            (Monomial::try_new(Scalar::ONE, factors, contractions, vec![], registry).unwrap(), 1)
        }
        _ => {
            let factors = smallvec::smallvec![Factor { head: r }, Factor { head: r }];
            let contractions = Matching::try_new([
                (SlotId { factor: 0, slot: 2 }, SlotId { factor: 1, slot: 0 }),
                (SlotId { factor: 0, slot: 3 }, SlotId { factor: 1, slot: 1 }),
            ])
            .unwrap();
            let free_idx = vec![free(0, 0, "e"), free(0, 1, "f"), free(1, 2, "g"), free(1, 3, "h")];
            (Monomial::try_new(Scalar::ONE, factors, contractions, free_idx, registry).unwrap(), 2)
        }
    }
}

fn remap(m: &Monomial, f: impl Fn(SlotId) -> SlotId, new_factors: SmallVec<[Factor; 4]>, registry: &Registry) -> Monomial {
    let free_idx = m.free().iter().map(|(s, l)| (f(*s), l.clone())).collect();
    let pairs: Vec<_> = m.contractions().pairs().iter().map(|&(a, b)| (f(a), f(b))).collect();
    Monomial::try_new(m.coeff(), new_factors, Matching::try_new(pairs).unwrap(), free_idx, registry).unwrap()
}

/// Applies one of `head.symmetry_generators[gen_idx % 3]` to `factor`'s
/// own slots. Returns the transformed monomial and that generator's sign.
fn apply_generator(m: &Monomial, factor: u16, gen_idx: usize, r: HeadId, registry: &Registry) -> (Monomial, i8) {
    let gens = &registry.head(r).symmetry_generators;
    let gen = &gens[gen_idx % gens.len()];
    let new = remap(
        m,
        |s| {
            if s.factor == factor {
                SlotId { factor: s.factor, slot: gen.perm.image(s.slot as u16) as u8 }
            } else {
                s
            }
        },
        m.factors().into(),
        registry,
    );
    (new, gen.sign)
}

/// Physically swaps two same-head factors (valid since tensor components
/// commute: sign +1).
fn swap_factors(m: &Monomial, i: u16, j: u16, registry: &Registry) -> Monomial {
    if i == j {
        return m.clone();
    }
    let mut factors: SmallVec<[Factor; 4]> = m.factors().into();
    factors.swap(i as usize, j as usize);
    let swap_idx = |f: u16| if f == i { j } else if f == j { i } else { f };
    remap(m, |s| SlotId { factor: swap_idx(s.factor), slot: s.slot }, factors, registry)
}

#[derive(Clone, Debug)]
enum Move {
    Gen(u16, usize),
    Swap(u16, u16),
}

fn moves_strategy() -> impl Strategy<Value = Vec<Move>> {
    let move_strategy = prop_oneof![
        (any::<u16>(), any::<usize>()).prop_map(|(f, g)| Move::Gen(f, g)),
        (any::<u16>(), any::<u16>()).prop_map(|(i, j)| Move::Swap(i, j)),
    ];
    prop::collection::vec(move_strategy, 0..8)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn canon_is_invariant_under_declared_symmetry(template_idx in any::<u8>(), moves in moves_strategy()) {
        let (registry, r) = riemann_registry();
        let (base, num_factors) = template(template_idx, r, &registry);

        let mut transformed = base.clone();
        let mut predicted_sign: i64 = 1;
        for mv in &moves {
            match *mv {
                Move::Gen(f, g) => {
                    let factor = f % num_factors;
                    let (new_m, sign) = apply_generator(&transformed, factor, g, r, &registry);
                    transformed = new_m;
                    predicted_sign *= sign as i64;
                }
                Move::Swap(i, j) => {
                    if num_factors > 1 {
                        transformed = swap_factors(&transformed, i % num_factors, j % num_factors, &registry);
                    }
                }
            }
        }

        let canon_base = canonicalize(&base, &registry).unwrap();
        let canon_transformed = canonicalize(&transformed, &registry).unwrap();

        match (canon_base, canon_transformed) {
            (CanonResult::Zero, CanonResult::Zero) => {}
            (CanonResult::Value(a), CanonResult::Value(b)) => {
                let mut fa: Vec<_> = a.monomial.free().iter().map(|(s, l)| (*s, l.clone())).collect();
                let mut fb: Vec<_> = b.monomial.free().iter().map(|(s, l)| (*s, l.clone())).collect();
                fa.sort_by_key(|(s, _)| *s);
                fb.sort_by_key(|(s, _)| *s);
                prop_assert_eq!(fa, fb);
                prop_assert_eq!(a.monomial.contractions(), b.monomial.contractions());
                prop_assert_eq!(a.monomial.coeff(), b.monomial.coeff() * Scalar::from_int(predicted_sign));
            }
            (a, b) => prop_assert!(false, "one side was Zero and the other wasn't: {:?} vs {:?}", a, b),
        }
    }
}
