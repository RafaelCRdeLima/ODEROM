//! Kerr (Boyer-Lindquist coordinates `t, r, theta, phi`) -- the fixture
//! this round's non-diagonal metric inversion exists to unlock. Kerr's
//! `g_t_phi` cross term (frame dragging) is genuinely off-diagonal, and
//! Kerr is vacuum (`R_ab = 0`): a non-trivial, closed-form-known result
//! that only comes out right if the whole chain -- non-diagonal
//! [`metric_inverse`], Christoffel built from it, Riemann, Ricci -- is
//! correct end to end.
//!
//! Was `kerr_non_diagonal_limitation.rs` (Rodada Galeria): pinned the
//! *old*, pre-this-round behavior (`metric_inverse_diagonal` rejecting
//! `g_t_phi` with `ComponentError::NonDiagonalMetric`) specifically so
//! that whichever round added general inversion would have to touch
//! this file -- see that commit's own doc comment. This is that
//! conversion, but NOT a full, unqualified win: **the metric inversion
//! itself is fast and correct** (`metric_block_structure` 1.7ms,
//! `metric_inverse` 29ms via the cheap 2x2 `{t,phi}` closed form) --
//! **`christoffel`/`riemann_mixed` downstream of it are not** (see
//! `ricci_of_kerr_is_identically_zero` below). Full writeup, single
//! source of truth: **DESIGN-RATIONAL-FORM.md, section 7.1**.
//!
//! `metric_inverse_diagonal` itself is untouched and still rejects
//! Kerr's off-diagonal `g_t_phi` the same way it always has (see
//! `metric_inverse_diagonal_still_refuses_kerrs_off_diagonal_component`
//! below) -- `metric_inverse` is the new general entry point real
//! callers use.

use oderom_components::curvature::{
    christoffel, christoffel_localized, kretschmann_localized, lower_first_index, lower_first_index_localized, metric_inverse, metric_inverse_diagonal, metric_block_structure,
    localization_generators, ricci_tensor, ricci_tensor_localized, riemann_mixed, riemann_mixed_localized, verify_metric_inverse,
};
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr, LocalizationContext};
use smallvec::SmallVec;

struct Kerr {
    registry: Registry,
    chart: Chart,
    g: ComponentTensor,
    ginv: Grid,
}

/// Boyer-Lindquist coordinates (t, r, theta, phi); `Sigma = r^2 +
/// a^2*cos(theta)^2`, `Delta = r^2 - 2*M*r + a^2` -- the standard Kerr
/// metric, `g_t_phi` its one genuinely off-diagonal component:
///
/// ```text
/// g_tt     = -(1 - 2Mr/Sigma)
/// g_t_phi  = -2Mar sin^2(theta)/Sigma
/// g_rr     = Sigma/Delta
/// g_theta  = Sigma
/// g_phi    = (r^2+a^2) sin^2(theta)
/// ```
///
/// Deliberately cheap: only builds `g` and inverts it (both fast,
/// measured in microseconds/milliseconds -- see this file's own module
/// doc comment). Does NOT compute Christoffel/Riemann/Ricci -- callers
/// that need those do so themselves, explicitly, so that tests which
/// only need `g`/`ginv` (block structure, `g*ginv=I`) stay fast and
/// don't silently inherit the expensive downstream pipeline's cost.
fn build() -> Result<Kerr, ComponentError> {
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
    g.set(&registry, &[0, 0], normalize(&-(Expr::int(1) - Expr::int(2) * m.clone() * r.clone() * Expr::Pow(Box::new(sigma.clone()), -1))))?;
    g.set(
        &registry,
        &[0, 3],
        normalize(&(-Expr::int(2) * m.clone() * a.clone() * r.clone() * theta.clone().sin().pow(2) * Expr::Pow(Box::new(sigma.clone()), -1))),
    )?;
    g.set(&registry, &[1, 1], normalize(&(sigma.clone() * Expr::Pow(Box::new(delta), -1))))?;
    g.set(&registry, &[2, 2], normalize(&sigma))?;
    // g_phi_phi carries the frame-dragging correction term
    // `2*M*a^2*r*sin(theta)^2/Sigma` alongside `r^2+a^2` -- dropping it
    // (as an earlier version of this fixture did) changes the metric
    // determinant away from the known `-Sigma^2*sin(theta)^2` and
    // breaks Ricci-flatness; see examples/kerr.od's own header for the
    // determinant check that caught this.
    g.set(
        &registry,
        &[3, 3],
        normalize(
            &((r.clone().pow(2) + a.clone().pow(2) + Expr::int(2) * m.clone() * a.pow(2) * r.clone() * theta.clone().sin().pow(2) * Expr::Pow(Box::new(sigma.clone()), -1))
                * theta.sin().pow(2)),
        ),
    )?;

    let ginv = metric_inverse(&registry, &chart, &g)?;

    Ok(Kerr { registry, chart, g, ginv })
}

