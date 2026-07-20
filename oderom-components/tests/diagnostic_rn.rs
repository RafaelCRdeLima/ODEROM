//! Diagnostic (not an acceptance test): measures expression size and
//! timing at each stage of the Reissner-Nordstrom Kretschmann pipeline,
//! to find out *where* the blowup the user reported actually happens,
//! rather than guessing. Run with:
//!
//! ```text
//! cargo test -p oderom-components --test diagnostic_rn -- --ignored --nocapture
//! ```
//!
//! `#[ignore]`d because it deliberately prints instead of asserting, and
//! one of its measurements is time-boxed (a giant un-normalized sum can
//! legitimately take longer than any reasonable test timeout) rather
//! than run to completion -- it is a report, not a pass/fail check.

use oderom_components::curvature::{
    christoffel, grid_to_component_tensor, lower_first_index, metric_inverse_diagonal, raise_index,
    riemann_mixed,
};
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{HeadId, Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct ReissnerNordstrom {
    registry: Registry,
    riemann_head: HeadId,
    chart: Chart,
    ginv: Grid,
    riem_cov: Grid,
}

fn build() -> Result<ReissnerNordstrom, ComponentError> {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("M", 4).unwrap();
    let tm = registry.declare_bundle("TM", manifold, 4).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };

    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head =
        registry.declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();

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
    let q = Expr::var("Q");
    let r = Expr::var("r");
    let theta = Expr::var("theta");

    // f = 1 - 2M/r + Q^2/r^2 -- three terms, unlike every existing
    // fixture's two-term f(r).
    let f = Expr::one() - Expr::int(2) * m * Expr::Pow(Box::new(r.clone()), -1)
        + q.pow(2) * Expr::Pow(Box::new(r.clone()), -2);

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&(Expr::int(-1) * f.clone())))?;
    g.set(&registry, &[1, 1], normalize(&Expr::Pow(Box::new(f), -1)))?;
    g.set(&registry, &[2, 2], normalize(&r.clone().pow(2)))?;
    g.set(&registry, &[3, 3], normalize(&(r.pow(2) * theta.sin().pow(2))))?;

    let t0 = Instant::now();
    let ginv = metric_inverse_diagonal(&registry, &chart, &g)?;
    report("metric_inverse_diagonal", &ginv, t0.elapsed());

    let t0 = Instant::now();
    let gamma = christoffel(&registry, &chart, &g, &ginv)?;
    report("christoffel", &gamma, t0.elapsed());

    let t0 = Instant::now();
    let riem_mixed = riemann_mixed(&chart, &gamma);
    report("riemann_mixed", &riem_mixed, t0.elapsed());

    let t0 = Instant::now();
    let riem_cov = lower_first_index(&registry, &chart, &riem_mixed, &g)?;
    report("lower_first_index (riem_cov)", &riem_cov, t0.elapsed());

    Ok(ReissnerNordstrom { registry, riemann_head, chart, ginv, riem_cov })
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
        "{label:<32} {elapsed:>8.3?}   total_nodes={total:>8}   components={count:>4}   avg_nodes/component={:.1}",
        total as f64 / count as f64
    );
}

