//! R0, item 10.4 of DESIGN-TENSOR-ALGEBRA.md: do the two halves of
//! ODEROM agree on the Riemann convention?
//!
//! The abstract-index half declares, in `prelude.od`:
//!
//! ```text
//! head R : TM*, TM*, TM*, TM* symmetry (1 2)- (3 4)- (1 3)(2 4)+
//! ```
//!
//! -- antisymmetric in the first pair, antisymmetric in the second, and
//! symmetric under exchanging the pairs. The components half computes
//! `R^a_bcd` from Christoffel symbols (`curvature.rs`, `riemann_mixed`)
//! and contracts `R_bd = R^a_{bad}`. Nothing checked that the tensor the
//! second half produces actually carries the symmetries the first half
//! declares, in that slot order.
//!
//! That gap matters beyond tidiness. The plan's R5 (Ricci identity) and
//! R7 (abstract -> components bridge) both assume the agreement; if it
//! does not hold, they produce a disagreement that reads as an algebra
//! bug and is a convention bug. Checking it is cheap, so it belongs here
//! rather than in R5, where it would be expensive to diagnose.
//!
//! De Sitter, not Schwarzschild, deliberately. Section 8 of the plan
//! warns that verifying an identity only on Ricci-flat metrics yields
//! false positives: any wrong claim that depends on `R_ab = 0` passes.
//! De Sitter has non-vanishing Ricci, so these symmetries are being
//! checked against a tensor whose contractions are not all zero.

use oderom_components::curvature::{christoffel, lower_first_index, metric_inverse_diagonal, riemann_mixed};
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;

/// De Sitter in flat slicing, `ds^2 = -dt^2 + e^{2Ht}(dx^2+dy^2+dz^2)`,
/// returning the fully covariant Riemann tensor.
fn de_sitter_riemann_covariant() -> Result<Grid, ComponentError> {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("M", 4).unwrap();
    let tm = registry.declare_bundle("TM", manifold, 4).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };
    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head =
        registry.declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();

    let chart = Chart::new(["t", "x", "y", "z"]);
    let scale = (Expr::int(2) * Expr::var("H") * Expr::var("t")).exp();

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&Expr::int(-1)))?;
    g.set(&registry, &[1, 1], normalize(&scale))?;
    g.set(&registry, &[2, 2], normalize(&scale.clone()))?;
    g.set(&registry, &[3, 3], normalize(&scale))?;

    let ginv = metric_inverse_diagonal(&registry, &chart, &g)?;
    let gamma = christoffel(&registry, &chart, &g, &ginv)?;
    let riem_mixed = riemann_mixed(&chart, &gamma);
    lower_first_index(&registry, &chart, &riem_mixed, &g)
}

/// `normalize(lhs - rhs)` is zero, reported with the offending indices.
fn assert_same(riem: &Grid, lhs: [u8; 4], rhs: [u8; 4], sign: i64, law: &str) {
    let diff = normalize(&(riem.get(&lhs) + Expr::int(-sign) * riem.get(&rhs)));
    assert!(
        diff.is_zero(),
        "{law} fails: R_{lhs:?} != {}R_{rhs:?}  (difference {diff:?})",
        if sign > 0 { "" } else { "-" }
    );
}

/// The three declared symmetries, checked on every component.
#[test]
fn components_riemann_carries_the_symmetries_the_prelude_declares() {
    let riem = de_sitter_riemann_covariant().expect("de Sitter Riemann");
    for a in 0..4u8 {
        for b in 0..4u8 {
            for c in 0..4u8 {
                for d in 0..4u8 {
                    // (1 2)-  : R_abcd = -R_bacd
                    assert_same(&riem, [a, b, c, d], [b, a, c, d], -1, "first-pair antisymmetry `(1 2)-`");
                    // (3 4)-  : R_abcd = -R_abdc
                    assert_same(&riem, [a, b, c, d], [a, b, d, c], -1, "second-pair antisymmetry `(3 4)-`");
                    // (1 3)(2 4)+ : R_abcd = R_cdab
                    assert_same(&riem, [a, b, c, d], [c, d, a, b], 1, "pair-exchange symmetry `(1 3)(2 4)+`");
                }
            }
        }
    }
}

/// The first Bianchi identity, which `--bianchi` lets a user *declare*
/// on the abstract side, must actually hold of the tensor the
/// components side computes -- in the same slot order.
///
/// This is the sharpest form of the convention question. `--bianchi`
/// cycles slots 1,2,3 holding slot 0 fixed; if the components half
/// ordered its indices differently, that cycle would be a false axiom,
/// and every `simplify --bianchi` result would be wrong in a way no
/// abstract-side test could detect.
#[test]
fn components_riemann_satisfies_the_cyclic_identity_that_bianchi_declares() {
    let riem = de_sitter_riemann_covariant().expect("de Sitter Riemann");
    for a in 0..4u8 {
        for b in 0..4u8 {
            for c in 0..4u8 {
                for d in 0..4u8 {
                    let cyclic =
                        normalize(&(riem.get(&[a, b, c, d]) + riem.get(&[a, c, d, b]) + riem.get(&[a, d, b, c])));
                    assert!(
                        cyclic.is_zero(),
                        "R_{a}{b}{c}{d} + R_{a}{c}{d}{b} + R_{a}{d}{b}{c} = {cyclic:?}, expected 0 -- \
                         the components half does not satisfy the cycle `--bianchi` declares"
                    );
                }
            }
        }
    }
}

/// De Sitter's Ricci is non-zero, so the checks above are not passing
/// vacuously on a tensor whose contractions all vanish. Without this,
/// a Ricci-flat fixture could make every symmetry hold for the wrong
/// reason -- the false-positive mode section 8 of the plan warns about.
#[test]
fn the_fixture_is_not_ricci_flat() {
    let riem = de_sitter_riemann_covariant().expect("de Sitter Riemann");
    let nonzero = (0..4u8)
        .flat_map(|a| (0..4u8).map(move |b| (a, b)))
        .flat_map(|(a, b)| (0..4u8).map(move |c| (a, b, c)))
        .flat_map(|(a, b, c)| (0..4u8).map(move |d| (a, b, c, d)))
        .any(|(a, b, c, d)| !normalize(&riem.get(&[a, b, c, d])).is_zero());
    assert!(nonzero, "de Sitter Riemann came out identically zero -- the fixture is not exercising anything");
}
