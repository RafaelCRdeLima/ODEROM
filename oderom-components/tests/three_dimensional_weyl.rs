//! Marco 6 step 6's other dimension-barrier case: unlike 2D (where the
//! Weyl tensor's formula is outright undefined), the Weyl tensor at
//! `n=3` is well-defined and identically zero -- a real theorem (in 3D,
//! the Riemann tensor has exactly as many independent components as the
//! Ricci tensor, so Ricci already determines all of Riemann and nothing
//! "purely conformal" is left over). This is verified here against a
//! genuinely non-maximally-symmetric 3D metric (a static, spherically
//! symmetric "3D Schwarzschild" shape: coordinates `t, r, theta`, only
//! one rotational Killing vector, not the three a maximally symmetric
//! 3-space would have) -- a stronger, more generic exercise of the
//! theorem than a maximally symmetric 3-space (e.g. the round 3-sphere)
//! would be, since a maximally symmetric space is conformally flat "for
//! free" and would make the theorem's content harder to distinguish
//! from a coincidence of high symmetry.

use oderom_components::curvature::{christoffel, lower_first_index, metric_inverse_diagonal, ricci_scalar, ricci_tensor, riemann_mixed, weyl_tensor};
use oderom_components::{Chart, ComponentTensor};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;

#[test]
fn weyl_of_a_generic_3d_metric_is_identically_zero() {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("M3", 3).unwrap();
    let tm = registry.declare_bundle("TM3", manifold, 3).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 3 };
    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry.declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();

    let chart = Chart::new(["t", "r", "theta"]);
    let m = Expr::var("M");
    let r = Expr::var("r");

    // f = 1 - 2M/r -- the same static, spherically symmetric shape as
    // 4D Schwarzschild, just one angular coordinate short of it, so
    // this is genuinely curved and genuinely NOT maximally symmetric
    // (only one rotational Killing vector survives in 3D with a single
    // angle), not a disguised constant-curvature space.
    let f = Expr::one() - Expr::int(2) * m * r.clone().pow(-1);

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&(Expr::int(-1) * f.clone()))).unwrap();
    g.set(&registry, &[1, 1], normalize(&f.pow(-1))).unwrap();
    g.set(&registry, &[2, 2], normalize(&r.pow(2))).unwrap();

    let ginv = metric_inverse_diagonal(&registry, &chart, &g).unwrap();
    let gamma = christoffel(&registry, &chart, &g, &ginv).unwrap();
    let riem_mixed = riemann_mixed(&chart, &gamma);
    let riem_cov = lower_first_index(&registry, &chart, &riem_mixed, &g).unwrap();
    let ricci = ricci_tensor(&chart, &riem_mixed);
    let scalar = ricci_scalar(&chart, &ricci, &ginv);

    // Confirm this metric is genuinely curved (Ricci and Riemann both
    // real) before treating a zero Weyl tensor as the theorem at work
    // rather than a vacuous all-zero-because-everything-is-zero case --
    // a 3D vacuum solution (Ricci=0) would make Riemann itself vanish
    // too (the same 3D theorem, one step further back), which would
    // prove nothing about Weyl specifically.
    assert!(!normalize(&ricci.get(&[0, 0])).is_zero(), "this metric must be genuinely curved for the test to mean anything");
    assert!(!normalize(&riem_cov.get(&[0, 1, 0, 1])).is_zero(), "Riemann itself must be nonzero here too");

    let weyl = weyl_tensor(&registry, &chart, &g, &riem_cov, &ricci, &scalar).expect("n=3 must NOT refuse -- only n=2 does");
    for a in 0..3u8 {
        for b in 0..3u8 {
            for c in 0..3u8 {
                for d in 0..3u8 {
                    let component = normalize(&weyl.get(&[a, b, c, d]));
                    assert!(component.is_zero(), "C_{{{a}{b}{c}{d}}} = {component:?}, expected 0 (Weyl vanishes identically in 3D)");
                }
            }
        }
    }
}