/// The old, pre-this-round behavior stays reachable and unchanged:
/// `metric_inverse_diagonal` (the diagonal-only fast path
/// [`metric_inverse`] dispatches to for an all-diagonal metric, and only
/// for one) still refuses Kerr's off-diagonal `g_t_phi` exactly as
/// before -- nobody silently broadened what that specific function
/// does.
#[test]
fn metric_inverse_diagonal_still_refuses_kerrs_off_diagonal_component() {
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

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&-(Expr::int(1) - Expr::int(2) * m.clone() * r.clone() * Expr::Pow(Box::new(sigma.clone()), -1))))
        .unwrap();
    g.set(
        &registry,
        &[0, 3],
        normalize(&(-Expr::int(2) * m * a * r * theta.sin().pow(2) * Expr::Pow(Box::new(sigma), -1))),
    )
    .unwrap();

    let err = metric_inverse_diagonal(&registry, &chart, &g).unwrap_err();
    assert!(matches!(err, ComponentError::NonDiagonalMetric { i: 0, j: 3 }), "expected the documented off-diagonal rejection naming g[0,3], got {err:?}");
}

/// `metric_inverse` recognizes Kerr's `{t,phi}` block: everything else
/// (`r`, `theta`) is a singleton block -- not a slow, blind fallback to
/// the fully general (4x4 determinant) path. Measured at 1.7ms
/// (`diagnostic_kerr.rs`).
#[test]
fn kerr_metric_block_structure_is_the_t_phi_pair_plus_two_singletons() {
    let k = build().unwrap();
    let mut blocks = metric_block_structure(&k.registry, &k.chart, &k.g).unwrap();
    for block in &mut blocks {
        block.sort_unstable();
    }
    blocks.sort_by_key(|b| b[0]);
    assert_eq!(blocks, vec![vec![0, 3], vec![1], vec![2]], "expected {{t,phi}} coupled, r and theta each their own singleton block, got {blocks:?}");
}

/// `g_ab g^bc = delta^a_c` for Kerr's own non-diagonal inverse --
/// independent of the golden Ricci test below, and the check that would
/// catch a block-detection bug specifically (see
/// `ComponentError::InverseVerificationFailed`'s own doc comment).
/// Measured at 29ms total for `metric_inverse` itself (`diagnostic_kerr.rs`).
#[test]
fn kerr_metric_inverse_satisfies_g_ginv_equals_identity() {
    let k = build().unwrap();
    verify_metric_inverse(&k.registry, &k.chart, &k.g, &k.ginv).expect("g_ab * g^bc must equal delta^a_c");
}

