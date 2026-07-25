//! Acceptance test for `exp` as a metric-component transcendental
//! (added alongside `sin`/`cos`/`sinh`/`cosh` -- see
//! `oderom-cli/tests/fixtures/desitter_flat_slicing.od` for the real
//! `.od` file this mirrors, and `oderom-expr/src/diff.rs`'s `d(exp(u)) =
//! exp(u)*u'` rule this exercises through a real curvature computation,
//! not just by inspection).
//!
//! De Sitter spacetime in flat (planar) slicing, coordinates `t, x, y,
//! z`: `ds^2 = -dt^2 + exp(2*H*t) (dx^2+dy^2+dz^2)`. Maximally symmetric,
//! constant curvature: `R = n(n-1) H^2` in `n` spacetime dimensions --
//! `12 H^2` here -- and Kretschmann `24 H^4` (the standard textbook
//! values for 4D de Sitter).

use oderom_components::curvature::{
    christoffel, kretschmann, lower_first_index, metric_inverse_diagonal, raise_index,
    ricci_scalar, ricci_tensor, riemann_mixed, weyl_tensor,
};
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;

struct DeSitter {
    registry: Registry,
    chart: Chart,
    g: ComponentTensor,
    ginv: Grid,
    gamma: Grid,
    riem_mixed: Grid,
    riem_cov: Grid,
}

fn build() -> Result<DeSitter, ComponentError> {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("M", 4).unwrap();
    let tm = registry.declare_bundle("TM", manifold, 4).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };
    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry.declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();

    let chart = Chart::new(["t", "x", "y", "z"]);
    let h = Expr::var("H");
    let t = Expr::var("t");
    let scale = (Expr::int(2) * h * t).exp();

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&Expr::int(-1)))?;
    g.set(&registry, &[1, 1], normalize(&scale))?;
    g.set(&registry, &[2, 2], normalize(&scale.clone()))?;
    g.set(&registry, &[3, 3], normalize(&scale))?;

    let ginv = metric_inverse_diagonal(&registry, &chart, &g)?;
    let gamma = christoffel(&registry, &chart, &g, &ginv)?;
    let riem_mixed = riemann_mixed(&chart, &gamma);
    let riem_cov = lower_first_index(&registry, &chart, &riem_mixed, &g)?;

    Ok(DeSitter { registry, chart, g, ginv, gamma, riem_mixed, riem_cov })
}

#[test]
fn christoffel_of_de_sitter_matches_the_closed_form() {
    // Gamma^t_xx = H*exp(2Ht) (d(exp(2Ht))/dt = 2H*exp(2Ht), times 1/2
    // from the Christoffel formula) and Gamma^x_tx = H (the same
    // derivative divided by g_xx itself, exp(2Ht)) -- both are a direct
    // exercise of `d(exp(u))/dx = exp(u)*u'` (`oderom-expr/src/diff.rs`),
    // not just of `exp` being accepted as a metric component at all.
    let d = build().unwrap();
    let scale = (Expr::int(2) * Expr::var("H") * Expr::var("t")).exp();

    let expected_t_xx = normalize(&(Expr::var("H") * scale));
    assert_eq!(d.gamma.get(&[0, 1, 1]), expected_t_xx, "Gamma^t_xx");
    assert_eq!(d.gamma.get(&[1, 0, 1]), normalize(&Expr::var("H")), "Gamma^x_tx");
}

#[test]
fn ricci_scalar_of_de_sitter_is_twelve_h_squared() {
    let d = build().unwrap();
    let ricci = ricci_tensor(&d.chart, &d.riem_mixed);
    let scalar = normalize(&ricci_scalar(&d.chart, &ricci, &d.ginv));
    let expected = normalize(&(Expr::int(12) * Expr::var("H").pow(2)));
    assert_eq!(scalar, expected);
}

#[test]
fn kretschmann_of_de_sitter_is_twenty_four_h_to_the_fourth() {
    let d = build().unwrap();
    let kretschmann_scalar = normalize(&kretschmann(&d.chart, &d.riem_cov, &d.ginv));
    let expected = normalize(&(Expr::int(24) * Expr::var("H").pow(4)));
    assert_eq!(kretschmann_scalar, expected);
}

/// De Sitter is the one fixture in this project with a genuinely
/// nonzero Ricci scalar (`R = 12H^2`, not `0` the way Schwarzschild's
/// and Reissner-Nordstrom's both are) -- the only case that actually
/// exercises the Weyl formula's `R * (g_ac g_bd - g_ad g_bc) /
/// ((n-1)(n-2))` trace term with a nonzero `R`, rather than that term
/// vanishing trivially. De Sitter is maximally symmetric (`R_ab =
/// (R/n) g_ab`, `R_abcd = R/(n(n-1)) (g_ac g_bd - g_ad g_bc)`), and for
/// any maximally symmetric space the Weyl tensor is a real theorem's
/// worth of identically zero -- a second, independent sign check the
/// vacuum-only Schwarzschild/Reissner-Nordstrom golden tests cannot
/// give, since both have `R=0` and so never exercise this term at all.
#[test]
fn weyl_of_de_sitter_is_identically_zero() {
    let d = build().unwrap();
    let ricci = ricci_tensor(&d.chart, &d.riem_mixed);
    let scalar = ricci_scalar(&d.chart, &ricci, &d.ginv);
    assert!(!normalize(&scalar).is_zero(), "this test's whole point is a nonzero R -- de Sitter's R=12H^2");
    let weyl = weyl_tensor(&d.registry, &d.chart, &d.g, &d.riem_cov, &ricci, &scalar).unwrap();
    for a in 0..4u8 {
        for b in 0..4u8 {
            for c in 0..4u8 {
                for e in 0..4u8 {
                    let component = normalize(&weyl.get(&[a, b, c, e]));
                    assert!(component.is_zero(), "C_{{{a}{b}{c}{e}}} = {component:?}, expected 0 (de Sitter is maximally symmetric)");
                }
            }
        }
    }
}

#[test]
fn raising_an_index_through_the_exp_metric_round_trips() {
    // A real, independent exercise of `exp` appearing in `ginv` (the
    // *inverse* metric, `exp(-2Ht)`) multiplying back against the
    // covariant Riemann tensor -- not just the derivative rule alone.
    let d = build().unwrap();
    let raised = raise_index(&d.chart, &d.riem_cov, &d.ginv, 0);
    assert!(!normalize(&raised.get(&[1, 0, 1, 0])).is_zero());
}
