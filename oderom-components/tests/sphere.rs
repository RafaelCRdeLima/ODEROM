//! Marco 3 acceptance test: declare `S^2` with two stereographic charts
//! (projected from the north and south pole) and check that the round
//! metric, declared independently in each, agrees once pulled back
//! through the transition between them.
//!
//! Chart N (from the north pole), coordinates `(u,v)`:
//! `ds^2 = 4(du^2+dv^2) / (1+u^2+v^2)^2`.
//! Chart S (from the south pole), coordinates `(u',v')`, same form.
//! Transition N -> S is inversion: `u' = u/(u^2+v^2)`, `v' = v/(u^2+v^2)`.

use oderom_components::{metric_agrees_across_transition, Atlas, Chart, ChartId, ComponentTensor, TransitionMap};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;

fn round_metric(head: oderom_core::HeadId, registry: &Registry, u: &str, v: &str) -> ComponentTensor {
    let u = Expr::var(u);
    let v = Expr::var(v);
    let conformal_factor =
        Expr::int(4) * Expr::Pow(Box::new(Expr::one() + u.clone().pow(2) + v.clone().pow(2)), -2);
    let mut g = ComponentTensor::new(head);
    g.set(registry, &[0, 0], normalize(&conformal_factor.clone())).unwrap();
    g.set(registry, &[1, 1], normalize(&conformal_factor)).unwrap();
    g
}

#[test]
fn round_metric_on_s2_agrees_across_the_stereographic_transition() {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("S2", 2).unwrap();
    let tm = registry.declare_bundle("TM", manifold, 2).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 2 };
    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry
        .declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)])
        .unwrap();

    let mut atlas = Atlas::new();
    let chart_n: ChartId = atlas.add_chart(Chart::new(["u", "v"]));
    let chart_s: ChartId = atlas.add_chart(Chart::new(["u2", "v2"]));

    let g_n = round_metric(metric_head, &registry, "u", "v");
    let g_s = round_metric(metric_head, &registry, "u2", "v2");

    let u = Expr::var("u");
    let v = Expr::var("v");
    let denom = Expr::Pow(Box::new(u.clone().pow(2) + v.clone().pow(2)), -1);
    let transition = TransitionMap {
        from: chart_n,
        to: chart_s,
        forward: vec![normalize(&(u * denom.clone())), normalize(&(v * denom))],
    };

    let agrees =
        metric_agrees_across_transition(&registry, &atlas, &g_n, &g_s, &transition).unwrap();
    assert!(agrees, "round metric should be invariant across the stereographic transition");
}

#[test]
fn an_unrelated_metric_does_not_falsely_agree() {
    // Sanity check on the checker itself: a metric that is *not* the
    // pullback of g_s (a flat metric instead of the round one) must be
    // rejected, not vacuously accepted.
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("S2", 2).unwrap();
    let tm = registry.declare_bundle("TM", manifold, 2).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 2 };
    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry
        .declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)])
        .unwrap();

    let mut atlas = Atlas::new();
    let chart_n = atlas.add_chart(Chart::new(["u", "v"]));
    let chart_s = atlas.add_chart(Chart::new(["u2", "v2"]));

    let mut g_n_flat = ComponentTensor::new(metric_head);
    g_n_flat.set(&registry, &[0, 0], Expr::one()).unwrap();
    g_n_flat.set(&registry, &[1, 1], Expr::one()).unwrap();
    let g_s = round_metric(metric_head, &registry, "u2", "v2");

    let u = Expr::var("u");
    let v = Expr::var("v");
    let denom = Expr::Pow(Box::new(u.clone().pow(2) + v.clone().pow(2)), -1);
    let transition = TransitionMap {
        from: chart_n,
        to: chart_s,
        forward: vec![normalize(&(u * denom.clone())), normalize(&(v * denom))],
    };

    let agrees =
        metric_agrees_across_transition(&registry, &atlas, &g_n_flat, &g_s, &transition).unwrap();
    assert!(!agrees);
}