/// The golden test this round's whole premise is about: Kerr is vacuum,
/// `R_ab = 0` identically -- mathematically correct as written, and the
/// test that will pass once the normalizer can handle it. `#[ignore]`d:
/// blocked by the multivariate-GCD limit (no single "pole variable" for
/// Kerr's `Sigma`) documented in full at **DESIGN-RATIONAL-FORM.md
/// section 7.1** -- that section is the single source of truth for this
/// limit; do not re-derive it here. Clearing `#[ignore]` after the
/// normalizer is extended is itself the proof the fix worked. Run
/// explicitly, with patience (many minutes, unbounded), via:
///
/// ```text
/// cargo test -p oderom-components --release --test kerr -- --ignored ricci_of_kerr_is_identically_zero
/// ```
#[test]
#[ignore] // blocked by DESIGN-RATIONAL-FORM.md section 7.1 (Kerr's bivariate Sigma denominator, no single pole variable)
fn ricci_of_kerr_is_identically_zero() {
    let k = build().unwrap();
    let gamma = christoffel(&k.registry, &k.chart, &k.g, &k.ginv).unwrap();
    let riem_mixed = riemann_mixed(&k.chart, &gamma);
    let ricci = ricci_tensor(&k.chart, &riem_mixed);
    for b in 0..k.chart.dim() as u8 {
        for d in 0..k.chart.dim() as u8 {
            let component = normalize(&ricci.get(&[b, d]));
            assert!(component.is_zero(), "R_{{{b}{d}}} = {component:?}, expected 0 (Kerr is vacuum)");
        }
    }
}

/// The same golden check as `ricci_of_kerr_is_identically_zero` above,
/// but through the structured-denominator engine
/// (DESIGN-RATIONAL-FORM.md section 8, `oderom_expr::{LocalizationContext,
/// normalize_localized}`) instead of the general recursive multivariate
/// GCD that test is blocked on. Not `#[ignore]`d: this is the actual fix
/// for section 7.1, not a restatement of the limit.
#[test]
fn ricci_of_kerr_is_identically_zero_via_the_localized_engine() {
    let k = build().unwrap();
    let seeds = localization_generators(&k.registry, &k.chart, &k.g).unwrap();
    let mut ctx = LocalizationContext::new(&seeds);
    let gamma = christoffel_localized(&k.registry, &k.chart, &k.g, &k.ginv, &mut ctx).unwrap();
    let riem_mixed = riemann_mixed_localized(&k.chart, &gamma, &mut ctx).unwrap();
    let ricci = ricci_tensor_localized(&k.chart, &riem_mixed, &mut ctx).unwrap();
    for b in 0..k.chart.dim() as u8 {
        for d in 0..k.chart.dim() as u8 {
            let component = ricci.get(&[b, d]);
            assert!(component.is_zero(), "R_{{{b}{d}}} = {component:?}, expected 0 (Kerr is vacuum)");
        }
    }
    // Structural, not timing: a fallback is the correct-but-slow path
    // (`RationalFunction`'s general engine), and a future regression
    // that silently routes more expressions through it would not fail
    // this test's own value assertions above -- Ricci would still come
    // out to zero, just slower, exactly the failure mode
    // `oderom-expr/src/localized.rs`'s own `add()` fast-path bug had
    // before it was found (28ms became well past 120s with no assertion
    // anywhere noticing). Asserting the fallback count directly turns
    // "got slow" into "test fails", which is the property that actually
    // matters. Zero is achievable here (not merely "small"): the
    // repeated-factor recovery in `classify_or_admit`
    // (`oderom-expr/src/localized.rs`) resolves `sin(theta)^2` into
    // `sin(theta)` even when encountered before any bare `sin(theta)`
    // denominator has been admitted.
    assert!(ctx.fallback_log().is_empty(), "expected zero fallbacks to the general engine, got: {:?}", ctx.fallback_log());
}

