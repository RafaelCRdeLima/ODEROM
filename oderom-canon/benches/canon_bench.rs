//! Marco 1 performance acceptance criteria:
//! - degree-3 Riemann monomial, 6 dummies: < 5 ms
//! - degree-4 Riemann monomial, 8 dummies: < 50 ms
//!
//! Both benchmarks fully contract a cyclic chain of `k` Riemann factors
//! (`R[.. p q] R[q r ..] .. R[.. p ..]`, 0 free indices, `2k` dummies),
//! which is the worst case for the search: every factor shares a head
//! with every other, so the acting group includes the full `S_k` factor
//! permutation on top of each factor's own order-8 slot symmetry.

use criterion::{criterion_group, criterion_main, Criterion};
use oderom_canon::canonicalize;
use oderom_core::{Factor, HeadId, Matching, Monomial, Perm, Registry, Scalar, SignedPerm, SlotId, SlotSig, Variance};
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

/// A fully-contracted cyclic chain of `k` Riemann factors: factor `i`'s
/// slots 2,3 contract with factor `(i+1)%k`'s slots 0,1. `2k` dummy pairs,
/// no free indices.
fn cyclic_chain(k: usize, r: HeadId, registry: &Registry) -> Monomial {
    let factors: SmallVec<[Factor; 4]> = (0..k).map(|_| Factor { head: r }).collect();
    let mut pairs = Vec::with_capacity(2 * k);
    for i in 0..k {
        let next = (i + 1) % k;
        pairs.push((SlotId { factor: i as u16, slot: 2 }, SlotId { factor: next as u16, slot: 0 }));
        pairs.push((SlotId { factor: i as u16, slot: 3 }, SlotId { factor: next as u16, slot: 1 }));
    }
    let contractions = Matching::try_new(pairs).unwrap();
    Monomial::try_new(Scalar::ONE, factors, contractions, vec![], registry).unwrap()
}

fn bench_degree_3(c: &mut Criterion) {
    let (registry, r) = riemann_registry();
    let m = cyclic_chain(3, r, &registry);
    c.bench_function("canonicalize degree-3 Riemann (6 dummies)", |b| {
        b.iter(|| canonicalize(&m, &registry).unwrap())
    });
}

fn bench_degree_4(c: &mut Criterion) {
    let (registry, r) = riemann_registry();
    let m = cyclic_chain(4, r, &registry);
    c.bench_function("canonicalize degree-4 Riemann (8 dummies)", |b| {
        b.iter(|| canonicalize(&m, &registry).unwrap())
    });
}

criterion_group!(benches, bench_degree_3, bench_degree_4);
criterion_main!(benches);