/// Runs `f` on a background thread, waiting at most `budget` -- honest
/// reporting instead of hanging the test suite for 15+ minutes.
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
fn measure_reissner_nordstrom_kretschmann_pipeline() {
    let rn = build().unwrap();
    let dim = rn.chart.dim();

    // The one place the user's own report (christoffel/riemann/ricci all
    // finish; kretschmann doesn't) hasn't yet been measured directly:
    // what kretschmann() itself does beyond what riemann/ricci already
    // exercise -- four raise_index passes, then a raw 256-term sum, then
    // one final normalize() call.
    let t0 = Instant::now();
    let raised0 = raise_index(&rn.chart, &rn.riem_cov, &rn.ginv, 0);
    report("raise_index(0)", &raised0, t0.elapsed());

    let t0 = Instant::now();
    let raised1 = raise_index(&rn.chart, &raised0, &rn.ginv, 1);
    report("raise_index(1)", &raised1, t0.elapsed());

    let t0 = Instant::now();
    let raised2 = raise_index(&rn.chart, &raised1, &rn.ginv, 2);
    report("raise_index(2)", &raised2, t0.elapsed());

    let t0 = Instant::now();
    let riemann_contra = raise_index(&rn.chart, &raised2, &rn.ginv, 3);
    report("raise_index(3) (riemann_contra)", &riemann_contra, t0.elapsed());

    // The raw sum kretschmann() builds via repeated `+` *before* its one
    // final normalize() call -- measuring this directly (no normalize
    // involved) tells us whether the blowup is already present in the
    // un-normalized accumulation, independent of normalize()'s own cost.
    let t0 = Instant::now();
    let mut raw_sum = Expr::zero();
    let mut term_count = 0;
    for i in 0..dim as u8 {
        for j in 0..dim as u8 {
            for k in 0..dim as u8 {
                for l in 0..dim as u8 {
                    let idx = [i, j, k, l];
                    let term = rn.riem_cov.get(&idx) * riemann_contra.get(&idx);
                    raw_sum = raw_sum + term;
                    term_count += 1;
                }
            }
        }
    }
    println!(
        "raw 256-term sum (pre-normalize)   {:>8.3?}   nodes={:>8}   terms_accumulated={term_count}",
        t0.elapsed(),
        raw_sum.node_count()
    );

    // The cost of normalizing a *single* pairwise product in isolation
    // (this is cheap even if the full sum isn't -- it isolates whether
    // per-term normalization or the final combination is the expensive
    // part).
    let sample_idx = [0u8, 1, 0, 1];
    let sample_term = rn.riem_cov.get(&sample_idx) * riemann_contra.get(&sample_idx);
    let t0 = Instant::now();
    let normalized_sample = normalize(&sample_term);
    println!(
        "normalize(single R_cov*R_contra term) {:>8.3?}   nodes_before={} nodes_after={}",
        t0.elapsed(),
        sample_term.node_count(),
        normalized_sample.node_count()
    );

    // Growth curve: normalize partial sums of increasing size (2, 4, 8,
    // ... terms) to see whether cost scales linearly, polynomially, or
    // explosively with the number of terms combined -- a real curve,
    // not just "it timed out eventually".
    let all_terms: Vec<Expr> = {
        let mut terms = Vec::with_capacity(256);
        for i in 0..dim as u8 {
            for j in 0..dim as u8 {
                for k in 0..dim as u8 {
                    for l in 0..dim as u8 {
                        let idx = [i, j, k, l];
                        terms.push(rn.riem_cov.get(&idx) * riemann_contra.get(&idx));
                    }
                }
            }
        }
        terms
    };
    let mut n = 2usize;
    while n <= 256 {
        let partial = Expr::Add(all_terms[..n].to_vec());
        match time_boxed(Duration::from_secs(20), move || normalize(&partial)) {
            Some((result, elapsed)) => {
                println!(
                    "normalize({n:>3} terms) {elapsed:>8.3?}   nodes_before={:>6} nodes_after={:>6}",
                    Expr::Add(all_terms[..n].to_vec()).node_count(),
                    result.node_count()
                );
            }
            None => {
                println!("normalize({n:>3} terms) DID NOT FINISH within 20s");
                break;
            }
        }
        n *= 2;
    }

    // The one call the user reported never finishing. Time-boxed at 60s:
    // if it doesn't finish, that itself is the measurement (blowup is in
    // normalize() on the full sum, not in building it).
    println!("normalize(raw 256-term sum): starting, budget 60s...");
    match time_boxed(Duration::from_secs(60), move || normalize(&raw_sum)) {
        Some((result, elapsed)) => {
            println!("normalize(raw 256-term sum) FINISHED in {elapsed:.3?}, nodes={}", result.node_count());
        }
        None => {
            println!("normalize(raw 256-term sum) DID NOT FINISH within 60s (matches the user's 15+ minute report)");
        }
    }

    // Also exercise the exact grid_to_component_tensor path riemann_cmd
    // uses, for a size comparison against the raw Grid.
    let cov_tensor = grid_to_component_tensor(&rn.registry, rn.riemann_head, &rn.riem_cov);
    println!("riem_cov as ComponentTensor: independent_len={}", cov_tensor.independent_len());
}
