//! Differential test: the localized engine (`oderom_expr::{LocalizationContext,
//! normalize_localized}`, DESIGN-RATIONAL-FORM.md section 8) against the
//! general one, on every fixture in this project's corpus that the
//! general engine already solves -- Schwarzschild, Reissner-Nordstrom,
//! a round 2-sphere, and Godel's Ricci tensor (not its Ricci *scalar*,
//! which neither engine reduces -- section 7.2, exp(a)^n never fusing,
//! a different mechanism entirely, out of scope here).
//!
//! This is the gate DESIGN-RATIONAL-FORM.md section 8's own CLI-default
//! decision names explicitly: "o que compra o direito de virar isso o
//! padrão" -- passing this is what would justify making the localized
//! engine the CLI's default (try to localize always, fall back to the
//! general engine automatically when coprimality/square-freeness fails,
//! never gated on whether a metric *looks* diagonal or not, which is
//! exactly the conceptual error this whole round undid).

use oderom_components::curvature::{
    christoffel, christoffel_localized, kretschmann, kretschmann_localized, localization_generators, lower_first_index, lower_first_index_localized, metric_inverse,
    ricci_tensor, ricci_tensor_localized, riemann_mixed, riemann_mixed_localized,
};
use oderom_components::{Chart, ComponentError, ComponentTensor};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr, LocalizationContext};
use smallvec::SmallVec;

struct Metric {
    registry: Registry,
    chart: Chart,
    g: ComponentTensor,
}

fn declare_metric_head(registry: &mut Registry, dim: u32) -> oderom_core::HeadId {
    let manifold = registry.declare_manifold("M", dim).unwrap();
    let tm = registry.declare_bundle("TM", manifold, dim).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim };
    let slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    registry.declare_head("g", slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap()
}

fn schwarzschild() -> Result<Metric, ComponentError> {
    let mut registry = Registry::new();
    let head = declare_metric_head(&mut registry, 4);
    let chart = Chart::new(["t", "r", "theta", "phi"]);
    let m = Expr::var("M");
    let r = Expr::var("r");
    let theta = Expr::var("theta");
    let f = Expr::one() - Expr::int(2) * m * r.clone().pow(-1);
    let mut g = ComponentTensor::new(head);
    g.set(&registry, &[0, 0], normalize(&(Expr::int(-1) * f.clone())))?;
    g.set(&registry, &[1, 1], normalize(&f.pow(-1)))?;
    g.set(&registry, &[2, 2], normalize(&r.clone().pow(2)))?;
    g.set(&registry, &[3, 3], normalize(&(r.pow(2) * theta.sin().pow(2))))?;
    Ok(Metric { registry, chart, g })
}

fn reissner_nordstrom() -> Result<Metric, ComponentError> {
    let mut registry = Registry::new();
    let head = declare_metric_head(&mut registry, 4);
    let chart = Chart::new(["t", "r", "theta", "phi"]);
    let m = Expr::var("M");
    let q = Expr::var("Q");
    let r = Expr::var("r");
    let theta = Expr::var("theta");
    let f = Expr::one() - Expr::int(2) * m * r.clone().pow(-1) + q.pow(2) * r.clone().pow(-2);
    let mut g = ComponentTensor::new(head);
    g.set(&registry, &[0, 0], normalize(&(Expr::int(-1) * f.clone())))?;
    g.set(&registry, &[1, 1], normalize(&f.pow(-1)))?;
    g.set(&registry, &[2, 2], normalize(&r.clone().pow(2)))?;
    g.set(&registry, &[3, 3], normalize(&(r.pow(2) * theta.sin().pow(2))))?;
    Ok(Metric { registry, chart, g })
}

fn round_sphere() -> Result<Metric, ComponentError> {
    // ds^2 = dtheta^2 + sin(theta)^2 dphi^2 -- geodesic polar coordinates
    // on the unit round S^2, same standard form `oderom-components/tests/
    // hyperbolic_plane.rs` uses for its own (negative-curvature) analogue.
    let mut registry = Registry::new();
    let head = declare_metric_head(&mut registry, 2);
    let chart = Chart::new(["theta", "phi"]);
    let theta = Expr::var("theta");
    let mut g = ComponentTensor::new(head);
    g.set(&registry, &[0, 0], Expr::one())?;
    g.set(&registry, &[1, 1], normalize(&theta.sin().pow(2)))?;
    Ok(Metric { registry, chart, g })
}

