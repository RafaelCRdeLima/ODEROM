//! Acceptance test for `sinh`/`cosh` as metric-component transcendentals
//! -- see `oderom-cli/tests/fixtures/hyperbolic_plane.od` for the real
//! `.od` file this mirrors, and `oderom-expr/src/diff.rs`'s
//! `d(sinh(u))=cosh(u)*u'`/`d(cosh(u))=sinh(u)*u'` rules this exercises
//! through a real curvature computation.
//!
//! The hyperbolic plane H^2 in geodesic polar coordinates `chi, phi`:
//! `ds^2 = dchi^2 + sinh(chi)^2 dphi^2` -- the direct hyperbolic
//! analogue of the round 2-sphere (`dtheta^2 + sin(theta)^2 dphi^2`,
//! `oderom-components/tests/sphere.rs`'s own metric shape, though that
//! file tests chart-transition consistency rather than curvature
//! values). Constant NEGATIVE curvature: Gaussian curvature -1, so Ricci
//! scalar `R = 2K = -2` -- the exact sign flip of the sphere's `+2`.
//! Kretschmann is quadratic in curvature, so it comes out the same
//! magnitude either way: `4`, same as the sphere.
//!
//! Also the direct real-computation exercise of D-RF.7's hyperbolic
//! identity `cosh(arg)^2 -> 1 + sinh(arg)^2`
//! (`oderom-expr/src/poly.rs::reduce_trig_powers`): `Ricci[0,0]` reduces
//! to the exact constant `-1`, not a `cosh(chi)^2 - ...`-shaped
//! expression left unsimplified -- if the hyperbolic rewrite were never
//! firing, this would not collapse to a bare integer.

use oderom_components::curvature::{
    christoffel, kretschmann, lower_first_index, metric_inverse_diagonal, ricci_scalar, ricci_tensor, riemann_mixed,
};
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;

struct HyperbolicPlane {
    chart: Chart,
    ginv: Grid,
    gamma: Grid,
    riem_mixed: Grid,
    riem_cov: Grid,
}

fn build() -> Result<HyperbolicPlane, ComponentError> {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("H2", 2).unwrap();
    let tm = registry.declare_bundle("TH2", manifold, 2).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 2 };
    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry.declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();

    let chart = Chart::new(["chi", "phi"]);
    let chi = Expr::var("chi");

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&Expr::one()))?;
    g.set(&registry, &[1, 1], normalize(&chi.sinh().pow(2)))?;

    let ginv = metric_inverse_diagonal(&registry, &chart, &g)?;
    let gamma = christoffel(&registry, &chart, &g, &ginv)?;
    let riem_mixed = riemann_mixed(&chart, &gamma);
    let riem_cov = lower_first_index(&registry, &chart, &riem_mixed, &g)?;

    Ok(HyperbolicPlane { chart, ginv, gamma, riem_mixed, riem_cov })
}

#[test]
fn christoffel_of_the_hyperbolic_plane_matches_the_closed_form() {
    // Gamma^chi_phiphi = -sinh(chi)*cosh(chi) (from d(sinh(chi)^2)/dchi =
    // 2*sinh(chi)*cosh(chi) -- d(sinh(u))=cosh(u)*u', d(u^2)=2u*u' --
    // times -1/2 in the Christoffel formula for a diagonal metric's
    // off-diagonal-index term); Gamma^phi_chiphi = cosh(chi)/sinh(chi)
    // (the same derivative divided by g_phiphi itself). Both are a
    // direct exercise of the sinh/cosh derivative rules, not just of the
    // functions being accepted at all.
    let hp = build().unwrap();
    let chi = Expr::var("chi");

    let expected_chi_phiphi = normalize(&(Expr::int(-1) * chi.clone().sinh() * chi.clone().cosh()));
    assert_eq!(hp.gamma.get(&[0, 1, 1]), expected_chi_phiphi, "Gamma^chi_phiphi");

    let expected_phi_chiphi = normalize(&(chi.clone().cosh() * Expr::Pow(Box::new(chi.sinh()), -1)));
    assert_eq!(hp.gamma.get(&[1, 0, 1]), expected_phi_chiphi, "Gamma^phi_chiphi");
}

#[test]
fn ricci_scalar_of_the_hyperbolic_plane_is_negative_two() {
    let hp = build().unwrap();
    let ricci = ricci_tensor(&hp.chart, &hp.riem_mixed);

    // The cosh^2-sinh^2=1 rewrite (D-RF.7's hyperbolic analogue) is what
    // makes THIS assertion possible: without it, Ricci[0,0] would stay
    // some cosh(chi)^2-minus-sinh(chi)^2-shaped expression, not the bare
    // constant -1 a real closed-form Ricci tensor component must be for
    // a space of constant curvature.
    let r00 = normalize(&ricci.get(&[0, 0]));
    assert_eq!(r00, Expr::int(-1), "R_chichi");

    let scalar = normalize(&ricci_scalar(&hp.chart, &ricci, &hp.ginv));
    assert_eq!(scalar, Expr::int(-2));
}

#[test]
fn kretschmann_of_the_hyperbolic_plane_matches_the_sphere_in_magnitude() {
    // Kretschmann is quadratic in curvature -- the sphere (K=+1,
    // `oderom-components/tests/sphere.rs`'s own metric shape, verified
    // via the CLI in this same round: kretschmann = 4) and this
    // hyperbolic plane (K=-1) must agree in magnitude even though their
    // Ricci scalars have opposite sign.
    let hp = build().unwrap();
    let kretschmann_scalar = normalize(&kretschmann(&hp.chart, &hp.riem_cov, &hp.ginv));
    assert_eq!(kretschmann_scalar, Expr::int(4));
}
