//! Marco 2 acceptance tests, all against the Schwarzschild metric (in
//! Schwarzschild coordinates `t, r, theta, phi`):
//! `ds^2 = -(1 - 2M/r) dt^2 + dr^2/(1 - 2M/r) + r^2 dtheta^2 + r^2 sin^2(theta) dphi^2`.

use oderom_components::curvature::{
    accel_equations, change_variance, christoffel, einstein_tensor, gauss_bonnet, geodesic_equations, grid_to_component_tensor,
    kretschmann, lower_first_index, metric_inverse_diagonal, raise_index, ricci_scalar, ricci_squared, ricci_tensor, riemann_mixed,
    weyl_squared, weyl_tensor,
};
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{HeadId, Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;

struct Schwarzschild {
    registry: Registry,
    riemann_head: HeadId,
    chart: Chart,
    g: ComponentTensor,
    ginv: Grid,
    gamma: Grid,
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

    Ok(Schwarzschild { registry, riemann_head, chart, g, ginv, gamma, riem_mixed, riem_cov })
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

/// The `einstein` query's golden check: Schwarzschild is vacuum
/// (`R_ab = 0` and `R = 0`, established by the test above), so
/// `G_ab = R_ab - (1/2) g_ab R` must come out identically zero in every
/// one of the ten independent components -- if even one came out
/// nonzero, that would be a real bug, since Schwarzschild is a vacuum
/// solution by construction.
#[test]
fn einstein_of_schwarzschild_is_identically_zero() {
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    let scalar = ricci_scalar(&s.chart, &ricci, &s.ginv);
    let einstein = einstein_tensor(&s.registry, &s.chart, &s.g, &ricci, &scalar);
    for a in 0..s.chart.dim() as u8 {
        for b in 0..s.chart.dim() as u8 {
            let component = normalize(&einstein.get(&[a, b]));
            assert!(component.is_zero(), "G_{{{a}{b}}} = {component:?}, expected 0 (Schwarzschild is vacuum)");
        }
    }
}

/// Marco 6 step 6's golden check: Schwarzschild is vacuum (`R_ab=0`,
/// `R=0`), so the Weyl tensor's own correction terms vanish and
/// `C_abcd` must equal `R_abcd` component by component, byte for byte
/// -- the strongest and cheapest check available, since it needs no
/// hand arithmetic at all, just a direct comparison against this same
/// run's own `riemann_cov`.
#[test]
fn weyl_of_schwarzschild_equals_riemann() {
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    let scalar = ricci_scalar(&s.chart, &ricci, &s.ginv);
    let weyl = weyl_tensor(&s.registry, &s.chart, &s.g, &s.riem_cov, &ricci, &scalar).unwrap();
    for a in 0..4u8 {
        for b in 0..4u8 {
            for c in 0..4u8 {
                for d in 0..4u8 {
                    let idx = [a, b, c, d];
                    assert_eq!(weyl.get(&idx), s.riem_cov.get(&idx), "C_{{{a}{b}{c}{d}}} != R_{{{a}{b}{c}{d}}}");
                }
            }
        }
    }
}

/// Ricci is identically zero in vacuum, so `R_ab R^ab` must be too --
/// same vacuum identity as `ricci_of_schwarzschild_is_zero`, one
/// contraction further.
#[test]
fn ricci_squared_of_schwarzschild_is_zero() {
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    assert!(normalize(&ricci_squared(&s.chart, &ricci, &s.ginv)).is_zero());
}

/// In vacuum the Gauss-Bonnet density's other two terms (`-4 R_ab R^ab`,
/// `R^2`) both vanish, so it must reduce to exactly `kretschmann` --
/// `48*M^2/r^6`, this crate's own standing Schwarzschild acceptance
/// value (`kretschmann_of_schwarzschild_is_48_m_squared_over_r_to_the_sixth`),
/// reused here as a cross-check rather than restated.
#[test]
fn gauss_bonnet_of_schwarzschild_equals_kretschmann() {
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    let scalar = ricci_scalar(&s.chart, &ricci, &s.ginv);
    let rsq = ricci_squared(&s.chart, &ricci, &s.ginv);
    let k = kretschmann(&s.chart, &s.riem_cov, &s.ginv);
    let gb = gauss_bonnet(&k, &rsq, &scalar);
    assert_eq!(gb, k, "gauss_bonnet={gb:?}, kretschmann={k:?}");
}

/// `weyl_squared` on Schwarzschild: since Weyl equals Riemann in vacuum
/// (the test above), `C_abcd C^abcd` must equal `kretschmann` exactly
/// too -- the same identity one contraction further downstream.
#[test]
fn weyl_squared_of_schwarzschild_equals_kretschmann() {
    let s = build().unwrap();
    let ricci = ricci_tensor(&s.chart, &s.riem_mixed);
    let scalar = ricci_scalar(&s.chart, &ricci, &s.ginv);
    let wsq = weyl_squared(&s.registry, &s.chart, &s.g, &s.riem_cov, &ricci, &scalar, &s.ginv).unwrap();
    let k = kretschmann(&s.chart, &s.riem_cov, &s.ginv);
    assert_eq!(wsq, k, "weyl_squared={wsq:?}, kretschmann={k:?}");
}

/// Marco 6 step 4, round B: `geodesic_equations` on Schwarzschild
/// produces one equation per coordinate. Rather than comparing against
/// an externally-remembered textbook formula, this hand-assembles the
/// t- and r-equations directly from THIS SAME RUN's own `christoffel`
/// output (`s.gamma`) -- summing only the individual `(b,c)` terms a
/// reader would pick out of a printed Christoffel table by hand -- and
/// confirms `geodesic_equations` gives the identical (already-normalized)
/// result. Which `(b,c)` pairs are the nonzero ones for `t`/`r` is
/// itself verified computationally below (`assert!(...is_zero())` on
/// every other pair), never assumed from an external reference.
#[test]
fn geodesic_of_schwarzschild_matches_a_hand_assembly_from_this_runs_own_christoffel() {
    let s = build().unwrap();
    let param = "tau";
    let equations = geodesic_equations(&s.chart, &s.gamma, param);
    assert_eq!(equations.len(), 4);

    let vel = |i: u8| Expr::Func { name: s.chart.coord(i).to_string(), args: vec![Expr::var(param)], order: vec![1] };
    let acc = |i: u8| Expr::Func { name: s.chart.coord(i).to_string(), args: vec![Expr::var(param)], order: vec![2] };

    // t-equation (a = 0): Gamma^t_bc is nonzero only for (t,r) and (r,t)
    // (t, 0-indexed) -- verified below, not assumed.
    for b in 0..4u8 {
        for c in 0..4u8 {
            if (b, c) == (0, 1) || (b, c) == (1, 0) {
                continue;
            }
            assert!(normalize(&s.gamma.get(&[0, b, c])).is_zero(), "Gamma^t_{{{b}{c}}} unexpectedly nonzero");
        }
    }
    let hand_t = normalize(&(acc(0) + s.gamma.get(&[0, 0, 1]) * vel(0) * vel(1) + s.gamma.get(&[0, 1, 0]) * vel(1) * vel(0)));
    assert_eq!(hand_t, equations[0], "t-equation disagrees with hand assembly from this run's own christoffel:\n{hand_t:?}\nvs\n{:?}", equations[0]);

    // r-equation (a = 1): Gamma^r_bc is nonzero only on the diagonal
    // (t,t), (r,r), (theta,theta), (phi,phi) -- verified below.
    for b in 0..4u8 {
        for c in 0..4u8 {
            if b == c {
                continue;
            }
            assert!(normalize(&s.gamma.get(&[1, b, c])).is_zero(), "Gamma^r_{{{b}{c}}} unexpectedly nonzero");
        }
    }
    let hand_r = normalize(
        &(acc(1)
            + s.gamma.get(&[1, 0, 0]) * vel(0) * vel(0)
            + s.gamma.get(&[1, 1, 1]) * vel(1) * vel(1)
            + s.gamma.get(&[1, 2, 2]) * vel(2) * vel(2)
            + s.gamma.get(&[1, 3, 3]) * vel(3) * vel(3)),
    );
    assert_eq!(hand_r, equations[1], "r-equation disagrees with hand assembly from this run's own christoffel:\n{hand_r:?}\nvs\n{:?}", equations[1]);
}

/// The metric's own free coordinate (`Expr::Var("r")`, inside a
/// Christoffel coefficient) and the geodesic's "coordinate as a
/// function of the affine parameter" (`Expr::Func { name: "r", .. }`)
/// must never be confused: the r-equation's Christoffel coefficients
/// stay written in terms of the free `r`, while the velocities/
/// accelerations are `Expr::Func` nodes -- both present in the SAME
/// equation, never collapsed into one shape.
#[test]
fn free_coordinate_and_function_of_parameter_coexist_without_collapsing() {
    let s = build().unwrap();
    let equations = geodesic_equations(&s.chart, &s.gamma, "tau");
    let r_eq = &equations[1];

    let has_free_r = contains_var(r_eq, "r");
    let has_func_r = contains_func(r_eq, "r");
    assert!(has_free_r, "r-equation must still contain the free coordinate `r` inside a Christoffel coefficient: {r_eq:?}");
    assert!(has_func_r, "r-equation must contain `r` as a function of the affine parameter: {r_eq:?}");
}

fn contains_var(e: &Expr, name: &str) -> bool {
    match e {
        Expr::Var(v) => v == name,
        Expr::Add(terms) | Expr::Mul(terms) => terms.iter().any(|t| contains_var(t, name)),
        Expr::Pow(base, _) => contains_var(base, name),
        Expr::Sin(x) | Expr::Cos(x) | Expr::Exp(x) | Expr::Sinh(x) | Expr::Cosh(x) => contains_var(x, name),
        Expr::Func { args, .. } => args.iter().any(|a| contains_var(a, name)),
        Expr::Rational(_) => false,
    }
}

fn contains_func(e: &Expr, name: &str) -> bool {
    match e {
        Expr::Func { name: n, args, .. } => n == name || args.iter().any(|a| contains_func(a, name)),
        Expr::Add(terms) | Expr::Mul(terms) => terms.iter().any(|t| contains_func(t, name)),
        Expr::Pow(base, _) => contains_func(base, name),
        Expr::Sin(x) | Expr::Cos(x) | Expr::Exp(x) | Expr::Sinh(x) | Expr::Cosh(x) => contains_func(x, name),
        Expr::Var(_) | Expr::Rational(_) => false,
    }
}

/// Marco 6 step 4, round C: `accel_equations` on Schwarzschild solves
/// each of `geodesic_equations`'s own four closed-form equations for
/// its coordinate's own second derivative. The golden check: for each
/// coordinate, take `geodesic_equations`' own canonical equation,
/// independently `isolate_linear` it (the same function
/// `accel_equations` itself uses) to recover its coefficient, and
/// confirm that multiplying `accel_equations`' solved RHS back by that
/// coefficient and moving it to the other side reproduces EXACTLY the
/// canonical equation -- the two forms (solved, unsolved) must be the
/// same equation, never independently computed values that merely look
/// similar.
#[test]
fn accel_of_schwarzschild_reproduces_geodesic_when_multiplied_back() {
    let s = build().unwrap();
    let param = "tau";
    let canonical = geodesic_equations(&s.chart, &s.gamma, param);
    let solved = accel_equations(&s.chart, &s.gamma, param);
    assert_eq!(solved.len(), 4);

    for a in 0..4u8 {
        let coord = s.chart.coord(a);
        let target = Expr::Func { name: coord.to_string(), args: vec![Expr::var(param)], order: vec![2] };
        let (coeff, _remainder) = oderom_expr::isolate_linear(&canonical[a as usize], &target)
            .expect("geodesic_equations' own output must be linear in its own acceleration");
        assert!(!coeff.is_zero(), "coefficient of {coord}'' must not be zero");
        // By construction (the acceleration term is added once,
        // additively, before any Christoffel-times-velocity product),
        // this coefficient is always exactly 1 -- confirmed here, not
        // just assumed by the reconstruction check below.
        assert_eq!(coeff, Expr::one(), "coefficient of {coord}'' is expected to be exactly 1 by construction");

        let reconstructed = normalize(&(coeff * (target - solved[a as usize].clone())));
        assert_eq!(reconstructed, canonical[a as usize], "coordinate {coord}: the solved form does not reconstruct geodesic's own equation");
    }
}

/// The radial equation's own Christoffel coefficients have a
/// non-trivial denominator (`2*M*r - r^2`, not a bare monomial) -- the
/// case DESIGN-M6-PREP.md's own worked example names as the real test
/// for "simplified, not just algebraically correct": the solved form
/// must come out as one reduced fraction, never a fraction whose own
/// denominator or numerator itself still carries another division.
#[test]
fn accel_radial_equation_of_schwarzschild_has_no_nested_fraction() {
    let s = build().unwrap();
    let solved = accel_equations(&s.chart, &s.gamma, "tau");
    let radial = &solved[1];
    assert!(!has_nested_negative_power(radial), "radial acceleration has a nested fraction: {radial:?}");
}

fn has_nested_negative_power(e: &Expr) -> bool {
    match e {
        Expr::Pow(base, k) if *k < 0 => contains_negative_power(base),
        Expr::Add(terms) | Expr::Mul(terms) => terms.iter().any(has_nested_negative_power),
        Expr::Pow(base, _) => has_nested_negative_power(base),
        Expr::Sin(x) | Expr::Cos(x) | Expr::Exp(x) | Expr::Sinh(x) | Expr::Cosh(x) => has_nested_negative_power(x),
        Expr::Func { args, .. } => args.iter().any(has_nested_negative_power),
        Expr::Var(_) | Expr::Rational(_) => false,
    }
}

fn contains_negative_power(e: &Expr) -> bool {
    match e {
        Expr::Pow(_, k) if *k < 0 => true,
        Expr::Add(terms) | Expr::Mul(terms) => terms.iter().any(contains_negative_power),
        Expr::Pow(base, _) => contains_negative_power(base),
        Expr::Sin(x) | Expr::Cos(x) | Expr::Exp(x) | Expr::Sinh(x) | Expr::Cosh(x) => contains_negative_power(x),
        Expr::Func { args, .. } => args.iter().any(contains_negative_power),
        Expr::Var(_) | Expr::Rational(_) => false,
    }
}

/// `accel_equations`' coefficient-vanishes check is provably
/// unreachable through `geodesic_equations`' real construction today
/// (see `accel_equations`' own doc comment: the acceleration term is
/// always added with a literal coefficient of 1, and the mandatory
/// parameter-collision check already rules out any Gamma component
/// containing a `coord''(param)`-shaped node) -- there is no metric or
/// connection to construct through the public `.od` surface that would
/// trip it. It stays a real, load-bearing, catchable check anyway
/// (unlike linearity below, a vanishing coefficient IS a genuine
/// property a future, more general metric/chart could have), so it is
/// exercised directly here against a hand-built equation, exactly the
/// way `accel_equations_checkpointed` itself calls
/// `solve_for_second_derivative` per coordinate -- not by pretending an
/// impossible metric exists.
#[test]
fn solve_for_second_derivative_refuses_a_coefficient_that_normalizes_to_zero() {
    // An equation for `r` that never mentions `r''(tau)` at all -- the
    // coefficient of a target that is simply absent is exactly zero,
    // the same as if some other computation had multiplied it away.
    let equation = Expr::var("M") * Expr::var("r");
    let err = oderom_components::curvature::solve_for_second_derivative(&equation, "r", "tau").unwrap_err();
    match err {
        ComponentError::GeodesicCoefficientVanishes { coordinate } => assert_eq!(coordinate, "r"),
        other => panic!("expected GeodesicCoefficientVanishes, got {other}"),
    }
}

/// Linearity in the acceleration, unlike the coefficient's vanishing,
/// is not a property of the metric/chart at all -- it is a mathematical
/// fact about what a geodesic equation IS. No legitimate input can
/// violate it, so `solve_for_second_derivative` treats a violation as a
/// crashing precondition failure (`.expect()`), never a `Result` a
/// caller could catch and paper over -- confirmed here via
/// `#[should_panic]` against a hand-built equation quadratic in its own
/// acceleration, the only way to exercise this path at all.
#[test]
#[should_panic(expected = "affine-linear in its own acceleration")]
fn solve_for_second_derivative_panics_on_an_equation_quadratic_in_its_own_acceleration() {
    let r_double_dot = Expr::Func { name: "r".to_string(), args: vec![Expr::var("tau")], order: vec![2] };
    let equation = r_double_dot.pow(2) + Expr::var("M");
    let _ = oderom_components::curvature::solve_for_second_derivative(&equation, "r", "tau");
}

// ---------------------------------------------------------------------
// Index-variance change (Rodada Variancia): change_variance's own
// component-level tests, against a real metric (Schwarzschild) rather
// than a synthetic hand-built grid -- the query grammar's
// `riemann [up,down,down,down]`-style tests (oderom-cli/oderom-session)
// exercise the same primitives from the query surface; these confirm
// the underlying arithmetic directly.
// ---------------------------------------------------------------------

/// Two `Expr`s denote the same value even when `normalize()` doesn't
/// give them the identical tree -- `normalize()` is a canonical form for
/// a SINGLE expression tree it is handed, not a proof that two
/// *independently derived* (different path, same math) expressions
/// collapse to byte-identical structure (a sign can end up folded into
/// the numerator on one path and the denominator on the other, e.g.
/// `M/(2M-r)` vs. `-M/(-2M+r)` -- equal, not equal-looking). The
/// standard, robust check either way: their difference normalizes to
/// zero. Same technique this project's own `kerr.rs`/`godel.rs` golden
/// tests already use (`normalize(&(ricci.get(...))).is_zero()`), applied
/// to a difference instead of a bare component.
fn assert_same_value(a: &Expr, b: &Expr, what: &str) {
    let diff = normalize(&(a.clone() - b.clone()));
    assert!(diff.is_zero(), "{what}: values differ (difference = {diff:?}); a={a:?} b={b:?}");
}

/// Round trip: raising Riemann's first index then lowering it straight
/// back must reproduce the original fully covariant tensor's VALUE,
/// component by component -- a strong, cheap check (DESIGN-M2.md's own
/// testing style: prefer a real algebraic identity over a hand-picked
/// expected value) that `change_variance`'s raise/lower pair are genuine
/// inverses of each other, not just individually plausible.
#[test]
fn raising_riemanns_first_index_then_lowering_it_back_reproduces_the_covariant_tensor() {
    let s = build().unwrap();
    let current = vec![Variance::Co; 4];
    let raised = vec![Variance::Contra, Variance::Co, Variance::Co, Variance::Co];
    let up = change_variance(&s.registry, &s.chart, &s.riem_cov, &current, &raised, &s.g, &s.ginv).unwrap();
    let back_down = change_variance(&s.registry, &s.chart, &up, &raised, &current, &s.g, &s.ginv).unwrap();

    let n = s.chart.dim() as u8;
    for a in 0..n {
        for b in 0..n {
            for c in 0..n {
                for d in 0..n {
                    let idx = [a, b, c, d];
                    assert_same_value(&back_down.get(&idx), &s.riem_cov.get(&idx), &format!("R_{{{a}{b}{c}{d}}} round-trip"));
                }
            }
        }
    }
}

/// `change_variance`'s raise of Riemann's first index agrees, in VALUE,
/// with the independently-computed `riemann_mixed` (built straight from
/// Christoffel, never touching `change_variance` at all) -- two
/// unrelated computational paths to the same mixed tensor agreeing is a
/// stronger check than either alone.
#[test]
fn change_variance_raising_riemanns_first_index_matches_riemann_mixed_computed_independently() {
    let s = build().unwrap();
    let current = vec![Variance::Co; 4];
    let raised = vec![Variance::Contra, Variance::Co, Variance::Co, Variance::Co];
    let via_change_variance = change_variance(&s.registry, &s.chart, &s.riem_cov, &current, &raised, &s.g, &s.ginv).unwrap();
    // Same result `raise_index` alone would give directly (this one IS
    // structurally identical -- `change_variance` just dispatches to
    // `raise_index_checkpointed`, no separate arithmetic of its own),
    // and, more importantly, the same VALUE as `riemann_mixed` itself (a
    // completely different derivation, straight from Christoffel,
    // computed inside `build()` before this test ever runs).
    let via_raise_index = raise_index(&s.chart, &s.riem_cov, &s.ginv, 0);

    let n = s.chart.dim() as u8;
    for a in 0..n {
        for b in 0..n {
            for c in 0..n {
                for d in 0..n {
                    let idx = [a, b, c, d];
                    let via_cv = normalize(&via_change_variance.get(&idx));
                    assert_eq!(via_cv, normalize(&via_raise_index.get(&idx)), "R^{{{a}}}_{{{b}{c}{d}}}: change_variance disagrees with raise_index");
                    assert_same_value(&via_cv, &s.riem_mixed.get(&idx), &format!("R^{{{a}}}_{{{b}{c}{d}}}: change_variance vs. independently-derived riemann_mixed"));
                }
            }
        }
    }
}

/// The trace test: raising Riemann's first index (via `change_variance`,
/// the same operation `riemann [up,down,down,down]` performs) and then
/// contracting it the way `ricci_tensor` always has (`R_bd = R^a_bad`)
/// reproduces `ricci`, Schwarzschild's own known-zero Ricci tensor --
/// verification independent of `raise_index`'s own correctness alone,
/// since it also exercises the downstream contraction.
#[test]
fn ricci_traced_from_the_change_variance_raised_riemann_matches_ricci_tensor() {
    let s = build().unwrap();
    let current = vec![Variance::Co; 4];
    let raised = vec![Variance::Contra, Variance::Co, Variance::Co, Variance::Co];
    let riemann_up_down_down_down = change_variance(&s.registry, &s.chart, &s.riem_cov, &current, &raised, &s.g, &s.ginv).unwrap();

    let ricci_from_raised = ricci_tensor(&s.chart, &riemann_up_down_down_down);
    let ricci_reference = ricci_tensor(&s.chart, &s.riem_mixed);

    let n = s.chart.dim() as u8;
    for b in 0..n {
        for d in 0..n {
            assert_same_value(
                &ricci_from_raised.get(&[b, d]),
                &ricci_reference.get(&[b, d]),
                &format!("R_{{{b}{d}}} traced from the change_variance-raised Riemann vs. ricci_tensor(riemann_mixed)"),
            );
            // Schwarzschild is vacuum: both should also just be zero.
            assert!(normalize(&ricci_from_raised.get(&[b, d])).is_zero(), "R_{{{b}{d}}} = 0 expected (vacuum)");
        }
    }
}

/// Vacuum sanity check: Schwarzschild's Ricci tensor is identically
/// zero, and raising/lowering its indices in any combination must stay
/// zero -- trivial algebraically (every term in the contraction is zero
/// times a metric component), but confirms `change_variance` introduces
/// no artifact that would turn a zero tensor nonzero.
#[test]
fn raising_or_lowering_schwarzschilds_zero_ricci_stays_zero_in_any_variance() {
    let s = build().unwrap();
    let ricci_cov = ricci_tensor(&s.chart, &s.riem_mixed);
    let base = vec![Variance::Co, Variance::Co];

    for pattern in [[Variance::Contra, Variance::Co], [Variance::Co, Variance::Contra], [Variance::Contra, Variance::Contra]] {
        let changed = change_variance(&s.registry, &s.chart, &ricci_cov, &base, &pattern, &s.g, &s.ginv).unwrap();
        for b in 0..s.chart.dim() as u8 {
            for d in 0..s.chart.dim() as u8 {
                let component = normalize(&changed.get(&[b, d]));
                assert!(component.is_zero(), "R_{{{b}{d}}} in variance {pattern:?} = {component:?}, expected 0 (vacuum, any variance)");
            }
        }
    }
}