fn godel() -> Result<Metric, ComponentError> {
    // Same metric as `oderom-components/tests/godel.rs`'s own build():
    // ds^2 = a^2[-(dt+e^x dy)^2 + dx^2 + (1/2)e^(2x)dy^2 + dz^2].
    let mut registry = Registry::new();
    let head = declare_metric_head(&mut registry, 4);
    let chart = Chart::new(["t", "x", "y", "z"]);
    let a = Expr::var("a");
    let x = Expr::var("x");
    let a2 = a.pow(2);
    let mut g = ComponentTensor::new(head);
    g.set(&registry, &[0, 0], normalize(&-a2.clone()))?;
    g.set(&registry, &[0, 2], normalize(&(-a2.clone() * x.clone().exp())))?;
    g.set(&registry, &[1, 1], normalize(&a2.clone()))?;
    g.set(&registry, &[2, 2], normalize(&(-Expr::rational(1, 2) * a2.clone() * (Expr::int(2) * x).exp())))?;
    g.set(&registry, &[3, 3], normalize(&a2))?;
    Ok(Metric { registry, chart, g })
}

/// Full pipeline (`metric_inverse` -> `christoffel` -> `riemann_mixed` ->
/// `lower_first_index` -> `kretschmann`) under the general engine.
fn kretschmann_general(m: &Metric) -> Result<Expr, ComponentError> {
    let ginv = metric_inverse(&m.registry, &m.chart, &m.g)?;
    let gamma = christoffel(&m.registry, &m.chart, &m.g, &ginv)?;
    let riem_mixed = riemann_mixed(&m.chart, &gamma);
    let riem_cov = lower_first_index(&m.registry, &m.chart, &riem_mixed, &m.g)?;
    Ok(normalize(&kretschmann(&m.chart, &riem_cov, &ginv)))
}

/// Same pipeline through the localized engine, seeded from `m`'s own
/// declared denominators and block determinants -- never a hardcoded
/// generator list, per requirement 2.
fn kretschmann_localized_pipeline(m: &Metric) -> Result<(Expr, LocalizationContext), ComponentError> {
    let ginv = metric_inverse(&m.registry, &m.chart, &m.g)?;
    let seeds = localization_generators(&m.registry, &m.chart, &m.g)?;
    let mut ctx = LocalizationContext::new(&seeds);
    let gamma = christoffel_localized(&m.registry, &m.chart, &m.g, &ginv, &mut ctx)?;
    let riem_mixed = riemann_mixed_localized(&m.chart, &gamma, &mut ctx)?;
    let riem_cov = lower_first_index_localized(&m.registry, &m.chart, &riem_mixed, &m.g, &mut ctx)?;
    let k = kretschmann_localized(&m.chart, &riem_cov, &ginv, &mut ctx)?;
    Ok((k, ctx))
}

#[test]
fn schwarzschild_kretschmann_agrees_between_engines() {
    let m = schwarzschild().unwrap();
    let general = kretschmann_general(&m).unwrap();
    let (localized, ctx) = kretschmann_localized_pipeline(&m).unwrap();
    assert_eq!(localized, general, "localized={localized:?}\ngeneral={general:?}");
    assert!(ctx.fallback_log().is_empty(), "{:?}", ctx.fallback_log());
}

#[test]
fn reissner_nordstrom_kretschmann_agrees_between_engines() {
    let m = reissner_nordstrom().unwrap();
    let general = kretschmann_general(&m).unwrap();
    let (localized, ctx) = kretschmann_localized_pipeline(&m).unwrap();
    assert_eq!(localized, general, "localized={localized:?}\ngeneral={general:?}");
    assert!(ctx.fallback_log().is_empty(), "{:?}", ctx.fallback_log());
}

#[test]
fn round_sphere_kretschmann_agrees_between_engines() {
    let m = round_sphere().unwrap();
    let general = kretschmann_general(&m).unwrap();
    let (localized, ctx) = kretschmann_localized_pipeline(&m).unwrap();
    assert_eq!(localized, general, "localized={localized:?}\ngeneral={general:?}");
    assert!(ctx.fallback_log().is_empty(), "{:?}", ctx.fallback_log());
}

