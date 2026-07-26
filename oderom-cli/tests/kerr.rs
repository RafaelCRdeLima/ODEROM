//! Kerr, loaded from `examples/kerr.od` -- the same fixture the
//! deliverable ("examples/kerr.od that I can run by hand") points at --
//! exercised through the real parser (`oderom_cli::parser::parse_model`),
//! not through `oderom-components`' own `kerr.rs` test, which builds the
//! `ComponentTensor` directly via the Rust API and never touches the
//! `.od` grammar's off-diagonal `[t,phi] = ...` syntax at all.
//!
//! DESIGN-RATIONAL-FORM.md section 7.1 (Kerr's bivariate `Sigma`
//! denominator, no single pole variable) is fixed: the structured-
//! denominator engine (section 8, `oderom_expr::localized`) closes it.
//! Every test in this file is active -- `kerr_metric_inverse_from_od_file_
//! satisfies_g_ginv_equals_identity` was always fast (inversion never
//! depended on this limit); `kerr_christoffel_satisfies_metric_compatibility`
//! and `kretschmann_of_kerr_from_od_file_matches_closed_form` now run
//! through the localized engine instead of the general one and complete
//! in low single-digit seconds total, not 213s/never. Only
//! `kretschmann_of_kerr_from_od_file_runs_and_produces_output` (the
//! actual `oderom` binary, via subprocess) is still `#[ignore]`d: the
//! CLI itself hasn't been wired to the localized engine yet, a separate,
//! pending integration decision.

use oderom_cli::parser::parse_model;
use oderom_components::curvature::{
    christoffel_localized, kretschmann_localized, localization_generators, lower_first_index_localized, metric_inverse, riemann_mixed_localized, verify_metric_inverse,
};
use oderom_expr::{diff, normalize, normalize_localized, Expr, LocalizationContext};

fn load_kerr() -> oderom_cli::model::Model {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/kerr.od")).expect("examples/kerr.od must exist and be readable");
    parse_model(&src).expect("examples/kerr.od must parse")
}

#[test]
fn kerr_metric_inverse_from_od_file_satisfies_g_ginv_equals_identity() {
    let model = load_kerr();
    let (chart_name, _head, g) = model.metrics.get("g").expect("examples/kerr.od must declare a metric named `g`");
    let chart = model.charts.get(chart_name).expect("the metric's chart must be declared");

    let ginv = metric_inverse(&model.registry, chart, g).expect("Kerr's {t,phi} block plus two singleton blocks must invert");
    verify_metric_inverse(&model.registry, chart, g, &ginv).expect("g_ab * g^bc must equal delta^a_c for the metric parsed from examples/kerr.od");
}

