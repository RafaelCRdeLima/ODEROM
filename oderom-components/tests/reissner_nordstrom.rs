//! Marco 2 acceptance test against the Reissner-Nordstrom metric (in
//! Schwarzschild-like coordinates `t, r, theta, phi`):
//! `ds^2 = -(1 - 2M/r + Q^2/r^2) dt^2 + dr^2/(1 - 2M/r + Q^2/r^2) + r^2 dtheta^2 + r^2 sin^2(theta) dphi^2`.
//!
//! Two free parameters (`M`, `Q`), unlike Schwarzschild's one -- this is
//! the case that motivated the rational-form engine's redesign in the
//! first place (DESIGN-RATIONAL-FORM.md): the legacy ad hoc engine
//! (`ODEROM_ENGINE=legacy`) cannot reduce this at all (naive expression
//! swell, no GCD ever attempted), and an earlier version of the
//! rational-form engine itself could not either (confirmed non-
//! termination, later root-caused and fixed: recursive multivariate
//! polynomial content management, subresultant PRS). Kept as a permanent
//! regression fixture for exactly that reason.

use oderom_components::curvature::{
    change_variance, christoffel, einstein_tensor, kretschmann, lower_first_index, metric_inverse_diagonal, ricci_scalar, ricci_squared,
    ricci_tensor, riemann_mixed, weyl_squared, weyl_tensor,
};
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;

struct ReissnerNordstrom {
    registry: Registry,
    chart: Chart,
    g: ComponentTensor,
    ginv: Grid,
    riem_mixed: Grid,
    riem_cov: Grid,
}

fn build() -> Result<ReissnerNordstrom, ComponentError> {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("M", 4).unwrap();
    let tm = registry.declare_bundle("TM", manifold, 4).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };

    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry
        .declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)])
        .unwrap();

    let chart = Chart::new(["t", "r", "theta", "phi"]);
    let m = Expr::var("M");
    let q = Expr::var("Q");
    let r = Expr::var("r");
    let theta = Expr::var("theta");

    // f = 1 - 2M/r + Q^2/r^2
    let f = Expr::one() - Expr::int(2) * m * r.clone().pow(-1) + q.pow(2) * r.clone().pow(-2);

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&(Expr::int(-1) * f.clone())))?;
    g.set(&registry, &[1, 1], normalize(&f.pow(-1)))?;
    g.set(&registry, &[2, 2], normalize(&r.clone().pow(2)))?;
    g.set(&registry, &[3, 3], normalize(&(r.pow(2) * theta.sin().pow(2))))?;

    let ginv = metric_inverse_diagonal(&registry, &chart, &g)?;
    let gamma = christoffel(&registry, &chart, &g, &ginv)?;
    let riem_mixed = riemann_mixed(&chart, &gamma);
    let riem_cov = lower_first_index(&registry, &chart, &riem_mixed, &g)?;

    Ok(ReissnerNordstrom { registry, chart, g, ginv, riem_mixed, riem_cov })
}

#[test]
fn kretschmann_of_reissner_nordstrom_matches_the_closed_form() {
    let s = build().unwrap();
    let kretschmann_scalar = kretschmann(&s.chart, &s.riem_cov, &s.ginv);
    // 48M^2/r^6 - 96MQ^2/r^7 + 56Q^4/r^8
    let m = Expr::var("M");
    let q = Expr::var("Q");
    let r = Expr::var("r");
    let expected_expr = Expr::int(48) * m.clone().pow(2) * r.clone().pow(-6)
        + Expr::int(-96) * m * q.clone().pow(2) * r.clone().pow(-7)
        + Expr::int(56) * q.pow(4) * r.pow(-8);
    let expected = normalize(&expected_expr);
    assert_eq!(kretschmann_scalar, expected, "kretschmann_scalar={kretschmann_scalar:?}\nexpected={expected:?}");
}

