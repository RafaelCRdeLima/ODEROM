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

use oderom_components::curvature::{christoffel, lower_first_index, metric_inverse, metric_inverse_diagonal, metric_block_structure, ricci_tensor, riemann_mixed, verify_metric_inverse};
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
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
    g.set(&registry, &[3, 3], normalize(&((r.pow(2) + a.clone().pow(2)) * theta.sin().pow(2))))?;

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
