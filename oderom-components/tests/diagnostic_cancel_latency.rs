//! Diagnostic (not an acceptance test): measures real per-component
//! timing for the whole-`Grid`-producing stages (`christoffel`,
//! `riemann_mixed`) against Reissner-Nordstrom, the worst real case this
//! project has -- to answer a concrete question from DESIGN-UI-SESSION.md
//! ("verificação 1"): what is the worst-case gap between a user clicking
//! cancel and the computation actually stopping, today (checkpoints only
//! between whole stages) versus with a per-component checkpoint added
//! inside these loops?
//!
//! Reimplements each function's loop body here (not a wrapper around the
//! real `christoffel`/`riemann_mixed`, which have no per-component hook
//! to time) rather than modifying production code for a measurement --
//! same primitives (`diff`, `normalize`), same data flow, so the numbers
//! are representative without touching `curvature.rs`.
//!
//! Run with:
//! ```text
//! cargo test -p oderom-components --release --test diagnostic_cancel_latency -- --ignored --nocapture
//! ```
//! `--release`: the debug-build numbers this project's other diagnostics
//! sometimes quote are 5-10x slower than what a real desktop app would
//! ship: DESIGN-UI-SESSION.md's answer needs the number a user actually
//! experiences.

use oderom_components::curvature::metric_inverse_diagonal;
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{diff, normalize, Expr};
use smallvec::SmallVec;
use std::time::{Duration, Instant};

struct ReissnerNordstrom {
    registry: Registry,
    chart: Chart,
    g: ComponentTensor,
    ginv: Grid,
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
    let f = Expr::one() - Expr::int(2) * m * r.clone().pow(-1) + q.pow(2) * r.clone().pow(-2);

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&(Expr::int(-1) * f.clone())))?;
    g.set(&registry, &[1, 1], normalize(&f.pow(-1)))?;
    g.set(&registry, &[2, 2], normalize(&r.clone().pow(2)))?;
    g.set(&registry, &[3, 3], normalize(&(r.pow(2) * theta.sin().pow(2))))?;

    let ginv = metric_inverse_diagonal(&registry, &chart, &g)?;
    Ok(ReissnerNordstrom { registry, chart, g, ginv })
}

struct Timing {
    total: Duration,
    max_single: Duration,
    count: usize,
}

fn report(label: &str, t: &Timing) {
    println!(
        "{label}: {} components, total={:?}, max single component={:?}",
        t.count, t.total, t.max_single
    );
}

#[test]
#[ignore]
fn measure_christoffel_and_riemann_mixed_per_component() {
    let s = build().unwrap();
    let n = s.chart.dim();

    // christoffel: reimplements the loop body exactly as curvature.rs
    // does, timing each (a,b,c) component individually.
    let mut gamma = Grid::new(n, 3);
    let mut christoffel_timing = Timing { total: Duration::ZERO, max_single: Duration::ZERO, count: 0 };
    for a in 0..n as u8 {
        for b in 0..n as u8 {
            for c in 0..n as u8 {
                let start = Instant::now();
                let mut sum = Expr::zero();
                for d in 0..n as u8 {
                    let ad = s.ginv.get(&[a, d]);
                    if ad.is_zero() {
                        continue;
                    }
                    let term = diff(&s.g.get(&s.registry, &[d, c]).unwrap(), s.chart.coord(b))
                        + diff(&s.g.get(&s.registry, &[d, b]).unwrap(), s.chart.coord(c))
                        + Expr::int(-1) * diff(&s.g.get(&s.registry, &[b, c]).unwrap(), s.chart.coord(d));
                    sum = sum + ad * term;
                }
                gamma.set(&[a, b, c], normalize(&(Expr::rational(1, 2) * sum)));
                let elapsed = start.elapsed();
                christoffel_timing.total += elapsed;
                christoffel_timing.max_single = christoffel_timing.max_single.max(elapsed);
                christoffel_timing.count += 1;
            }
        }
    }
    report("christoffel", &christoffel_timing);

    // riemann_mixed: same treatment, fed the just-computed real gamma
    // (identical data flow to production -- christoffel() then
    // riemann_mixed()).
    let mut riemann_timing = Timing { total: Duration::ZERO, max_single: Duration::ZERO, count: 0 };
    for a in 0..n as u8 {
        for b in 0..n as u8 {
            for c in 0..n as u8 {
                for d in 0..n as u8 {
                    let start = Instant::now();
                    let mut val = diff(&gamma.get(&[a, b, d]), s.chart.coord(c))
                        + Expr::int(-1) * diff(&gamma.get(&[a, b, c]), s.chart.coord(d));
                    for e in 0..n as u8 {
                        val = val
                            + gamma.get(&[a, c, e]) * gamma.get(&[e, b, d])
                            + Expr::int(-1) * gamma.get(&[a, d, e]) * gamma.get(&[e, b, c]);
                    }
                    normalize(&val);
                    let elapsed = start.elapsed();
                    riemann_timing.total += elapsed;
                    riemann_timing.max_single = riemann_timing.max_single.max(elapsed);
                    riemann_timing.count += 1;
                }
            }
        }
    }
    report("riemann_mixed", &riemann_timing);

    // `lower_first_index`/`raise_index` are not measured separately here:
    // same rank-4, dim=4 inner-sum shape as `riemann_mixed`'s loop above,
    // but with no `diff()` call per term (they only multiply and sum
    // already-computed `Grid` values) -- strictly cheaper per component,
    // so `riemann_mixed`'s numbers are already the representative
    // (and worse-case) ones among the four whole-`Grid` stages.

    println!(
        "\nworst-case cancel latency TODAY (checkpoint only between whole stages) = slowest single stage total = {:?}",
        christoffel_timing.total.max(riemann_timing.total)
    );
    println!(
        "worst-case cancel latency WITH a per-component checkpoint = slowest single component across both stages = {:?}",
        christoffel_timing.max_single.max(riemann_timing.max_single)
    );
}