#[test]
fn ricci_scalar_of_reissner_nordstrom_is_zero() {
    // Reissner-Nordstrom is electrovac, not vacuum: the Ricci *tensor* is
    // nonzero (sourced by the electromagnetic stress-energy), but its
    // trace -- the Ricci *scalar* -- vanishes, since the EM stress-energy
    // tensor is traceless in 4D.
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    assert!(normalize(&ricci_scalar(&s.chart, &ricci, &s.ginv)).is_zero());
}

/// The new `einstein` query's golden check for a genuinely non-vacuum
/// case: `G_ab = R_ab - (1/2) g_ab R`, assembled BY HAND here from this
/// same program's own `ricci_tensor`/`ricci_scalar` outputs for the same
/// metric, must match `einstein_tensor`'s own output component by
/// component -- never a value copied from a textbook, always the
/// program's own other two queries recombined.
#[test]
fn einstein_of_reissner_nordstrom_matches_ricci_and_scalar_assembled_by_hand() {
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    let scalar = ricci_scalar(&s.chart, &ricci, &s.ginv);
    let einstein = einstein_tensor(&s.registry, &s.chart, &s.g, &ricci, &scalar);

    for a in 0..4u8 {
        for b in 0..4u8 {
            let g_ab = s.g.get(&s.registry, &[a, b]).unwrap();
            let hand = normalize(&(ricci.get(&[a, b]) - g_ab * scalar.clone() / Expr::int(2)));
            assert_eq!(einstein.get(&[a, b]), hand, "G_{{{a}{b}}}: einstein={:?}, hand={hand:?}", einstein.get(&[a, b]));
        }
    }
}

/// Sharper than the generic hand-assembly above, specific to this
/// metric: since `ricci_scalar_of_reissner_nordstrom_is_zero` already
/// establishes `R = 0` for Reissner-Nordstrom (traceless electromagnetic
/// stress-energy in 4D), the formula collapses to `G_ab = R_ab` exactly
/// -- so `einstein_tensor` must equal `ricci_tensor` component by
/// component, not merely be "close" or "proportional".
#[test]
fn einstein_of_reissner_nordstrom_equals_ricci_since_the_scalar_vanishes() {
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    let scalar = ricci_scalar(&s.chart, &ricci, &s.ginv);
    assert!(scalar.is_zero(), "this test's whole premise is R=0 for Reissner-Nordstrom");
    let einstein = einstein_tensor(&s.registry, &s.chart, &s.g, &ricci, &scalar);

    for a in 0..4u8 {
        for b in 0..4u8 {
            assert_eq!(einstein.get(&[a, b]), ricci.get(&[a, b]), "G_{{{a}{b}}} should equal R_{{{a}{b}}} exactly since R=0");
        }
    }
}

/// Confirms `einstein_tensor` genuinely produces a NON-zero tensor here
/// -- otherwise the two tests above would pass vacuously (an
/// all-zero-equals-all-zero comparison proves nothing about the real
/// formula being exercised).
#[test]
fn einstein_of_reissner_nordstrom_is_not_identically_zero() {
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    let scalar = ricci_scalar(&s.chart, &ricci, &s.ginv);
    let einstein = einstein_tensor(&s.registry, &s.chart, &s.g, &ricci, &scalar);
    let any_nonzero = (0..4u8).flat_map(|a| (0..4u8).map(move |b| (a, b))).any(|(a, b)| !einstein.get(&[a, b]).is_zero());
    assert!(any_nonzero, "Reissner-Nordstrom is not vacuum; the Einstein tensor must be genuinely nonzero");
}

/// Reissner-Nordstrom's Ricci tensor is genuinely nonzero (electrovac,
/// not vacuum), so `R_ab R^ab` must be too.
#[test]
fn ricci_squared_of_reissner_nordstrom_is_not_zero() {
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    assert!(!normalize(&ricci_squared(&s.chart, &ricci, &s.ginv)).is_zero());
}

