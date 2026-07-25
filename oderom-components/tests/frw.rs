//! Acceptance test for the gallery's flat-FRW entry
//! (`oderom-notebook/src/gallery.rs`, entry `frw`) -- `ds^2 = -dt^2 +
//! a(t)^2 (dx^2+dy^2+dz^2)`, `a(t)` an indeterminate function of `t`
//! left completely unspecified (`Expr::func`, the same
//! coordinate-as-function machinery `unknown_static_spherical.od`
//! already exercises for a *radial* function -- this is the first
//! fixture to put one directly in a metric component depending on
//! *time*, and outside a geodesic-specific derived quantity).
//!
//! Closed forms (standard cosmology, signature `-,+,+,+`):
//! `Gamma^t_xx = a a'`, `Gamma^x_tx = a'/a`,
//! `R = 6(a''/a + (a'/a)^2)` -- hand-derived from
//! `R_tt = -3 a''/a`, `R_ij = (a a'' + 2 a'^2) delta_ij`,
//! `R = g^tt R_tt + g^ii R_ii = 3a''/a + 3(a''/a + 2(a'/a)^2) =
//! 6(a''/a + (a'/a)^2)` -- see the session log for the derivation in
//! full.

use oderom_components::curvature::{christoffel, metric_inverse_diagonal, ricci_scalar, ricci_tensor, riemann_mixed};
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;

struct Frw {
    chart: Chart,
    ginv: Grid,
    gamma: Grid,
    riem_mixed: Grid,
}

fn a(order: u32) -> Expr {
    Expr::Func { name: "a".to_string(), args: vec![Expr::var("t")], order: vec![order] }
}

fn build() -> Result<Frw, ComponentError> {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("M", 4).unwrap();
    let tm = registry.declare_bundle("TM", manifold, 4).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };
    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry.declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();

    let chart = Chart::new(["t", "x", "y", "z"]);
    let a_sq = normalize(&a(0).pow(2));

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&Expr::int(-1)))?;
    g.set(&registry, &[1, 1], a_sq.clone())?;
    g.set(&registry, &[2, 2], a_sq.clone())?;
    g.set(&registry, &[3, 3], a_sq)?;

    let ginv = metric_inverse_diagonal(&registry, &chart, &g)?;
    let gamma = christoffel(&registry, &chart, &g, &ginv)?;
    let riem_mixed = riemann_mixed(&chart, &gamma);

    Ok(Frw { chart, ginv, gamma, riem_mixed })
}

#[test]
fn christoffel_of_flat_frw_matches_the_closed_form() {
    let d = build().unwrap();
    let expected_t_xx = normalize(&(a(0) * a(1)));
    let expected_x_tx = normalize(&(a(1) * Expr::Pow(Box::new(a(0)), -1)));

    assert_eq!(d.gamma.get(&[0, 1, 1]), expected_t_xx, "Gamma^t_xx");
    assert_eq!(d.gamma.get(&[0, 2, 2]), expected_t_xx, "Gamma^t_yy");
    assert_eq!(d.gamma.get(&[1, 0, 1]), expected_x_tx, "Gamma^x_tx");
    assert_eq!(d.gamma.get(&[2, 0, 2]), expected_x_tx, "Gamma^y_ty");
}

#[test]
fn ricci_scalar_of_flat_frw_matches_the_closed_form() {
    let d = build().unwrap();
    let ricci = ricci_tensor(&d.chart, &d.riem_mixed);
    let scalar = normalize(&ricci_scalar(&d.chart, &ricci, &d.ginv));

    // 6*(a''/a + (a'/a)^2)
    let expected = normalize(
        &(Expr::int(6) * (a(2) * Expr::Pow(Box::new(a(0)), -1) + a(1).pow(2) * Expr::Pow(Box::new(a(0)), -2))),
    );
    assert_eq!(scalar, expected);
}
