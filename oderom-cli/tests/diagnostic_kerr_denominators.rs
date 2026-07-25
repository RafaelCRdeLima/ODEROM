//! Diagnostic (not an acceptance test): before touching the rational-form
//! engine to attack DESIGN-RATIONAL-FORM.md section 7.1 (Kerr's
//! bivariate `Sigma` denominator blocking `christoffel`/`riemann_mixed`),
//! this measures *what* the distinct denominators of Kerr's Christoffel
//! symbols actually are, once `normalize()` is done with them -- the
//! premise of one proposed fix (represent a rational function as
//! numerator polynomial + multiset of `(factor, exponent)` over the
//! localization at the metric's own declared denominators, `Sigma` and
//! `Delta`) is that every denominator that ever shows up is a power
//! product of just those two. This is the check for that premise,
//! not an assumption of it.
//!
//! Deliberately loads `examples/kerr.od` through the real parser
//! (`oderom_cli::parser::parse_model`), the same file the CLI's
//! `oderom kretschmann examples/kerr.od` would read -- not a
//! hand-built `ComponentTensor` via the Rust API the way
//! `oderom-components/tests/kerr.rs`'s own `build()` does. That
//! distinction matters here specifically: it is the input this
//! diagnostic exists to interrogate.
//!
//! Run with:
//! ```text
//! cargo test -p oderom-cli --release --test diagnostic_kerr_denominators -- --ignored --nocapture
//! ```

use oderom_cli::parser::parse_model;
use oderom_components::curvature::{christoffel, metric_inverse};
use oderom_expr::{normalize, rationalize, Expr};
use std::time::Instant;

fn load_kerr() -> oderom_cli::model::Model {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/kerr.od")).expect("examples/kerr.od must exist and be readable");
    parse_model(&src).expect("examples/kerr.od must parse")
}

/// Every top-level `Mul` factor of `den`, with `Pow(base, n)` split into
/// `(base, n)` and a bare (non-`Pow`) factor treated as `(factor, 1)` --
/// enough to see, per component, which sub-expressions are acting as
/// independent "poles", without needing a general factorization.
fn top_level_pow_factors(den: &Expr) -> Vec<(Expr, i32)> {
    fn one_factor(f: &Expr, out: &mut Vec<(Expr, i32)>) {
        match f {
            Expr::Pow(base, n) => out.push((base.as_ref().clone(), *n)),
            Expr::Rational(_) => {} // pure numeric factor, not a "pole"
            other => out.push((other.clone(), 1)),
        }
    }
    let mut out = Vec::new();
    match den {
        Expr::Mul(factors) => {
            for f in factors {
                one_factor(f, &mut out);
            }
        }
        other => one_factor(other, &mut out),
    }
    out
}

#[test]
#[ignore]
fn kerr_christoffel_denominators_are_logged() {
    let model = load_kerr();
    let (chart_name, _head, g) = model.metrics.get("g").expect("examples/kerr.od must declare metric `g`");
    let chart = model.charts.get(chart_name).expect("chart must be declared");

    let t0 = Instant::now();
    let ginv = metric_inverse(&model.registry, chart, &g).expect("Kerr's metric must invert");
    println!("metric_inverse: {:?}", t0.elapsed());

    let t0 = Instant::now();
    let gamma = christoffel(&model.registry, chart, &g, &ginv).expect("christoffel must succeed for Kerr");
    println!("christoffel: {:?}", t0.elapsed());

    let dim = chart.dim() as u8;
    let mut distinct_denominators: Vec<Expr> = Vec::new();
    let mut distinct_pole_factors: Vec<Expr> = Vec::new();
    let mut nonzero_components = 0usize;

    for a in 0..dim {
        for b in 0..dim {
            for c in b..dim {
                let component = normalize(&gamma.get(&[a, b, c]));
                if component.is_zero() {
                    continue;
                }
                nonzero_components += 1;
                let (_num, den) = rationalize(&component);
                if den != Expr::one() && !distinct_denominators.contains(&den) {
                    distinct_denominators.push(den.clone());
                }
                for (base, _n) in top_level_pow_factors(&den) {
                    let base = normalize(&base);
                    if !distinct_pole_factors.contains(&base) {
                        distinct_pole_factors.push(base);
                    }
                }
            }
        }
    }

    println!("\nnonzero independent Christoffel components: {nonzero_components}");
    println!("distinct denominators (whole, as rationalize() sees them): {}", distinct_denominators.len());
    for (i, d) in distinct_denominators.iter().enumerate() {
        println!("  [{i}] {d}");
    }
    println!("\ndistinct top-level pole factors (Pow bases / bare non-numeric factors):");
    for (i, f) in distinct_pole_factors.iter().enumerate() {
        println!("  [{i}] {f}");
    }

    // Reference candidates named in examples/kerr.od's own header --
    // printed, not asserted against: this test's job is to report what
    // is actually there, not to assume the answer.
    let sigma = normalize(&(Expr::var("r").pow(2) + Expr::var("a").pow(2) * Expr::var("theta").cos().pow(2)));
    let delta = normalize(&(Expr::var("r").pow(2) - Expr::int(2) * Expr::var("M") * Expr::var("r") + Expr::var("a").pow(2)));
    println!("\nfor reference -- Sigma normalizes to: {sigma}");
    println!("for reference -- Delta normalizes to: {delta}");
}