/// Marco 6 step 6's stress case: Reissner-Nordstrom is NOT vacuum, so
/// unlike Schwarzschild's golden test, Weyl must genuinely differ from
/// Riemann here -- confirmed both ways: `weyl_tensor` reproduces a hand
/// assembly of the standard formula built directly from THIS SAME run's
/// own `riemann_cov`/`ricci_tensor`/`ricci_scalar` (never a textbook
/// value), and separately is shown to actually disagree with
/// `riemann_cov` at at least one component (otherwise the hand-assembly
/// check alone could pass vacuously if some upstream bug made every
/// Ricci-dependent term cancel by accident).
#[test]
fn weyl_of_reissner_nordstrom_matches_a_hand_assembly_and_differs_from_riemann() {
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    let scalar = ricci_scalar(&s.chart, &ricci, &s.ginv);
    let weyl = weyl_tensor(&s.registry, &s.chart, &s.g, &s.riem_cov, &ricci, &scalar).unwrap();

    let n = Expr::int(4);
    let mut any_differs = false;
    for a in 0..4u8 {
        for b in 0..4u8 {
            for c in 0..4u8 {
                for d in 0..4u8 {
                    let g_ac = s.g.get(&s.registry, &[a, c]).unwrap();
                    let g_ad = s.g.get(&s.registry, &[a, d]).unwrap();
                    let g_bc = s.g.get(&s.registry, &[b, c]).unwrap();
                    let g_bd = s.g.get(&s.registry, &[b, d]).unwrap();
                    let correction = g_ac.clone() * ricci.get(&[d, b]) - g_ad.clone() * ricci.get(&[c, b]) - g_bc.clone() * ricci.get(&[d, a])
                        + g_bd.clone() * ricci.get(&[c, a]);
                    let trace_term = g_ac * g_bd - g_ad * g_bc;
                    let hand = normalize(
                        &(s.riem_cov.get(&[a, b, c, d]) - correction / (n.clone() - Expr::int(2))
                            + scalar.clone() * trace_term / ((n.clone() - Expr::int(1)) * (n.clone() - Expr::int(2)))),
                    );
                    let got = weyl.get(&[a, b, c, d]);
                    assert_eq!(got, hand, "C_{{{a}{b}{c}{d}}}: weyl={got:?}, hand={hand:?}");
                    if got != s.riem_cov.get(&[a, b, c, d]) {
                        any_differs = true;
                    }
                }
            }
        }
    }
    assert!(any_differs, "Reissner-Nordstrom is not vacuum; Weyl must differ from Riemann at at least one component");
}

/// The defining algebraic property of the Weyl tensor -- distinguishing
/// it from Riemann, which this formula subtracts trace parts out of --
/// is that it is COMPLETELY traceless: contracting any index pair with
/// the inverse metric gives exactly zero. Checked here on the first
/// index pair, `g^ac C_abcd`, for every `(b,d)`, against a metric where
/// the correction terms are genuinely doing something (unlike vacuum,
/// where this would hold trivially for Riemann itself too). A real
/// mathematical litmus test for the formula's sign convention,
/// independent of comparing against this crate's own other outputs.
#[test]
fn weyl_of_reissner_nordstrom_is_traceless() {
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    let scalar = ricci_scalar(&s.chart, &ricci, &s.ginv);
    let weyl = weyl_tensor(&s.registry, &s.chart, &s.g, &s.riem_cov, &ricci, &scalar).unwrap();

    for b in 0..4u8 {
        for d in 0..4u8 {
            let mut trace = Expr::zero();
            for a in 0..4u8 {
                let coeff = s.ginv.get(&[a, a]);
                if coeff.is_zero() {
                    continue;
                }
                for c in 0..4u8 {
                    let w = weyl.get(&[a, b, c, d]);
                    if w.is_zero() {
                        continue;
                    }
                    // g^ac is diagonal, so only a==c contributes.
                    if a == c {
                        trace = trace + coeff.clone() * w;
                    }
                }
            }
            let trace = normalize(&trace);
            assert!(trace.is_zero(), "g^{{ac}} C_{{a{b}c{d}}} = {trace:?}, expected 0 (Weyl must be totally traceless)");
        }
    }
}

