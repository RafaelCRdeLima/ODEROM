//! Diagnostic (not an acceptance test): measures time stage-by-stage
//! through Kerr's Christoffel/Riemann/Ricci pipeline, same discipline
//! `diagnostic_rn.rs` already established for Reissner-Nordstrom -- run
//! with:
//!
//! ```text
//! cargo test -p oderom-components --release --test diagnostic_kerr -- --ignored --nocapture
//! ```
//!
//! Exists because `kerr.rs`'s own golden `ricci_of_kerr_is_identically_zero`
//! test did not finish within several minutes (debug or release) the
//! first time it ran -- this isolates *which* stage the cost is in
//! (`metric_inverse`'s own 2x2 block, or `christoffel`/`riemann_mixed`/
//! `ricci_tensor` downstream of it) rather than guessing. `#[ignore]`d
//! for the same reason `diagnostic_rn.rs` is: several measurements below
//! are time-boxed, not run to completion.

use oderom_components::curvature::{christoffel, metric_block_structure, metric_inverse, ricci_tensor, riemann_mixed};
use oderom_components::{Chart, ComponentTensor, Grid};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn kerr_metric() -> (Registry, Chart, ComponentTensor) {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("M", 4).unwrap();
    let tm = registry.declare_bundle("TM", manifold, 4).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };
    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry.declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();

    let chart = Chart::new(["t", "r", "theta", "phi"]);
    let m = Expr::var("M");
    let a = Expr::var("a");
    let r = Expr::var("r");
    let theta = Expr::var("theta");
    let sigma = r.clone().pow(2) + a.clone().pow(2) * theta.clone().cos().pow(2);
    let delta = r.clone().pow(2) - Expr::int(2) * m.clone() * r.clone() + a.clone().pow(2);

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&-(Expr::int(1) - Expr::int(2) * m.clone() * r.clone() * Expr::Pow(Box::new(sigma.clone()), -1)))).unwrap();
    g.set(
        &registry,
        &[0, 3],
        normalize(&(-Expr::int(2) * m.clone() * a.clone() * r.clone() * theta.clone().sin().pow(2) * Expr::Pow(Box::new(sigma.clone()), -1))),
    )
    .unwrap();
    g.set(&registry, &[1, 1], normalize(&(sigma.clone() * Expr::Pow(Box::new(delta), -1)))).unwrap();
    g.set(&registry, &[2, 2], normalize(&sigma)).unwrap();
    g.set(&registry, &[3, 3], normalize(&((r.pow(2) + a.clone().pow(2)) * theta.sin().pow(2)))).unwrap();

    (registry, chart, g)
}

fn grid_total_nodes(grid: &Grid, dim: usize) -> usize {
    let mut total = 0;
    for i in 0..dim as u8 {
        for j in 0..dim as u8 {
            match grid.rank() {
                2 => total += grid.get(&[i, j]).node_count(),
                3 => {
                    for k in 0..dim as u8 {
                        total += grid.get(&[i, j, k]).node_count();
                    }
                }
                4 => {
                    for k in 0..dim as u8 {
                        for l in 0..dim as u8 {
                            total += grid.get(&[i, j, k, l]).node_count();
                        }
                    }
                }
                other => panic!("unexpected rank {other}"),
            }
        }
    }
    total
}

fn report(label: &str, grid: &Grid, elapsed: Duration) {
    let dim = grid.dim();
    let total = grid_total_nodes(grid, dim);
    let count = dim.pow(grid.rank() as u32);
    println!(
        "{label:<20} {elapsed:>10.3?}   total_nodes={total:>8}   components={count:>4}   avg_nodes/component={:.1}",
        total as f64 / count as f64
    );
}

/// Runs `f` on a background thread, waiting at most `budget` -- see
/// `diagnostic_rn.rs`'s own copy of this helper for why (honest
/// reporting instead of hanging the test suite).
fn time_boxed<T: Send + 'static>(budget: Duration, f: impl FnOnce() -> T + Send + 'static) -> Option<(T, Duration)> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let result = f();
        let _ = tx.send((result, t0.elapsed()));
    });
    rx.recv_timeout(budget).ok()
}

#[test]
#[ignore]
fn measure_kerr_christoffel_riemann_ricci_pipeline() {
    let (registry, chart, g) = kerr_metric();

    let t0 = Instant::now();
    let blocks = metric_block_structure(&registry, &chart, &g).unwrap();
    println!("metric_block_structure    {:>10.3?}   blocks={blocks:?}", t0.elapsed());

    let t0 = Instant::now();
    let ginv = metric_inverse(&registry, &chart, &g).unwrap();
    report("metric_inverse", &ginv, t0.elapsed());

    let (r1, c1, g1, gi1) = (registry.clone(), chart.clone(), g.clone(), ginv.clone());
    let christoffel_result = time_boxed(Duration::from_secs(180), move || christoffel(&r1, &c1, &g1, &gi1));
    let gamma = match christoffel_result {
        Some((Ok(gamma), elapsed)) => {
            report("christoffel", &gamma, elapsed);
            gamma
        }
        Some((Err(e), elapsed)) => {
            println!("christoffel FAILED after {elapsed:.3?}: {e:?}");
            return;
        }
        None => {
            println!("christoffel DID NOT FINISH within 180s");
            return;
        }
    };

    let c2 = chart.clone();
    let gamma1 = gamma.clone();
    let riemann_result = time_boxed(Duration::from_secs(180), move || riemann_mixed(&c2, &gamma1));
    let riem_mixed = match riemann_result {
        Some((riem, elapsed)) => {
            report("riemann_mixed", &riem, elapsed);
            riem
        }
        None => {
            println!("riemann_mixed DID NOT FINISH within 180s");
            return;
        }
    };

    let c3 = chart.clone();
    let riem1 = riem_mixed.clone();
    let ricci_result = time_boxed(Duration::from_secs(180), move || ricci_tensor(&c3, &riem1));
    match ricci_result {
        Some((ricci, elapsed)) => {
            report("ricci_tensor", &ricci, elapsed);
            for b in 0..chart.dim() as u8 {
                for d in 0..chart.dim() as u8 {
                    let component = normalize(&ricci.get(&[b, d]));
                    println!("  R_{{{b}{d}}} normalized nodes={}, is_zero={}", component.node_count(), component.is_zero());
                }
            }
        }
        None => println!("ricci_tensor DID NOT FINISH within 180s"),
    }
}
