//! Acceptance test for the gallery's anti-de Sitter entry
//! (`oderom-notebook/src/gallery.rs`, entry `antidesitter`) -- constant
//! *negative* curvature, in contrast to `desitter.rs`'s constant
//! *positive* curvature fixture. Poincare-patch coordinates `t, x, y,
//! z`: `ds^2 = (1/(Hz))^2 (-dt^2+dx^2+dy^2+dz^2)`, `z` playing the role
//! of the "holographic" radial direction (never assumed positive by the
//! engine -- it treats `z` as an ordinary free symbol, same as every
//! other coordinate).
//!
//! Closed forms below (`Gamma^a_bc`, `R = -12H^2`) are the standard
//! conformally-flat-metric result for `g = phi(z)^2 eta`, `phi = 1/(Hz)`:
//! `Gamma^a_bc = delta^a_b d_c(ln phi) + delta^a_c d_b(ln phi) - eta_bc
//! eta^ad d_d(ln phi)`, hand-derived (not copied from a table) using
//! `d(ln phi)/dz = phi'/phi = -1/z` (H cancels out of this ratio,
//! itself an independent check that the engine's H-dependence enters
//! only through the overall conformal factor, exactly as the closed
//! form predicts) -- see the session log for the derivation in full.
//! `R = -n(n-1)/L^2` for AdS_n with `L = 1/H`, `n=4`: the standard,
//! widely cited textbook/AdS-CFT-literature result (e.g. Wald's
//! conventions), same status as `desitter.rs`'s own `R = n(n-1)H^2`
//! citation for de Sitter -- re-derived by the engine from Christoffel
//! symbols up, never hardcoded into the test as anything but the target
//! to check against.

use oderom_components::curvature::{christoffel, metric_inverse_diagonal, ricci_scalar, ricci_tensor, riemann_mixed};
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;

struct AntiDeSitter {
    registry: Registry,
    chart: Chart,
    g: ComponentTensor,
    ginv: Grid,
    gamma: Grid,
    riem_mixed: Grid,
}

fn build() -> Result<AntiDeSitter, ComponentError> {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("M", 4).unwrap();
    let tm = registry.declare_bundle("TM", manifold, 4).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };
    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry.declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();

    let chart = Chart::new(["t", "x", "y", "z"]);
    // (1/(H*z))^2, written as Pow(H*z, -2) to keep the metric a single
    // rational power rather than a squared fraction -- same shape
    // `unknown_static_spherical.od`'s own reciprocal ansatz uses.
    let conformal = || Expr::Pow(Box::new(Expr::var("H") * Expr::var("z")), -2);

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&-conformal()))?;
    g.set(&registry, &[1, 1], normalize(&conformal()))?;
    g.set(&registry, &[2, 2], normalize(&conformal()))?;
    g.set(&registry, &[3, 3], normalize(&conformal()))?;

    let ginv = metric_inverse_diagonal(&registry, &chart, &g)?;
    let gamma = christoffel(&registry, &chart, &g, &ginv)?;
    let riem_mixed = riemann_mixed(&chart, &gamma);

    Ok(AntiDeSitter { registry, chart, g, ginv, gamma, riem_mixed })
}

#[test]
fn christoffel_of_anti_de_sitter_matches_the_hand_derived_closed_form() {
    let d = build().unwrap();
    let inv_z = normalize(&Expr::Pow(Box::new(Expr::var("z")), -1));
    let minus_inv_z = normalize(&-Expr::Pow(Box::new(Expr::var("z")), -1));

    // Gamma^z_xx = 1/z (z is coordinate index 3, x is index 1).
    assert_eq!(d.gamma.get(&[3, 1, 1]), inv_z, "Gamma^z_xx");
    // Gamma^z_yy shares the same spatial-partner form as Gamma^z_xx.
    assert_eq!(d.gamma.get(&[3, 2, 2]), inv_z, "Gamma^z_yy");
    // Gamma^x_xz = Gamma^t_tz = Gamma^z_zz = -1/z.
    assert_eq!(d.gamma.get(&[1, 1, 3]), minus_inv_z, "Gamma^x_xz");
    assert_eq!(d.gamma.get(&[0, 0, 3]), minus_inv_z, "Gamma^t_tz");
    assert_eq!(d.gamma.get(&[3, 3, 3]), minus_inv_z, "Gamma^z_zz");
    // Gamma^z_tt = -1/z (opposite sign from Gamma^z_xx: eta_tt = -1).
    assert_eq!(d.gamma.get(&[3, 0, 0]), minus_inv_z, "Gamma^z_tt");
}

#[test]
fn ricci_scalar_of_anti_de_sitter_is_minus_twelve_h_squared() {
    let d = build().unwrap();
    let ricci = ricci_tensor(&d.chart, &d.riem_mixed);
    let scalar = normalize(&ricci_scalar(&d.chart, &ricci, &d.ginv));
    let expected = normalize(&(Expr::int(-12) * Expr::var("H").pow(2)));
    assert_eq!(scalar, expected);
}

/// Maximally symmetric spaces satisfy `R_ab = (R/n) g_ab` -- an
/// independent, structural check beyond the scalar trace alone (a
/// coincidentally-matching trace could still hide a Ricci tensor that
/// isn't actually proportional to the metric). `R/n = -12H^2/4 = -3H^2`.
#[test]
fn ricci_tensor_of_anti_de_sitter_is_proportional_to_the_metric() {
    let d = build().unwrap();
    let ricci = ricci_tensor(&d.chart, &d.riem_mixed);
    let minus_three_h_sq = normalize(&(Expr::int(-3) * Expr::var("H").pow(2)));
    for a in 0..4u8 {
        for b in 0..4u8 {
            let g_ab = normalize(&d.g.get(&d.registry, &[a, b]).unwrap());
            let expected = normalize(&(minus_three_h_sq.clone() * g_ab));
            let actual = normalize(&ricci.get(&[a, b]));
            assert_eq!(actual, expected, "Ricci[{a},{b}] must equal -3H^2 * g[{a},{b}]");
        }
    }
}
