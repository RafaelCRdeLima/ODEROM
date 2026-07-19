//! Marco 5 acceptance test (DESIGN-M5.md, section 4): the holonomy of a
//! geodesic triangle on the unit round sphere equals its area, within
//! numerical tolerance -- the first acceptance test in this project
//! that isn't exact symbolic equality, since it requires solving the
//! geodesic and parallel-transport ODEs numerically.
//!
//! The triangle is the "octant" spanned by the standard basis points
//! `A=(1,0,0)`, `B=(0,1,0)`, `C=(0,0,1)` on the unit sphere in R^3, each
//! side a quarter great circle. By symmetry it is exactly 1/8 of the
//! sphere's area, `4*pi/8 = pi/2`; by Gauss-Bonnet, on the unit sphere
//! (Gaussian curvature `K=1` everywhere), the holonomy angle around any
//! geodesic triangle equals its area, so this triangle's holonomy is
//! exactly `pi/2`.
//!
//! Everything is computed in one stereographic chart, projected from the
//! *south* pole `(0,0,-1)` (`u=X/(1+Z)`, `v=Y/(1+Z)`) so that none of
//! A, B, C sits at the chart's own singular point (only the south pole
//! itself is excluded, and the triangle never goes near it). In these
//! coordinates the three vertices land on convenient values:
//! `C -> (0,0)`, `A -> (1,0)`, `B -> (0,1)`.
//!
//! The three initial (position, unit-speed velocity) pairs below (one
//! per side, `C->A`, `A->B`, `B->C`) come from differentiating this
//! projection along each great circle at its starting vertex by hand;
//! each is independently checked to have `|v|_g = 1` exactly in the
//! comment next to it, and the test itself checks the integrator
//! actually lands close to the expected next vertex, which would fail
//! immediately if that hand derivation were wrong.

use oderom_components::curvature::{christoffel, metric_inverse_diagonal};
use oderom_components::{Chart, ChristoffelPrograms, ComponentTensor, integrate_geodesic_with_transport};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;
use std::f64::consts::PI;

fn round_metric_on_s2() -> (Registry, Chart, ChristoffelPrograms) {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("S2", 2).unwrap();
    let tm = registry.declare_bundle("TM", manifold, 2).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 2 };
    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry
        .declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)])
        .unwrap();

    let chart = Chart::new(["u", "v"]);
    let u = Expr::var("u");
    let v = Expr::var("v");
    let conformal_factor =
        Expr::int(4) * Expr::Pow(Box::new(Expr::one() + u.clone().pow(2) + v.clone().pow(2)), -2);

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&conformal_factor.clone())).unwrap();
    g.set(&registry, &[1, 1], normalize(&conformal_factor)).unwrap();

    let ginv = metric_inverse_diagonal(&registry, &chart, &g).unwrap();
    let gamma = christoffel(&registry, &chart, &g, &ginv).unwrap();
    let christoffel_programs = ChristoffelPrograms::compile(&chart, &gamma).unwrap();

    (registry, chart, christoffel_programs)
}

/// `(g_uu, g_vv)` of the round metric at `(u,v)` -- known in closed form,
/// used only to measure the result (not part of what's being tested,
/// which is the numerical integration using the *compiled* Christoffel
/// symbols).
fn diag_metric_at(u: f64, v: f64) -> (f64, f64) {
    let factor = 4.0 / (1.0 + u * u + v * v).powi(2);
    (factor, factor)
}

/// The oriented angle from `a` to `b`, both tangent vectors at the same
/// point with diagonal metric `(g_uu, g_vv)`.
fn angle_between(g: (f64, f64), a: [f64; 2], b: [f64; 2]) -> f64 {
    let (guu, gvv) = g;
    let dot = guu * a[0] * b[0] + gvv * a[1] * b[1];
    let cross = (guu * gvv).sqrt() * (a[0] * b[1] - a[1] * b[0]);
    let norm_a = (guu * a[0] * a[0] + gvv * a[1] * a[1]).sqrt();
    let norm_b = (guu * b[0] * b[0] + gvv * b[1] * b[1]).sqrt();
    (cross / (norm_a * norm_b)).atan2(dot / (norm_a * norm_b))
}

#[test]
fn holonomy_of_the_octant_triangle_equals_its_area() {
    let (_, _, christoffel) = round_metric_on_s2();
    let steps = 20_000;
    let quarter_turn = PI / 2.0;

    // Side C -> A: start (0,0), velocity (0.5, 0); |v|_g^2 = 4*0.5^2 = 1.
    let w0 = [1.0, 0.0];
    let (x1, _v1, w1) =
        integrate_geodesic_with_transport(&christoffel, &[0.0, 0.0], &[0.5, 0.0], &w0, quarter_turn, steps);
    assert_close(&x1, &[1.0, 0.0], 1e-4, "end of side C->A should reach A");

    // Side A -> B: start (1,0), velocity (0, 1); g_vv(1,0) = 4/4 = 1, |v|_g^2 = 1.
    let (x2, _v2, w2) =
        integrate_geodesic_with_transport(&christoffel, &[1.0, 0.0], &[0.0, 1.0], &w1, quarter_turn, steps);
    assert_close(&x2, &[0.0, 1.0], 1e-4, "end of side A->B should reach B");

    // Side B -> C: start (0,1), velocity (0,-1); g_vv(0,1) = 4/4 = 1, |v|_g^2 = 1.
    let (x3, _v3, w3) =
        integrate_geodesic_with_transport(&christoffel, &[0.0, 1.0], &[0.0, -1.0], &w2, quarter_turn, steps);
    assert_close(&x3, &[0.0, 0.0], 1e-4, "end of side B->C should return to C");

    let g_at_c = diag_metric_at(x3[0], x3[1]);
    let holonomy_angle = angle_between(g_at_c, w0, [w3[0], w3[1]]);

    let expected_area = PI / 2.0; // 1/8 of the unit sphere's area 4*pi
    assert!(
        (holonomy_angle.abs() - expected_area).abs() < 1e-3,
        "holonomy angle {holonomy_angle} (abs {}) should match the triangle's area {expected_area}",
        holonomy_angle.abs()
    );
}

fn assert_close(actual: &[f64], expected: &[f64], tol: f64, msg: &str) {
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < tol, "{msg}: got {actual:?}, expected {expected:?}");
    }
}
