//! Marco 2 acceptance tests, all against the Schwarzschild metric (in
//! Schwarzschild coordinates `t, r, theta, phi`):
//! `ds^2 = -(1 - 2M/r) dt^2 + dr^2/(1 - 2M/r) + r^2 dtheta^2 + r^2 sin^2(theta) dphi^2`.

use oderom_components::curvature::{
    christoffel, grid_to_component_tensor, kretschmann, lower_first_index,
    metric_inverse_diagonal, ricci_scalar, ricci_tensor, riemann_mixed,
};
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{HeadId, Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;

struct Schwarzschild {
    registry: Registry,
    riemann_head: HeadId,
    chart: Chart,
    ginv: Grid,
    riem_mixed: Grid,
    riem_cov: Grid,
}

fn build() -> Result<Schwarzschild, ComponentError> {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("M", 4).unwrap();
    let tm = registry.declare_bundle("TM", manifold, 4).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };

    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry
        .declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)])
        .unwrap();

    let riemann_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co, co, co];
    let pair_swap = SignedPerm::new(Perm::try_from_images(&[2, 3, 0, 1]).unwrap(), 1);
    let riemann_gens = vec![
        SignedPerm::new(Perm::transposition(4, 0, 1), -1),
        SignedPerm::new(Perm::transposition(4, 2, 3), -1),
        pair_swap,
    ];
    let riemann_head = registry.declare_head("R", riemann_slots, riemann_gens).unwrap();

    let chart = Chart::new(["t", "r", "theta", "phi"]);
    let m = Expr::var("M");
    let r = Expr::var("r");
    let theta = Expr::var("theta");

    // f = 1 - 2M/r
    let f = Expr::one() - Expr::int(2) * m * Expr::Pow(Box::new(r.clone()), -1);

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&(Expr::int(-1) * f.clone())))?;
    g.set(&registry, &[1, 1], normalize(&Expr::Pow(Box::new(f), -1)))?;
    g.set(&registry, &[2, 2], normalize(&r.clone().pow(2)))?;
    g.set(&registry, &[3, 3], normalize(&(r.pow(2) * theta.sin().pow(2))))?;

    let ginv = metric_inverse_diagonal(&registry, &chart, &g)?;
    let gamma = christoffel(&registry, &chart, &g, &ginv)?;
    let riem_mixed = riemann_mixed(&chart, &gamma);
    let riem_cov = lower_first_index(&registry, &chart, &riem_mixed, &g)?;

    Ok(Schwarzschild { registry, riemann_head, chart, ginv, riem_mixed, riem_cov })
}

#[test]
fn kretschmann_of_schwarzschild_is_48_m_squared_over_r_to_the_sixth() {
    let s = build().unwrap();

    // Exercise the "store only the independent components" path (Marco 2's
    // other headline requirement) before using it for the contraction.
    // 21, not the more commonly quoted 20: N(N+1)/2 for N = n(n-1)/2 = 6
    // antisymmetric-pair "slots" in 4D, treating R_{[ab][cd]} as symmetric
    // under pair exchange. The familiar 20 additionally imposes the first
    // Bianchi identity (R_{a[bcd]} = 0), which is a multi-term relation --
    // out of scope for both Marco 1's canonicalizer and this orbit count,
    // which only ever uses slot *permutation* symmetry (see DESIGN.md).
    let riemann_tensor = grid_to_component_tensor(&s.registry, s.riemann_head, &s.riem_cov);
    assert_eq!(riemann_tensor.independent_len(), 21);

    let kretschmann_scalar = kretschmann(&s.chart, &s.riem_cov, &s.ginv);
    let expected = normalize(&(Expr::int(48) * Expr::var("M").pow(2) * Expr::var("r").pow(-6)));
    assert_eq!(kretschmann_scalar, expected);
}

#[test]
fn ricci_of_schwarzschild_is_zero() {
    // Schwarzschild is a vacuum solution: R_bd = 0 everywhere, and so R = 0.
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    for b in 0..s.chart.dim() as u8 {
        for d in 0..s.chart.dim() as u8 {
            let component = normalize(&ricci.get(&[b, d]));
            assert!(component.is_zero(), "R_{{{b}{d}}} = {component:?}, expected 0");
        }
    }
    assert!(normalize(&ricci_scalar(&s.chart, &ricci, &s.ginv)).is_zero());
}