/// Kerr's Kretschmann scalar via the localized engine, matching the
/// closed form named in `examples/kerr.od`'s own header:
///
/// ```text
/// K = 48*M^2*(r^2-a^2*cos(theta)^2)*((r^2+a^2*cos(theta)^2)^2
///     - 16*r^2*a^2*cos(theta)^2) / (r^2+a^2*cos(theta)^2)^6
/// ```
///
/// The other half of this round's completion criterion alongside
/// `ricci_of_kerr_is_identically_zero_via_the_localized_engine` above --
/// together these two are what clearing `#[ignore]` on the original,
/// general-engine-blocked tests in this file is standing in for.
#[test]
fn kretschmann_of_kerr_matches_the_closed_form_via_the_localized_engine() {
    let k = build().unwrap();
    let seeds = localization_generators(&k.registry, &k.chart, &k.g).unwrap();
    let mut ctx = LocalizationContext::new(&seeds);
    let gamma = christoffel_localized(&k.registry, &k.chart, &k.g, &k.ginv, &mut ctx).unwrap();
    let riem_mixed = riemann_mixed_localized(&k.chart, &gamma, &mut ctx).unwrap();
    let riem_cov = lower_first_index_localized(&k.registry, &k.chart, &riem_mixed, &k.g, &mut ctx).unwrap();
    let kretschmann = kretschmann_localized(&k.chart, &riem_cov, &k.ginv, &mut ctx).unwrap();

    let m = Expr::var("M");
    let a = Expr::var("a");
    let r = Expr::var("r");
    let theta = Expr::var("theta");
    let sigma = r.clone().pow(2) + a.clone().pow(2) * theta.clone().cos().pow(2);
    let expected = normalize(
        &(Expr::int(48) * m.pow(2) * (r.clone().pow(2) - a.clone().pow(2) * theta.clone().cos().pow(2)) * (sigma.clone().pow(2) - Expr::int(16) * r.pow(2) * a.pow(2) * theta.cos().pow(2))
            * Expr::Pow(Box::new(sigma), -6)),
    );
    assert_eq!(kretschmann, expected, "kretschmann={kretschmann:?}\nexpected={expected:?}");
    // Same structural (not timing) regression guard as the Ricci test
    // above: a fallback that silently reappeared here wouldn't fail
    // this assertion's value check, only make it slower.
    assert!(ctx.fallback_log().is_empty(), "expected zero fallbacks to the general engine, got: {:?}", ctx.fallback_log());
}

/// `lower_first_index` (fully covariant Riemann) exercised through
/// Kerr's non-diagonal `g` at least once. `#[ignore]`d for the same
/// reason as `ricci_of_kerr_is_identically_zero` above -- it depends on
/// the same expensive `riemann_mixed` stage.
#[test]
#[ignore]
fn riemann_covariant_of_kerr_lowers_cleanly_through_the_non_diagonal_metric() {
    let k = build().unwrap();
    let gamma = christoffel(&k.registry, &k.chart, &k.g, &k.ginv).unwrap();
    let riem_mixed = riemann_mixed(&k.chart, &gamma);
    let riem_cov = lower_first_index(&k.registry, &k.chart, &riem_mixed, &k.g).unwrap();
    // Not identically zero (Kerr is curved, just Ricci-flat): sanity
    // check that lowering didn't accidentally zero everything out.
    let mut any_nonzero = false;
    for a in 0..4u8 {
        for b in 0..4u8 {
            for c in 0..4u8 {
                for d in 0..4u8 {
                    if !normalize(&riem_cov.get(&[a, b, c, d])).is_zero() {
                        any_nonzero = true;
                    }
                }
            }
        }
    }
    assert!(any_nonzero, "Kerr's fully covariant Riemann tensor should not be identically zero");
}

