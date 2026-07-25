//! Godel's rotating-universe metric (Godel's own coordinates `t, x, y,
//! z`) -- the second non-diagonal fixture this round adds, deliberately
//! chosen for its unusual signature/coupling shape rather than
//! re-deriving Kerr's own frame-dragging structure: `g_ty` (not `g_tx`)
//! is the one off-diagonal component, so the coupled block is `{t,y}`,
//! and `y` itself also carries a coordinate-dependent (`e^x`) diagonal
//! entry unlike anything Kerr's own `{r,theta}` singletons have.
//!
//! `ds^2 = a^2 [ -(dt + e^x dy)^2 + dx^2 + (1/2) e^(2x) dy^2 + dz^2 ]`,
//! Godel's own original chart (Godel 1949; see also the Wikipedia
//! "Godel metric" article, same line element up to the parametrization
//! `a^2 = 1/(2*omega^2)`). Expanded:
//!
//! ```text
//! g_tt = -a^2
//! g_ty = -a^2 e^x
//! g_yy = -(1/2) a^2 e^(2x)
//! g_xx = a^2
//! g_zz = a^2
//! ```
//!
//! Known closed form, confirmed independently rather than assumed from
//! memory (Wikipedia's own Einstein-tensor line for this exact chart,
//! `G^ab_hat = omega^2 diag(-1,1,1,1) + 2 omega^2 diag(1,0,0,0)` in an
//! orthonormal frame, trace `= 2 omega^2`; the mostly-plus convention
//! this project's other fixtures already use gives `R = -trace(G) =
//! -2*omega^2`, and `a^2 = 1/(2*omega^2)` turns that into `R = -1/a^2`):
//! Godel's Ricci **scalar** is constant, `R = -1/a^2`, everywhere.
//! (Godel's Ricci *tensor* is NOT zero -- Godel is a dust+Lambda
//! solution, not vacuum, unlike Kerr -- so this fixture's own golden
//! check is the scalar invariant, not `R_ab = 0`.)

use oderom_components::curvature::{metric_block_structure, metric_inverse, ricci_scalar, ricci_tensor, riemann_mixed, verify_metric_inverse, christoffel};
use oderom_components::{Chart, ComponentError, ComponentTensor, Grid};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;

struct Godel {
    registry: Registry,
    chart: Chart,
    g: ComponentTensor,
    ginv: Grid,
    ricci: Grid,
}

fn build() -> Result<Godel, ComponentError> {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("M", 4).unwrap();
    let tm = registry.declare_bundle("TM", manifold, 4).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };
    let metric_slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let metric_head = registry.declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();

    // Coordinate order (t, x, y, z) -- indices 0,1,2,3 -- matching this
    // project's other 4D gallery fixtures' own chart order.
    let chart = Chart::new(["t", "x", "y", "z"]);
    let a = Expr::var("a");
    let x = Expr::var("x");
    let a2 = a.clone().pow(2);

    let mut g = ComponentTensor::new(metric_head);
    g.set(&registry, &[0, 0], normalize(&-a2.clone()))?; // g_tt = -a^2
    g.set(&registry, &[0, 2], normalize(&(-a2.clone() * x.clone().exp())))?; // g_ty = -a^2 e^x
    g.set(&registry, &[1, 1], normalize(&a2.clone()))?; // g_xx = a^2
    g.set(&registry, &[2, 2], normalize(&(-Expr::rational(1, 2) * a2.clone() * (Expr::int(2) * x.clone()).exp())))?; // g_yy = -(1/2) a^2 e^(2x)
    g.set(&registry, &[3, 3], normalize(&a2))?; // g_zz = a^2

    let ginv = metric_inverse(&registry, &chart, &g)?;
    let gamma = christoffel(&registry, &chart, &g, &ginv)?;
    let riem_mixed = riemann_mixed(&chart, &gamma);
    let ricci = ricci_tensor(&chart, &riem_mixed);

    Ok(Godel { registry, chart, g, ginv, ricci })
}

/// `{t,y}` coupled (Godel's own `g_ty` cross term), `{x}`/`{z}` each
/// singleton -- a genuinely different block shape from Kerr's own
/// `{t,phi}` (different coordinate pair, and `y`'s own diagonal entry
/// is coordinate-dependent unlike Kerr's constant-in-angle `r`/`theta`
/// singletons), confirming block detection isn't special-cased to
/// Kerr's particular layout.
#[test]
fn godel_metric_block_structure_is_the_t_y_pair_plus_two_singletons() {
    let g = build().unwrap();
    let mut blocks = metric_block_structure(&g.registry, &g.chart, &g.g).unwrap();
    for block in &mut blocks {
        block.sort_unstable();
    }
    blocks.sort_by_key(|b| b[0]);
    assert_eq!(blocks, vec![vec![0, 2], vec![1], vec![3]], "expected {{t,y}} coupled, x and z each their own singleton block, got {blocks:?}");
}

#[test]
fn godel_metric_inverse_satisfies_g_ginv_equals_identity() {
    let g = build().unwrap();
    verify_metric_inverse(&g.registry, &g.chart, &g.g, &g.ginv).expect("g_ab * g^bc must equal delta^a_c");
}

/// The golden check for this fixture: Godel's Ricci scalar is constant
/// and equals `-1/a^2` everywhere (mostly-plus convention -- see this
/// file's own module doc comment for the independent derivation) --
/// mathematically correct as written, and the test that will pass once
/// the normalizer can handle it. `#[ignore]`d: blocked by the
/// `exp(a)^n` power-fusion gap in `AtomTable::exp`
/// (`oderom-expr/src/poly.rs`) documented in full at
/// **DESIGN-RATIONAL-FORM.md section 7.2** -- that section is the
/// single source of truth for this limit; do not re-derive it here.
/// Clearing `#[ignore]` after the normalizer is extended is itself the
/// proof the fix worked.
#[test]
#[ignore] // blocked by DESIGN-RATIONAL-FORM.md section 7.2 (exp(a)^n never fuses into exp(n*a))
fn ricci_scalar_of_godel_is_minus_one_over_a_squared() {
    let g = build().unwrap();
    let scalar = ricci_scalar(&g.chart, &g.ricci, &g.ginv);
    let expected = normalize(&(Expr::int(-1) * Expr::Pow(Box::new(Expr::var("a").pow(2)), -1)));
    assert_eq!(scalar, expected, "R = {scalar:?}, expected -1/a^2 = {expected:?}");
}

/// Unlike Kerr, Godel is NOT vacuum -- its Ricci tensor is genuinely
/// nonzero (dust + negative cosmological constant, not empty space).
/// Confirms this fixture isn't accidentally testing a degenerate
/// (Ricci-flat) case the way a copy-pasted Kerr-style assertion would.
#[test]
fn ricci_tensor_of_godel_is_not_identically_zero() {
    let g = build().unwrap();
    let mut any_nonzero = false;
    for b in 0..g.chart.dim() as u8 {
        for d in 0..g.chart.dim() as u8 {
            if !normalize(&g.ricci.get(&[b, d])).is_zero() {
                any_nonzero = true;
            }
        }
    }
    assert!(any_nonzero, "Godel is a dust+Lambda solution, not vacuum -- R_ab should not be identically zero");
}