/// Numeric oracle, same idea (and same reason) as `oderom-expr`'s own
/// `canonical.rs`/`localized.rs` test modules' private `eval` helpers:
/// used below because Godel's own denominators are not fully reduced by
/// *either* engine (section 7.2, `exp(a)^n` never fusing into
/// `exp(n*a)`) -- the two engines' content-stripping can legitimately
/// leave a different (but equal-value) overall scalar factor in
/// num/den, the same situation `oderom-expr/src/normalize.rs`'s
/// `v1_and_v2_agree` already lives with for legacy vs. current.
fn eval(e: &Expr, vars: &[(&str, f64)]) -> f64 {
    match e {
        Expr::Rational(s) => s.to_f64_lossy(),
        Expr::Var(name) => vars.iter().find(|(n, _)| *n == name).map(|(_, v)| *v).unwrap_or_else(|| panic!("unbound var {name}")),
        Expr::Add(terms) => terms.iter().map(|t| eval(t, vars)).sum(),
        Expr::Mul(factors) => factors.iter().map(|f| eval(f, vars)).product(),
        Expr::Pow(base, n) => eval(base, vars).powi(*n),
        Expr::Sin(arg) => eval(arg, vars).sin(),
        Expr::Cos(arg) => eval(arg, vars).cos(),
        Expr::Exp(arg) => eval(arg, vars).exp(),
        Expr::Sinh(arg) => eval(arg, vars).sinh(),
        Expr::Cosh(arg) => eval(arg, vars).cosh(),
        Expr::Func { name, .. } => panic!("no numeric value for indeterminate function `{name}` in this test"),
    }
}

/// Godel's Ricci *tensor* (not its scalar -- neither engine reduces that
/// one, section 7.2, an unrelated exp(a)^n-fusion gap) already completes
/// under the general engine today (`godel.rs`'s own non-`#[ignore]`d
/// `ricci_tensor_of_godel_is_not_identically_zero`). Compared
/// component-by-component, not as one contracted scalar, since the
/// scalar is exactly the part that doesn't close either way yet, and by
/// numeric value (see `eval`'s own doc comment) rather than `Expr`
/// equality: a first version of this test asserted structural equality
/// and found a real, expected mismatch -- both engines leave `R_00` with
/// `exp(x)^2` and `exp(2x)` un-fused (correct, per section 7.2), but
/// disagree on which overall factor of 2 got content-stripped from
/// numerator/denominator, an artifact of the two engines' different GCD
/// paths on an input neither one fully reduces, not a value bug.
#[test]
fn godel_ricci_tensor_agrees_between_engines() {
    let m = godel().unwrap();
    let ginv_g = metric_inverse(&m.registry, &m.chart, &m.g).unwrap();
    let gamma_g = christoffel(&m.registry, &m.chart, &m.g, &ginv_g).unwrap();
    let riem_g = riemann_mixed(&m.chart, &gamma_g);
    let ricci_general = ricci_tensor(&m.chart, &riem_g);

    let seeds = localization_generators(&m.registry, &m.chart, &m.g).unwrap();
    let mut ctx = LocalizationContext::new(&seeds);
    let ginv_l = metric_inverse(&m.registry, &m.chart, &m.g).unwrap();
    let gamma_l = christoffel_localized(&m.registry, &m.chart, &m.g, &ginv_l, &mut ctx).unwrap();
    let riem_l = riemann_mixed_localized(&m.chart, &gamma_l, &mut ctx).unwrap();
    let ricci_localized = ricci_tensor_localized(&m.chart, &riem_l, &mut ctx).unwrap();

    let vars: &[(&str, f64)] = &[("a", 1.3), ("x", 0.7)];
    for b in 0..m.chart.dim() as u8 {
        for d in 0..m.chart.dim() as u8 {
            let general = normalize(&ricci_general.get(&[b, d]));
            let localized = ricci_localized.get(&[b, d]);
            let gv = eval(&general, vars);
            let lv = eval(&localized, vars);
            assert!((gv - lv).abs() < 1e-6, "R_{{{b}{d}}}: localized={lv} general={gv}\nlocalized_expr={localized:?}\ngeneral_expr={general:?}");
        }
    }
}