/// Diagnostic (not an acceptance test -- prints instead of asserting),
/// same discipline as `diagnostic_kerr.rs`. Measured once (release,
/// this machine): `christoffel_localized` 28ms, `riemann_mixed_localized`
/// 672ms, `ricci_tensor_localized` 7.5ms -- under 1s total, against the
/// general engine's 70.5s/never-finishes-in-180s for the same two
/// stages (`diagnostic_kerr.rs`'s own numbers). Run with:
/// `cargo test -p oderom-components --release --test kerr -- --ignored
/// --nocapture diagnostic_localized_engine_stage_timing`.
#[test]
#[ignore]
fn diagnostic_localized_engine_stage_timing() {
    use std::time::Instant;
    let k = build().unwrap();
    let seeds = localization_generators(&k.registry, &k.chart, &k.g).unwrap();
    use std::io::Write;
    macro_rules! flushed {
        ($($arg:tt)*) => {{
            println!($($arg)*);
            std::io::stdout().flush().unwrap();
        }};
    }
    flushed!("seeds: {:?}", seeds.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    let mut ctx = LocalizationContext::new(&seeds);
    flushed!("generator_count after seeding: {}", ctx.generator_count());

    let t0 = Instant::now();
    let gamma = christoffel_localized(&k.registry, &k.chart, &k.g, &k.ginv, &mut ctx).unwrap();
    flushed!("christoffel_localized: {:?}  (generators now: {})", t0.elapsed(), ctx.generator_count());
    flushed!("fallback_log after christoffel: {:?}", ctx.fallback_log());

    let t0 = Instant::now();
    let riem_mixed = riemann_mixed_localized(&k.chart, &gamma, &mut ctx).unwrap();
    flushed!("riemann_mixed_localized: {:?}  (generators now: {})", t0.elapsed(), ctx.generator_count());
    flushed!("fallback_log after riemann_mixed: {:?}", ctx.fallback_log());

    let t0 = Instant::now();
    let ricci = ricci_tensor_localized(&k.chart, &riem_mixed, &mut ctx).unwrap();
    flushed!("ricci_tensor_localized: {:?}", t0.elapsed());

    for b in 0..k.chart.dim() as u8 {
        for d in 0..k.chart.dim() as u8 {
            println!("R_{{{b}{d}}} = {}", ricci.get(&[b, d]));
        }
    }
}

/// The execution-budget guardrail (DESIGN-RATIONAL-FORM.md section 8,
/// Phase 1) actually firing through the real curvature pipeline, not
/// just at the `oderom-expr` unit level
/// (`oderom_expr::localized::tests::the_execution_budget_actually_fires_at_the_fallback_boundary`
/// covers that precisely and deterministically already). A synthetic
/// 2D metric, not Kerr itself: Kerr now falls back zero times (the
/// whole point of this round's fixes), so it can't exercise this path
/// any more -- this fixture's `g_00 = 1/(x-1)^3` is deliberately
/// unresolvable by one repeated-factor extraction (same shape as
/// `oderom-expr`'s own `(x-1)^3` test), seeded with no generators at
/// all, so `christoffel_localized_checkpointed` is guaranteed to hit
/// the general-engine fallback while computing `Gamma^0_00`.
#[test]
fn christoffel_localized_reports_the_escaped_denominator_when_the_budget_runs_out() {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("M", 2).unwrap();
    let tm = registry.declare_bundle("TM", manifold, 2).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 2 };
    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry.declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();
    let chart = Chart::new(["x", "y"]);
    let x = Expr::var("x");
    let cubed = (x.clone() - Expr::one()).pow(3);
    let g_00 = normalize(&Expr::Pow(Box::new(cubed), -1));

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], g_00).unwrap();
    g.set(&registry, &[1, 1], Expr::one()).unwrap();
    let ginv = metric_inverse(&registry, &chart, &g).unwrap();

    let mut ctx = LocalizationContext::new(&[]);
    let mut calls = 0u32;
    let mut checkpoint = move || {
        calls += 1;
        calls > 2
    };
    let err = oderom_components::curvature::christoffel_localized_checkpointed(&registry, &chart, &g, &ginv, &mut ctx, &mut checkpoint).expect_err("a (x-1)^3-shaped denominator must force the general-engine fallback, which the tripped checkpoint must then catch");

    let message = err.to_string();
    match &err {
        ComponentError::LocalizationFallbackBudgetExceeded { component, denominator, generators_rendered } => {
            assert!(component.contains("Gamma"), "expected the offending component named, got {component:?}");
            let rendered_denominator = denominator.to_string();
            assert!(rendered_denominator.contains('x'), "expected the escaped (x-1)-shaped denominator, got {rendered_denominator:?}");
            assert!(generators_rendered.is_empty(), "no generator was ever seeded or admitted in this test, got {generators_rendered:?}");
            // The Display message (what a user actually sees) must
            // surface both pieces, not just carry them as unused struct
            // fields never actually rendered.
            assert!(message.contains(component.as_str()), "{message}");
            assert!(message.contains(&rendered_denominator), "{message}");
        }
        other => panic!("expected LocalizationFallbackBudgetExceeded, got {other:?}"),
    }
}
