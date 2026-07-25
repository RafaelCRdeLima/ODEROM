//! The round unit sphere `S^2` (spherical coordinates `theta, phi`, the
//! exact metric `ODEROM-manual.html`'s chapter 10 example 3 already
//! uses) as this project's standing 2D fixture, for two related edge
//! cases:
//!
//! - `einstein`'s own instructive one: nonzero Ricci curvature
//!   (`R_ab = g_ab`, `R = 2`, NOT vacuum), and yet the Einstein tensor
//!   still comes out identically zero -- a real fact about `G_ab`'s
//!   own definition in exactly 2D, not a property specific to the
//!   sphere.
//! - `weyl`'s mandatory dimension barrier (Marco 6 step 6): the Weyl
//!   tensor's standard formula is undefined at `n=2` (its `1/(n-2)`
//!   term), and this is where that has to actually refuse, not
//!   silently divide by zero or invent a component.
//!
//! Both verified computationally against this program's own
//! Ricci/scalar output, never assumed.

use oderom_components::curvature::{
    christoffel, einstein_tensor, metric_inverse_diagonal, ricci_scalar, ricci_tensor, riemann_mixed, weyl_squared, weyl_tensor,
};
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;

struct RoundSphere {
    registry: Registry,
    chart: Chart,
    g: ComponentTensor,
    ginv: Grid,
    ricci: Grid,
    scalar: Expr,
}

fn build() -> RoundSphere {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("S2", 2).unwrap();
    let tm = registry.declare_bundle("TS2", manifold, 2).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 2 };
    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry.declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();

    let chart = Chart::new(["theta", "phi"]);
    let theta = Expr::var("theta");

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], Expr::one()).unwrap();
    g.set(&registry, &[1, 1], normalize(&theta.clone().sin().pow(2))).unwrap();

    let ginv = metric_inverse_diagonal(&registry, &chart, &g).unwrap();
    let gamma = christoffel(&registry, &chart, &g, &ginv).unwrap();
    let riem_mixed = riemann_mixed(&chart, &gamma);
    let ricci = ricci_tensor(&chart, &riem_mixed);
    let scalar = ricci_scalar(&chart, &ricci, &ginv);

    // Confirm this really is the nonzero-Ricci case the manual's own
    // example 3 documents -- R_{theta,theta}=1, R_{phi,phi}=sin^2(theta),
    // R=2 -- before treating a zero Einstein tensor (or a Weyl refusal)
    // as meaningful rather than a vacuous all-zero-because-everything-
    // is-zero case.
    assert_eq!(normalize(&ricci.get(&[0, 0])), Expr::one());
    assert_eq!(normalize(&ricci.get(&[1, 1])), normalize(&theta.sin().pow(2)));
    assert_eq!(normalize(&scalar), Expr::int(2));

    RoundSphere { registry, chart, g, ginv, ricci, scalar }
}

#[test]
fn einstein_of_the_round_sphere_is_identically_zero_despite_nonzero_ricci() {
    let s = build();
    let einstein = einstein_tensor(&s.registry, &s.chart, &s.g, &s.ricci, &s.scalar);
    for a in 0..2u8 {
        for b in 0..2u8 {
            let component = normalize(&einstein.get(&[a, b]));
            assert!(component.is_zero(), "G_{{{a}{b}}} = {component:?}, expected 0 (Einstein tensor vanishes identically in 2D)");
        }
    }
}

/// The mandatory dimension barrier (Marco 6 step 6): `weyl_tensor`
/// refuses cleanly in 2D, never dividing by the formula's `1/(n-2)`
/// term at `n=2`, never inventing a component.
#[test]
fn weyl_tensor_refuses_in_two_dimensions() {
    let s = build();
    // The dimension check runs before `riemann_cov` is ever read, so an
    // all-zero placeholder Grid is enough -- computing the real
    // riemann_cov here would only add unused work to a test whose whole
    // point is that the function refuses before reaching it.
    let placeholder_riemann_cov = Grid::new(2, 4);
    let err = weyl_tensor(&s.registry, &s.chart, &s.g, &placeholder_riemann_cov, &s.ricci, &s.scalar).unwrap_err();
    assert!(matches!(err, ComponentError::WeylUndefinedInDimension2), "expected WeylUndefinedInDimension2, got {err}");
}

/// `weyl_squared` inherits the same refusal (it computes the Weyl
/// tensor first, propagating the error via `?`) -- confirmed directly,
/// not just assumed from `weyl_tensor`'s own test above.
#[test]
fn weyl_squared_refuses_in_two_dimensions() {
    let s = build();
    let placeholder_riemann_cov = Grid::new(2, 4);
    let err = weyl_squared(&s.registry, &s.chart, &s.g, &placeholder_riemann_cov, &s.ricci, &s.scalar, &s.ginv).unwrap_err();
    assert!(matches!(err, ComponentError::WeylUndefinedInDimension2), "expected WeylUndefinedInDimension2, got {err}");
}