/// Metric compatibility, `nabla_mu g_nu_rho = 0`, computed directly from
/// `examples/kerr.od`'s own `g`/`Gamma`, through the localized engine
/// (`oderom_expr::{LocalizationContext, normalize_localized}`,
/// DESIGN-RATIONAL-FORM.md section 8) instead of the general one.
///
/// **Important scope note, unchanged from the first version of this
/// test** (kept verbatim -- it was a genuine correction, not a detail
/// that stopped mattering once the engine got faster): `nabla_mu g_nu_rho
/// = 0` is an *algebraic identity* of the Levi-Civita construction --
/// substituting `Gamma^lambda_mu_nu = (1/2) g^lambda_sigma (d_mu g_sigma_nu
/// + d_nu g_sigma_mu - d_sigma g_mu_nu)` (and the same for
/// `Gamma^lambda_mu_rho`) back into `nabla_mu g_nu_rho` and using
/// `g^lambda_sigma g_lambda_rho = delta^sigma_rho` collapses everything
/// to `d_mu g_nu_rho - d_mu g_nu_rho = 0` -- true for *any* symmetric,
/// invertible `g` provided `Gamma` is built from `g` by this exact
/// formula, not a physical fact specific to Kerr. That means this test
/// cannot, by itself, catch a metric written down with the *wrong*
/// closed-form component (a different-but-still-symmetric-and-invertible
/// `g` satisfies its own compatibility identity just as trivially) --
/// concretely, it would **not** have caught this fixture's earlier
/// missing frame-dragging term in `g_phi_phi` (see this file's own doc
/// comment above and `examples/kerr.od`'s header for that bug), since
/// the version without that term is itself a perfectly valid symmetric
/// invertible metric with its own compatible connection. What it *does*
/// catch: any bug in `christoffel`'s formula/implementation itself, or
/// in how a genuinely off-diagonal `ComponentTensor`/`Grid` round-trips
/// through `get()` (sign, index, or contraction mistakes) -- a different
/// and still valuable class of error, exercised here on Kerr
/// specifically because it is the one fixture in this project with a
/// real off-diagonal metric component. Identity tests (this one,
/// Riemann's slot symmetries, Bianchi) catch implementation bugs and are
/// structurally blind to a wrong fixture; only a fact specific to *this*
/// metric (Ricci=0, the Kretschmann closed form, the block determinant
/// reducing to `-Delta*sin(theta)^2`, the `a->0` Schwarzschild limit)
/// can catch that. That's the corrected reason this test earns its
/// keep: implementation coverage, not fixture validation.
///
/// **Measured cost, corrected again**: the general engine took 213s in
/// `--release` here -- not `christoffel` itself (28ms via the localized
/// engine, see `diagnostic_localized_engine_stage_timing` in
/// `oderom-components/tests/kerr.rs`), but the compatibility sum's own
/// ~100+ `normalize()` calls over Kerr's nontrivial `Sigma`/`Delta`
/// denominators -- exactly the per-call cost the localized engine
/// exists to eliminate. Not `#[ignore]`d anymore.
#[test]
fn kerr_christoffel_satisfies_metric_compatibility() {
    let model = load_kerr();
    let (chart_name, _head, g) = model.metrics.get("g").expect("examples/kerr.od must declare a metric named `g`");
    let chart = model.charts.get(chart_name).expect("the metric's chart must be declared");

    let ginv = metric_inverse(&model.registry, chart, g).expect("Kerr's {t,phi} block plus two singleton blocks must invert");
    let seeds = localization_generators(&model.registry, chart, g).expect("localization_generators must succeed for Kerr");
    let mut ctx = LocalizationContext::new(&seeds);
    let gamma = christoffel_localized(&model.registry, chart, g, &ginv, &mut ctx).expect("christoffel must succeed for Kerr");

    let dim = chart.dim() as u8;
    for mu in 0..dim {
        for nu in 0..dim {
            for rho in nu..dim {
                let g_nu_rho = g.get(&model.registry, &[nu, rho]).unwrap();
                let mut sum = diff(&g_nu_rho, chart.coord(mu));
                for lambda in 0..dim {
                    let gamma_lambda_mu_nu = gamma.get(&[lambda, mu, nu]);
                    if !gamma_lambda_mu_nu.is_zero() {
                        sum = sum - gamma_lambda_mu_nu * g.get(&model.registry, &[lambda, rho]).unwrap();
                    }
                    let gamma_lambda_mu_rho = gamma.get(&[lambda, mu, rho]);
                    if !gamma_lambda_mu_rho.is_zero() {
                        sum = sum - gamma_lambda_mu_rho * g.get(&model.registry, &[nu, lambda]).unwrap();
                    }
                }
                let reduced = normalize_localized(&sum, &mut ctx);
                assert!(reduced.is_zero(), "nabla_{mu} g_{{{nu}{rho}}} = {reduced}, expected 0 (metric compatibility)");
            }
        }
    }
    assert!(ctx.fallback_log().is_empty(), "expected zero fallbacks to the general engine, got: {:?}", ctx.fallback_log());
}