/// `weyl_squared` on Reissner-Nordstrom: genuinely nonzero (Weyl itself
/// is nonzero here) and, as a real cross-check, distinct from
/// `kretschmann` -- unlike vacuum, where the two coincide.
#[test]
fn weyl_squared_of_reissner_nordstrom_is_nonzero_and_differs_from_kretschmann() {
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    let scalar = ricci_scalar(&s.chart, &ricci, &s.ginv);
    let wsq = normalize(&weyl_squared(&s.registry, &s.chart, &s.g, &s.riem_cov, &ricci, &scalar, &s.ginv).unwrap());
    assert!(!wsq.is_zero());
    let k = normalize(&kretschmann(&s.chart, &s.riem_cov, &s.ginv));
    assert_ne!(wsq, k, "weyl_squared should differ from kretschmann in a non-vacuum spacetime");
}

/// A general decomposition identity, independent of anything specific
/// to this metric: `R_abcd R^abcd = C_abcd C^abcd + 4/(n-2) R_ab R^ab -
/// 2/((n-1)(n-2)) R^2` (Riemann-squared splits into a Weyl part and a
/// Ricci part) -- in 4D, `kretschmann = weyl_squared + 2*ricci_squared -
/// R^2/3`. Reissner-Nordstrom's own `R=0` simplifies the last term away,
/// leaving `kretschmann = weyl_squared + 2*ricci_squared` exactly --
/// checked here against this run's own three already-verified outputs,
/// a real cross-check between three independently-implemented functions
/// that has to hold if all three are actually correct, not just each
/// individually plausible.
#[test]
fn kretschmann_of_reissner_nordstrom_equals_weyl_squared_plus_twice_ricci_squared() {
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    let scalar = ricci_scalar(&s.chart, &ricci, &s.ginv);
    assert!(scalar.is_zero(), "this identity's simplified form assumes R=0 for Reissner-Nordstrom");
    let wsq = weyl_squared(&s.registry, &s.chart, &s.g, &s.riem_cov, &ricci, &scalar, &s.ginv).unwrap();
    let rsq = ricci_squared(&s.chart, &ricci, &s.ginv);
    let k = kretschmann(&s.chart, &s.riem_cov, &s.ginv);
    let rhs = normalize(&(wsq + Expr::int(2) * rsq));
    assert_eq!(normalize(&k), rhs, "kretschmann={k:?} should equal weyl_squared + 2*ricci_squared={rhs:?}");
}

/// Index-variance round trip (Rodada Variancia) for Ricci specifically,
/// against genuinely NONZERO components -- Reissner-Nordstrom is not
/// vacuum (unlike Schwarzschild, whose own Ricci is identically zero and
/// so round-trips trivially regardless of whether raise/lower is
/// correct). Raising both of Ricci's indices then lowering them straight
/// back must reproduce the original covariant `R_bd`, value for value
/// (`normalize()` of the difference, not structural equality -- see
/// `schwarzschild.rs`'s own `assert_same_value` for why two
/// independently-derived-but-equal expressions can differ in exact tree
/// shape).
#[test]
fn ricci_index_variance_round_trip_reproduces_the_original_on_a_non_vacuum_metric() {
    let s = build().unwrap();
    let ricci_cov = ricci_tensor(&s.chart, &s.riem_mixed);

    let down = vec![Variance::Co, Variance::Co];
    let up = vec![Variance::Contra, Variance::Contra];
    let raised = change_variance(&s.registry, &s.chart, &ricci_cov, &down, &up, &s.g, &s.ginv).unwrap();
    let back_down = change_variance(&s.registry, &s.chart, &raised, &up, &down, &s.g, &s.ginv).unwrap();

    let mut any_nonzero = false;
    for b in 0..s.chart.dim() as u8 {
        for d in 0..s.chart.dim() as u8 {
            let original = ricci_cov.get(&[b, d]);
            if !normalize(&original).is_zero() {
                any_nonzero = true;
            }
            let diff = normalize(&(back_down.get(&[b, d]) - original));
            assert!(diff.is_zero(), "R_{{{b}{d}}} did not round-trip through raise-then-lower: difference = {diff:?}");
        }
    }
    assert!(any_nonzero, "test setup error: Reissner-Nordstrom's Ricci tensor should have at least one nonzero component -- this round trip would be vacuous otherwise");
}