/// The actual golden check this round exists to unlock: Kerr's
/// Kretschmann scalar, computed from `examples/kerr.od` end to end
/// through the localized engine (`metric_inverse` -> `christoffel_localized`
/// -> `riemann_mixed_localized` -> `lower_first_index_localized` ->
/// `kretschmann_localized`), compared by *structural* equality against
/// the known closed form -- not a string comparison against CLI stdout
/// (see `kretschmann_of_kerr_from_od_file_runs_and_produces_output`
/// below for why that's a separate, weaker check). Found during the
/// fixture sweep this round also asked for: an earlier version of this
/// test only asserted the CLI's stdout was non-empty, which would have
/// silently passed for *any* value the engine produced, correct or not
/// -- exactly the "sleeping test with an invisible bug" risk that sweep
/// was looking for, in this file itself.
///
/// No longer `#[ignore]`d: DESIGN-RATIONAL-FORM.md section 7.1 (Kerr's
/// bivariate `Sigma` denominator, no single pole variable) is fixed by
/// the structured-denominator engine (section 8) -- this test, and
/// `oderom-components/tests/kerr.rs::ricci_of_kerr_is_identically_zero_via_the_localized_engine`,
/// are that fix's own acceptance criteria. `oderom kretschmann
/// examples/kerr.od` from an actual shell still goes through the
/// general engine, not this one -- see
/// `kretschmann_of_kerr_from_od_file_runs_and_produces_output` below.
#[test]
fn kretschmann_of_kerr_from_od_file_matches_closed_form() {
    let model = load_kerr();
    let (chart_name, _head, g) = model.metrics.get("g").expect("examples/kerr.od must declare a metric named `g`");
    let chart = model.charts.get(chart_name).expect("the metric's chart must be declared");

    let ginv = metric_inverse(&model.registry, chart, g).unwrap();
    let seeds = localization_generators(&model.registry, chart, g).unwrap();
    let mut ctx = LocalizationContext::new(&seeds);
    let gamma = christoffel_localized(&model.registry, chart, g, &ginv, &mut ctx).unwrap();
    let riem_mixed = riemann_mixed_localized(chart, &gamma, &mut ctx);
    let riem_cov = lower_first_index_localized(&model.registry, chart, &riem_mixed, g, &mut ctx).unwrap();
    let k = kretschmann_localized(chart, &riem_cov, &ginv, &mut ctx);

    // K = 48*M^2*(r^2-a^2*cos(theta)^2)*((r^2+a^2*cos(theta)^2)^2 -
    // 16*r^2*a^2*cos(theta)^2) / (r^2+a^2*cos(theta)^2)^6 -- the closed
    // form named in examples/kerr.od's own header, collapsing to
    // 48*M^2/r^6 (Schwarzschild's own known value) as a -> 0.
    let m = Expr::var("M");
    let a = Expr::var("a");
    let r = Expr::var("r");
    let theta = Expr::var("theta");
    let sigma = r.clone().pow(2) + a.clone().pow(2) * theta.clone().cos().pow(2);
    let expected = normalize(
        &(Expr::int(48) * m.pow(2) * (r.clone().pow(2) - a.clone().pow(2) * theta.clone().cos().pow(2)) * (sigma.clone().pow(2) - Expr::int(16) * r.pow(2) * a.pow(2) * theta.cos().pow(2))
            * Expr::Pow(Box::new(sigma), -6)),
    );
    assert_eq!(k, expected, "kretschmann={k:?}\nexpected={expected:?}");
    assert!(ctx.fallback_log().is_empty(), "expected zero fallbacks to the general engine, got: {:?}", ctx.fallback_log());
}

/// Lighter, separate smoke check that `oderom kretschmann
/// examples/kerr.od` (the actual command line a user runs) completes
/// and prints something -- not a value check (see the structural test
/// above for that): pinning an exact stdout string here would be
/// guessing at a rendering this engine doesn't produce yet. Once
/// `kretschmann_of_kerr_from_od_file_matches_closed_form` above passes,
/// this is the place to tighten to `assert_eq!(stdout.trim(), "...")`
/// against the real rendered form, the same way
/// `end_to_end.rs::kretschmann_of_schwarzschild_from_a_file_ascii_and_latex_agree`
/// already does for Schwarzschild.
#[test]
#[ignore] // blocked by DESIGN-RATIONAL-FORM.md section 7.1, same as above
fn kretschmann_of_kerr_from_od_file_runs_and_produces_output() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_oderom"))
        .args(["kretschmann", concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/kerr.od")])
        .output()
        .expect("failed to run the oderom binary");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!String::from_utf8_lossy(&output.stdout).trim().is_empty());
}
