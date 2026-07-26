//! Christoffel symbols, Riemann and Ricci tensors, and curvature scalars,
//! computed from a metric's components by the standard formulas:
//!
//! ```text
//! Gamma^a_bc = 1/2 g^ad (d_b g_dc + d_c g_db - d_d g_bc)
//! R^a_bcd    = d_c Gamma^a_bd - d_d Gamma^a_bc
//!              + Gamma^a_ce Gamma^e_bd - Gamma^a_de Gamma^e_bc
//! R_bd       = R^a_bad                              (Ricci tensor)
//! R          = g^bd R_bd                            (Ricci scalar)
//! ```
//!
//! The only real design choice here is inverting the metric: Marco 2
//! only handles a **diagonal** metric (inverse = reciprocal down the
//! diagonal), which is all the Schwarzschild acceptance test needs.
//! General symbolic cofactor inversion is future work (see DESIGN-M2.md,
//! D-M2.1) -- [`metric_inverse_diagonal`] errors on any nonzero
//! off-diagonal component rather than silently ignoring it.

use crate::chart::Chart;
use crate::error::ComponentError;
use crate::grid::Grid;
use crate::tensor::ComponentTensor;
use oderom_core::{HeadId, Registry, Variance};
use oderom_expr::{diff, isolate_linear, normalize, normalize_localized, rationalize, LocalizationContext, Expr};

/// `f` returning `Err` (a checkpoint cancelling, or a genuine
/// `ComponentTensor::get` error in [`lower_first_index`]) stops the walk
/// immediately -- propagated out through every level of `rec`'s own
/// recursion via `?`, not a captured-and-checked-afterward flag the way
/// an earlier version of [`lower_first_index`] had to do because this
/// function used to be infallible.
fn for_each_index_tuple(
    dim: usize,
    rank: usize,
    mut f: impl FnMut(&[u8]) -> Result<(), ComponentError>,
) -> Result<(), ComponentError> {
    fn rec(
        dim: usize,
        rank: usize,
        pos: usize,
        idx: &mut Vec<u8>,
        f: &mut impl FnMut(&[u8]) -> Result<(), ComponentError>,
    ) -> Result<(), ComponentError> {
        if pos == rank {
            return f(idx);
        }
        for v in 0..dim as u8 {
            idx[pos] = v;
            rec(dim, rank, pos + 1, idx, f)?;
        }
        Ok(())
    }
    let mut idx = vec![0u8; rank];
    rec(dim, rank, 0, &mut idx, &mut f)
}

/// Checked once per independent component inside every stage function
/// below that has a `_checkpointed` variant -- lets a caller interrupt
/// mid-stage (the CLI's own timeout today; `oderom-session`'s
/// `cancel_running()`, DESIGN-UI-SESSION.md, tomorrow) instead of only
/// between whole stages. Returns `true` to cancel. The plain,
/// non-`_checkpointed` functions pass `&mut || false` ("never
/// cancelled") and keep their exact original signature and behavior --
/// see each one's doc comment for why that `.expect()`/direct
/// `Result` unwrap is always safe.
pub type Checkpoint<'a> = &'a mut dyn FnMut() -> bool;

/// `g^ab`, as a `Grid` (the inverse of a tensor is not itself a tensor
/// with a reusable declared symmetry the way `g` is, so it isn't stored
/// as a `ComponentTensor` -- though it happens to be symmetric too, and a
/// future version could).
pub fn metric_inverse_diagonal(
    registry: &Registry,
    chart: &Chart,
    g: &ComponentTensor,
) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let mut inv = Grid::new(n, 2);
    for i in 0..n as u8 {
        for j in 0..n as u8 {
            if i != j {
                let off_diag = normalize(&g.get(registry, &[i, j])?);
                if !off_diag.is_zero() {
                    return Err(ComponentError::NonDiagonalMetric { i, j });
                }
            }
        }
    }
    for i in 0..n as u8 {
        let g_ii = g.get(registry, &[i, i])?;
        inv.set(&[i, i], normalize(&Expr::Pow(Box::new(g_ii), -1)));
    }
    Ok(inv)
}

/// `g`'s components, normalized, as a plain `n x n` matrix -- the shared
/// starting point for [`metric_block_structure`] and every non-diagonal
/// inversion path below, so the "is this off-diagonal entry really
/// nonzero" question is answered by [`normalize`] exactly once per pair,
/// the same way [`metric_inverse_diagonal`]'s own off-diagonal check
/// already does.
fn normalized_matrix(registry: &Registry, chart: &Chart, g: &ComponentTensor) -> Result<Vec<Vec<Expr>>, ComponentError> {
    let n = chart.dim();
    let mut mat = vec![vec![Expr::zero(); n]; n];
    for i in 0..n as u8 {
        for j in 0..n as u8 {
            mat[i as usize][j as usize] = normalize(&g.get(registry, &[i, j])?);
        }
    }
    Ok(mat)
}

/// Partitions `g`'s coordinate indices into the coarsest decoupled
/// blocks its off-diagonal structure actually supports: two indices land
/// in the same block iff a chain of pairwise-nonzero off-diagonal
/// entries connects them (union-find over the "is `g[i,j]` nonzero"
/// graph). Every entry ever declared diagonal produces `n` singleton
/// blocks -- the same partition [`metric_inverse_checkpointed`] uses to
/// recognize "dispatch to the untouched [`metric_inverse_diagonal`] fast
/// path" and stay byte-for-byte as fast as before this round for every
/// metric that was already diagonal.
///
/// **Why this can't misclassify a coupled pair as decoupled** (the one
/// failure mode that would make block inversion silently wrong, see
/// `ComponentError::InverseVerificationFailed`'s own doc comment): a
/// block boundary only exists where *every* cross entry is
/// `normalize()`-zero. Any single nonzero cross entry, however small or
/// simple-looking, merges the two blocks via a union -- there is no
/// threshold, no "probably small enough to ignore" step. The only way
/// this could go wrong is [`normalize`] itself failing to recognize a
/// genuinely-zero expression as zero (which only ever costs speed, by
/// falling through to the slower general path on an entry that could
/// have been treated as diagonal) or failing to recognize a
/// genuinely-nonzero one (which cannot happen: `normalize` never
/// discards a nonzero contribution, only rewrites it).
pub fn metric_block_structure(registry: &Registry, chart: &Chart, g: &ComponentTensor) -> Result<Vec<Vec<u8>>, ComponentError> {
    let n = chart.dim();
    let mat = normalized_matrix(registry, chart, g)?;

    let mut parent: Vec<u8> = (0..n as u8).collect();
    fn find(parent: &mut [u8], x: u8) -> u8 {
        if parent[x as usize] != x {
            let root = find(parent, parent[x as usize]);
            parent[x as usize] = root;
        }
        parent[x as usize]
    }

    for i in 0..n {
        for j in (i + 1)..n {
            if !mat[i][j].is_zero() {
                let ri = find(&mut parent, i as u8);
                let rj = find(&mut parent, j as u8);
                if ri != rj {
                    parent[ri as usize] = rj;
                }
            }
        }
    }

    let mut blocks: std::collections::BTreeMap<u8, Vec<u8>> = Default::default();
    for i in 0..n as u8 {
        let root = find(&mut parent, i);
        blocks.entry(root).or_default().push(i);
    }
    Ok(blocks.into_values().collect())
}

/// Removes row `skip_row` and column `skip_col` from `mat` -- the minor
/// used by both [`determinant_symbolic`]'s own Laplace expansion and the
/// cofactor matrix [`invert_symbolic_block`] builds from it.
fn minor_matrix(mat: &[Vec<Expr>], skip_row: usize, skip_col: usize) -> Vec<Vec<Expr>> {
    mat.iter()
        .enumerate()
        .filter(|(r, _)| *r != skip_row)
        .map(|(_, row)| row.iter().enumerate().filter(|(c, _)| *c != skip_col).map(|(_, v)| v.clone()).collect())
        .collect()
}

/// `det(mat)` for a general (not-assumed-diagonal) square symbolic
/// matrix, by Laplace expansion along the first row -- correct for any
/// `mat`, deliberately not optimized beyond normalizing each recursive
/// sub-determinant as soon as it's built (the same "reduce during the
/// contraction, not after" discipline `kretschmann_checkpointed` already
/// uses, DESIGN-RATIONAL-FORM.md section 2.5) rather than only once at
/// the very end -- correctness first (D-M2.1 revisited), not premature
/// optimization of the general case, which the block decomposition above
/// already keeps off the hot path for every metric this project's own
/// gallery uses.
fn determinant_symbolic(mat: &[Vec<Expr>], checkpoint: Checkpoint) -> Result<Expr, ComponentError> {
    let n = mat.len();
    match n {
        1 => Ok(mat[0][0].clone()),
        2 => Ok(normalize(&(mat[0][0].clone() * mat[1][1].clone() - mat[0][1].clone() * mat[1][0].clone()))),
        _ => {
            let mut sum = Expr::zero();
            for j in 0..n {
                if checkpoint() {
                    return Err(ComponentError::Cancelled);
                }
                let minor = minor_matrix(mat, 0, j);
                let term = mat[0][j].clone() * determinant_symbolic(&minor, checkpoint)?;
                sum = sum + if j % 2 == 1 { -term } else { term };
            }
            Ok(normalize(&sum))
        }
    }
}

/// Inverts one decoupled block (a genuinely square, dense sub-matrix --
/// `mat[i][j]` for `i,j` both drawn from the same
/// [`metric_block_structure`] block) by symbolic size, from cheapest to
/// most general:
///
/// - size 1: the existing reciprocal -- callers never actually reach
///   this arm for an all-singleton partition ([`metric_inverse_checkpointed`]
///   dispatches those to [`metric_inverse_diagonal`] directly instead),
///   kept here only so a size-1 block that shows up *alongside* a larger
///   one (e.g. Kerr's own `{r}`/`{theta}` singletons next to its `{t,phi}`
///   pair) still goes through the same per-block loop uniformly.
/// - size 2: the textbook closed form (`[[d,-b],[-c,a]]/det` for
///   `[[a,b],[c,d]]`) -- Kerr's `{t,phi}` block, and the whole reason
///   this stays cheap instead of falling through to a full 4x4
///   determinant.
/// - size >= 3: general adjugate/cofactor (transpose of the cofactor
///   matrix, divided by the determinant) via [`determinant_symbolic`] --
///   correct for any block, the fallback this round adds for a metric
///   whose coupling doesn't decompose into anything smaller (including
///   the degenerate case of one block covering the whole metric, i.e. no
///   usable block structure at all).
fn invert_symbolic_block(block: &[u8], mat: &[Vec<Expr>], checkpoint: Checkpoint) -> Result<Vec<Vec<Expr>>, ComponentError> {
    let n = mat.len();
    match n {
        1 => {
            let d = mat[0][0].clone();
            if d.is_zero() {
                return Err(ComponentError::SingularMetric { index: block[0] });
            }
            Ok(vec![vec![normalize(&Expr::Pow(Box::new(d), -1))]])
        }
        2 => {
            let det = normalize(&(mat[0][0].clone() * mat[1][1].clone() - mat[0][1].clone() * mat[1][0].clone()));
            if det.is_zero() {
                return Err(ComponentError::SingularMetric { index: block[0] });
            }
            let inv00 = normalize(&(mat[1][1].clone() / det.clone()));
            let inv01 = normalize(&(-mat[0][1].clone() / det.clone()));
            let inv10 = normalize(&(-mat[1][0].clone() / det.clone()));
            let inv11 = normalize(&(mat[0][0].clone() / det));
            Ok(vec![vec![inv00, inv01], vec![inv10, inv11]])
        }
        _ => {
            let det = determinant_symbolic(mat, checkpoint)?;
            if det.is_zero() {
                return Err(ComponentError::SingularMetric { index: block[0] });
            }
            let mut inv = vec![vec![Expr::zero(); n]; n];
            for i in 0..n {
                for j in 0..n {
                    if checkpoint() {
                        return Err(ComponentError::Cancelled);
                    }
                    let minor = minor_matrix(mat, i, j);
                    let cofactor = determinant_symbolic(&minor, checkpoint)?;
                    let signed = if (i + j) % 2 == 1 { -cofactor } else { cofactor };
                    // Adjugate is the cofactor matrix's TRANSPOSE: inv[j][i], not inv[i][j].
                    inv[j][i] = normalize(&(signed / det.clone()));
                }
            }
            Ok(inv)
        }
    }
}

/// `g^ab`, general: detects `g`'s block structure
/// ([`metric_block_structure`]) and dispatches per block, from fastest
/// to most general -- see this function's `_checkpointed` sibling for
/// the full three-tier strategy (diagonal / block-decoupled / general)
/// and why detection can't silently misclassify a coupled pair as
/// decoupled. [`metric_inverse_diagonal`] remains the dedicated fast
/// path for the all-diagonal case and is unchanged by this round; this
/// function is what every real caller (`oderom-cli::commands::build_from_metric`)
/// should call instead, so a metric that happens to be non-diagonal
/// (Kerr, Godel, a perturbed background) is inverted correctly rather
/// than rejected.
pub fn metric_inverse(registry: &Registry, chart: &Chart, g: &ComponentTensor) -> Result<Grid, ComponentError> {
    metric_inverse_checkpointed(registry, chart, g, &mut || false)
}

/// Same as [`metric_inverse`], but calls `checkpoint` once per block
/// (block/general path) and, within a block of size >= 3, once per
/// cofactor entry and once per recursive determinant term
/// ([`determinant_symbolic`]) -- the full symbolic inversion of a dense
/// matrix is exactly the kind of new, potentially expensive computation
/// path DESIGN-M6-PREP.md section 4's rule requires a checkpoint for
/// from its first version.
///
/// Three tiers, cheapest to most general:
/// 1. **All-diagonal** (every [`metric_block_structure`] block is a
///    singleton): dispatches straight to [`metric_inverse_diagonal`],
///    unchanged -- every metric that was fast before this round stays on
///    that exact code path, not a new one that merely behaves the same.
/// 2. **Block-decoupled** (Kerr's `{t,phi}` pair alongside singleton
///    `{r}`/`{theta}` blocks; Godel's `{t,y}` pair alongside `{x}`/`{z}`):
///    each block inverted independently via [`invert_symbolic_block`],
///    cross-block entries left at `Grid`'s own implicit zero -- correct
///    exactly because a block boundary means *every* cross entry
///    verifiably normalized to zero (see `metric_block_structure`'s
///    own doc comment).
/// 3. **General** (block structure collapses to one block spanning every
///    coordinate): [`invert_symbolic_block`] runs its size->=3 arm on the
///    whole matrix -- the same code as tier 2's larger blocks, just with
///    one block instead of several.
///
/// In debug builds only (`debug_assert!`, compiled out of release
/// entirely -- never the cost of the check in production), every result
/// is verified against `g_ab g^bc = delta^a_c` via
/// [`verify_metric_inverse`] before being returned: the test suite's own
/// explicit `g*ginv=identity` coverage (diagonal/block/general/synthetic
/// hand-built) is the same check, run directly rather than incidentally
/// through this debug-only path.
pub fn metric_inverse_checkpointed(registry: &Registry, chart: &Chart, g: &ComponentTensor, checkpoint: Checkpoint) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let blocks = metric_block_structure(registry, chart, g)?;
    if blocks.iter().all(|b| b.len() == 1) {
        return metric_inverse_diagonal(registry, chart, g);
    }

    let mat = normalized_matrix(registry, chart, g)?;
    let mut inv = Grid::new(n, 2);
    for block in &blocks {
        if checkpoint() {
            return Err(ComponentError::Cancelled);
        }
        let sub: Vec<Vec<Expr>> = block.iter().map(|&i| block.iter().map(|&j| mat[i as usize][j as usize].clone()).collect()).collect();
        let inv_sub = invert_symbolic_block(block, &sub, checkpoint)?;
        for (bi, &i) in block.iter().enumerate() {
            for (bj, &j) in block.iter().enumerate() {
                inv.set(&[i, j], inv_sub[bi][bj].clone());
            }
        }
    }

    debug_assert!(verify_metric_inverse(registry, chart, g, &inv).is_ok(), "metric_inverse_checkpointed: g_ab * g^bc != delta^a_c -- block detection or block inversion produced a wrong result");
    Ok(inv)
}

/// `g_ab g^bc = delta^a_c`, checked directly rather than assumed:
/// verifies `ginv` really is `g`'s inverse, independent of which of
/// [`metric_inverse_checkpointed`]'s three tiers produced it. This is
/// the concrete test [`ComponentError::InverseVerificationFailed`]'s own
/// doc comment describes -- a block-detection bug (treating a coupled
/// pair as decoupled) is exactly the kind of error that produces a
/// specific, wrong, non-symmetric-looking failure here rather than a
/// crash, which is why this check exists as a standalone, independently
/// callable function and not just inline arithmetic: both the debug-only
/// runtime check in [`metric_inverse_checkpointed`] and this crate's own
/// test suite call the identical code.
pub fn verify_metric_inverse(registry: &Registry, chart: &Chart, g: &ComponentTensor, ginv: &Grid) -> Result<(), ComponentError> {
    let n = chart.dim();
    for a in 0..n as u8 {
        for c in 0..n as u8 {
            let mut sum = Expr::zero();
            for b in 0..n as u8 {
                let g_ab = g.get(registry, &[a, b])?;
                if g_ab.is_zero() {
                    continue;
                }
                let ginv_bc = ginv.get(&[b, c]);
                if ginv_bc.is_zero() {
                    continue;
                }
                sum = sum + g_ab * ginv_bc;
            }
            let sum = normalize(&sum);
            let expected = if a == c { Expr::one() } else { Expr::zero() };
            if sum != expected {
                return Err(ComponentError::InverseVerificationFailed { a, c });
            }
        }
    }
    Ok(())
}

/// `Gamma^a_bc`.
pub fn christoffel(
    registry: &Registry,
    chart: &Chart,
    g: &ComponentTensor,
    ginv: &Grid,
) -> Result<Grid, ComponentError> {
    // `&mut || false`: never cancels, so the only way `christoffel_checkpointed`
    // returns `Err` here is a genuine `ComponentError` from `g.get` --
    // exactly what this signature already propagates.
    christoffel_checkpointed(registry, chart, g, ginv, &mut || false)
}

/// Same as [`christoffel`], but calls `checkpoint` once per independent
/// `(a,b,c)` component, returning `Err(ComponentError::Cancelled)`
/// immediately if it reports true -- measured worst-case gap between
/// stage-only and per-component cancellation on Reissner-Nordstrom:
/// ~500ms -> ~40ms (`oderom-components/tests/diagnostic_cancel_latency.rs`).
pub fn christoffel_checkpointed(
    registry: &Registry,
    chart: &Chart,
    g: &ComponentTensor,
    ginv: &Grid,
    checkpoint: Checkpoint,
) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let mut gamma = Grid::new(n, 3);
    for a in 0..n as u8 {
        for b in 0..n as u8 {
            for c in 0..n as u8 {
                if checkpoint() {
                    return Err(ComponentError::Cancelled);
                }
                let mut sum = Expr::zero();
                for d in 0..n as u8 {
                    let ad = ginv.get(&[a, d]);
                    if ad.is_zero() {
                        continue;
                    }
                    let term = diff(&g.get(registry, &[d, c])?, chart.coord(b))
                        + diff(&g.get(registry, &[d, b])?, chart.coord(c))
                        + Expr::int(-1) * diff(&g.get(registry, &[b, c])?, chart.coord(d));
                    sum = sum + ad * term;
                }
                gamma.set(&[a, b, c], normalize(&(Expr::rational(1, 2) * sum)));
            }
        }
    }
    Ok(gamma)
}

/// The localization generator set for `g` (DESIGN-RATIONAL-FORM.md
/// section 8.4, requirement 2): every distinct denominator appearing in
/// `g`'s own declared components, plus the determinant of every
/// [`metric_block_structure`] block of size >= 2 -- never a hardcoded
/// list. For Kerr this produces (a superset of) `{Sigma, Delta}`
/// (measured directly, `oderom-cli/tests/diagnostic_kerr_denominators.rs`);
/// for a different metric it produces whatever that metric's own
/// components and block determinants actually are, including the empty
/// list for an all-diagonal metric with no fractional components at all
/// (e.g. Schwarzschild in the reciprocal `-f`/`1/f` form still has `f`
/// itself as a component denominator, so that case is not empty either,
/// but nothing stops it from being if a future metric genuinely has no
/// denominators of its own).
///
/// [`LocalizationContext::new`] itself re-checks every candidate
/// (square-free, pairwise coprime) before accepting it, so a duplicate
/// or a genuinely non-qualifying candidate here degrades gracefully
/// (logged, folded into the general engine when actually needed) rather
/// than corrupting the invariant everything else in `localized.rs`
/// relies on.
pub fn localization_generators(registry: &Registry, chart: &Chart, g: &ComponentTensor) -> Result<Vec<Expr>, ComponentError> {
    let n = chart.dim();
    let mut generators = Vec::new();
    for i in 0..n as u8 {
        for j in i..n as u8 {
            let component = g.get(registry, &[i, j])?;
            if component.is_zero() {
                continue;
            }
            let (_, den) = rationalize(&component);
            if den != Expr::one() {
                generators.push(den);
            }
        }
    }
    let blocks = metric_block_structure(registry, chart, g)?;
    let full_matrix = normalized_matrix(registry, chart, g)?;
    for block in &blocks {
        if block.len() < 2 {
            continue;
        }
        let sub: Vec<Vec<Expr>> = block.iter().map(|&i| block.iter().map(|&j| full_matrix[i as usize][j as usize].clone()).collect()).collect();
        let det = determinant_symbolic(&sub, &mut || false)?;
        if !det.is_zero() {
            generators.push(det);
        }
    }
    Ok(generators)
}

/// Names the tensor component being computed when
/// `oderom_expr::LocalizationBudgetExceeded` fires, and joins its
/// generator list into the rendered form
/// `ComponentError::LocalizationFallbackBudgetExceeded` displays --
/// the one place that conversion happens, so every `*_localized_checkpointed`
/// stage below reports the same shape of message.
fn budget_exceeded_error(component: impl Into<String>, e: oderom_expr::LocalizationBudgetExceeded) -> ComponentError {
    ComponentError::LocalizationFallbackBudgetExceeded {
        component: component.into(),
        denominator: e.denominator,
        generators_rendered: e.generators.iter().map(|g| g.to_string()).collect::<Vec<_>>().join(", "),
    }
}

/// Same as [`christoffel`], but reduces every component against `ctx`'s
/// localization generators (DESIGN-RATIONAL-FORM.md section 8) instead
/// of relying solely on the general recursive multivariate GCD
/// `normalize()` calls into -- the fix for a denominator with no single
/// pole variable (Kerr's `Sigma`, section 7.1). `ctx` should be seeded
/// via [`localization_generators`] and shared with every later stage of
/// the same metric's computation (`riemann_mixed_localized`,
/// `ricci_tensor_localized`) so a generator discovered in one stage is
/// recognized in the next.
pub fn christoffel_localized(registry: &Registry, chart: &Chart, g: &ComponentTensor, ginv: &Grid, ctx: &mut LocalizationContext) -> Result<Grid, ComponentError> {
    christoffel_localized_checkpointed(registry, chart, g, ginv, ctx, &mut || false)
}

/// Same as [`christoffel_localized`], but calls `checkpoint` once per
/// independent `(a,b,c)` component (coarse -- matching
/// [`christoffel_checkpointed`]'s own granularity, DESIGN-RATIONAL-FORM.md
/// section 8's "checkpoint per component in the outer loop is enough,
/// don't instrument Poly's own arithmetic") *and* once more, mandatorily,
/// at the one point a component's computation can leave the localized
/// representation for the general engine
/// (`oderom_expr::localized`'s single, unified fallback boundary) --
/// that second check is where the historically non-terminating case
/// actually lives (section 7.1), so it is never skipped regardless of
/// how coarse the outer loop's own checking is.
pub fn christoffel_localized_checkpointed(
    registry: &Registry,
    chart: &Chart,
    g: &ComponentTensor,
    ginv: &Grid,
    ctx: &mut LocalizationContext,
    checkpoint: Checkpoint,
) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let mut gamma = Grid::new(n, 3);
    for a in 0..n as u8 {
        for b in 0..n as u8 {
            for c in 0..n as u8 {
                if checkpoint() {
                    return Err(ComponentError::Cancelled);
                }
                let mut sum = Expr::zero();
                for d in 0..n as u8 {
                    let ad = ginv.get(&[a, d]);
                    if ad.is_zero() {
                        continue;
                    }
                    let term = diff(&g.get(registry, &[d, c])?, chart.coord(b))
                        + diff(&g.get(registry, &[d, b])?, chart.coord(c))
                        + Expr::int(-1) * diff(&g.get(registry, &[b, c])?, chart.coord(d));
                    sum = sum + ad * term;
                }
                let reduced = normalize_localized(&(Expr::rational(1, 2) * sum), ctx, checkpoint)
                    .map_err(|e| budget_exceeded_error(format!("Gamma^{a}_{{{b}{c}}}"), e))?;
                gamma.set(&[a, b, c], reduced);
            }
        }
    }
    Ok(gamma)
}

/// The geodesic equation, one closed-form equation per coordinate
/// (Marco 6 step 4, round B -- `DESIGN-M6-PREP.md` section 1 built the
/// machinery this reuses unchanged, nothing new added to `Expr`):
///
/// ```text
/// x_a'' + Gamma^a_bc x_b' x_c' = 0
/// ```
///
/// `gamma` is exactly `Gamma^a_bc` -- the MIXED Christoffel, one upper
/// index two lower, whatever `christoffel`/`christoffel_checkpointed`
/// already produced from a metric, or a bare `connection`'s own
/// directly-declared grid. This function never lowers or raises an
/// index and needs no `ginv` at all, which is exactly why `geodesic`
/// (unlike `scalar`/`kretschmann`) works from a connection alone, with
/// no metric in scope.
///
/// Each coordinate `x_a` is represented, along the geodesic, as an
/// [`Expr::Func`] of the affine parameter `param` -- the SAME
/// indeterminate-function machinery `f(r)`/`f'(r)` already use (round
/// A), applied here to a coordinate's own name instead of a
/// user-written one: `chart.coord(a)` differentiated once is the
/// velocity `x_a'(param)`, twice is the acceleration `x_a''(param)`.
/// This is a different *value* than the free coordinate `Expr::Var`
/// `christoffel`'s own Gamma is written in terms of -- the two never
/// collapse into each other (a real `Var("t")` inside a Gamma
/// coefficient stays exactly that, never silently promoted to a
/// function of `param`), which is the whole point: `Gamma^a_bc`, as
/// written here, is evaluated symbolically at a general point (the free
/// coordinates), while `x_b'`/`x_c'` name the derivatives actually
/// being solved for. Rendering which encarnação is on screen (plain
/// coordinate vs. function-of-parameter) is a presentation decision,
/// made downstream by `crate::render::render_geodesic` -- never here.
///
/// Panics only if `param` collides with one of `chart`'s own coordinate
/// names or a free variable already used in `gamma` -- the caller
/// (`oderom-cli::commands::check_affine_parameter_collision`) must
/// reject that with a clean error before this is ever called; this
/// function trusts that check already ran, exactly like
/// `metric_inverse_diagonal`'s callers already have to invert first.
pub fn geodesic_equations(chart: &Chart, gamma: &Grid, param: &str) -> Vec<Expr> {
    // Safe: `&mut || false` never cancels, so `_checkpointed` can only
    // return `Ok` -- this function stays infallible exactly as before.
    geodesic_equations_checkpointed(chart, gamma, param, &mut || false).expect("never cancelled")
}

/// Same as [`geodesic_equations`], but calls `checkpoint` once per
/// `(a,b,c)` triple summed while building each equation -- same
/// granularity `christoffel_checkpointed` already uses, and the same
/// reason: contracting Gamma with two velocities across every `(b,c)`
/// pair, for every coordinate `a`, is a new, potentially expensive
/// computation (DESIGN-M6-PREP.md section 4's rule: every new
/// computation path needs this from its first version, not retrofitted
/// later).
pub fn geodesic_equations_checkpointed(chart: &Chart, gamma: &Grid, param: &str, checkpoint: Checkpoint) -> Result<Vec<Expr>, ComponentError> {
    let n = chart.dim();
    let velocity = |i: u8| Expr::Func { name: chart.coord(i).to_string(), args: vec![Expr::var(param)], order: vec![1] };
    let mut equations = Vec::with_capacity(n);
    for a in 0..n as u8 {
        let acceleration = Expr::Func { name: chart.coord(a).to_string(), args: vec![Expr::var(param)], order: vec![2] };
        let mut sum = acceleration;
        for b in 0..n as u8 {
            for c in 0..n as u8 {
                if checkpoint() {
                    return Err(ComponentError::Cancelled);
                }
                let coeff = gamma.get(&[a, b, c]);
                if coeff.is_zero() {
                    continue;
                }
                sum = sum + coeff * velocity(b) * velocity(c);
            }
        }
        equations.push(normalize(&sum));
    }
    Ok(equations)
}

/// The geodesic equation solved for each coordinate's own second
/// derivative -- `x_a'' = f_a(x, x')` -- the form a numerical
/// integrator consumes directly, as opposed to [`geodesic_equations`]'s
/// canonical `... = 0` form (Marco 6 step 4, round C -- `geodesic`
/// itself is unchanged; this is a separate command reusing its output,
/// never a mode of it). This only isolates algebraically -- divides by
/// the coefficient of `x_a''` and moves everything else to the other
/// side -- never integrates the ODE, never picks an initial condition,
/// never distinguishes null/timelike geodesics: the same equation
/// serves all three, as `geodesic` already documents, and solving for
/// the acceleration changes none of that.
///
/// Each of [`geodesic_equations`]'s own closed-form equations is linear
/// in its own coordinate's second derivative *by construction* (the
/// acceleration term is added exactly once, additively, with a literal
/// coefficient of 1, before any Christoffel-times-velocity product, none
/// of which can ever introduce a second derivative) -- and more than
/// that, affine-linearity in the acceleration is a mathematical fact
/// about what a geodesic equation IS (`x_a'' + Gamma^a_bc x_b' x_c' =
/// 0`), not an implementation choice this engine happens to make: no
/// *correct* geodesic equation, from any metric or connection whatsoever,
/// can ever be non-linear in its own acceleration. [`solve_for_second_derivative`]
/// still verifies this structurally, via [`oderom_expr::isolate_linear`],
/// rather than skip straight to dividing -- but treats a violation as
/// `.expect()` (a crashing precondition failure, documenting an upstream
/// bug), not a `Result` a caller could "handle": there is no legitimate
/// input that reaches it, only a bug in `geodesic_equations`' own
/// construction or a caller passing something that was never a geodesic
/// equation to begin with. The coefficient's *vanishing*, by contrast, IS
/// a genuine property of the metric/chart this engine can encounter for
/// real one day (even though, given `geodesic_equations`' own
/// construction plus `geodesic`/`accel`'s shared mandatory
/// parameter-collision check, it is provably always exactly 1 today) --
/// that check stays a real, catchable [`ComponentError::GeodesicCoefficientVanishes`],
/// naming the offending coordinate rather than ever dividing by zero.
/// Unreachable from any metric/connection through the real command
/// surface today, so it is exercised directly against a hand-built
/// equation in this module's own tests and in
/// `oderom-components/tests/schwarzschild.rs`, not by trying to
/// construct an impossible metric.
pub fn accel_equations(chart: &Chart, gamma: &Grid, param: &str) -> Vec<Expr> {
    accel_equations_checkpointed(chart, gamma, param, &mut || false).expect("never cancelled")
}

/// Same as [`accel_equations`], but calls `checkpoint` once per
/// coordinate, on top of [`geodesic_equations_checkpointed`]'s own
/// per-`(a,b,c)` checkpoint (reused unchanged to build the closed-form
/// equations this then isolates) -- coarser, matching the
/// per-component granularity `ricci_tensor_checkpointed` already uses
/// for a similarly cheap-per-item loop. The actual expensive part of
/// this path is `oderom_expr::normalize()`, called repeatedly inside
/// `isolate_linear` and once more for the final division; `normalize()`
/// already carries its own deep, per-call cancellation check (Marco 6
/// step 4, round B), so this per-coordinate checkpoint is the same
/// coarser, between-item backstop every other `_checkpointed` function
/// already has, not the only line of defense.
pub fn accel_equations_checkpointed(
    chart: &Chart,
    gamma: &Grid,
    param: &str,
    checkpoint: Checkpoint,
) -> Result<Vec<Expr>, ComponentError> {
    let equations = geodesic_equations_checkpointed(chart, gamma, param, checkpoint)?;
    let n = chart.dim();
    let mut solved = Vec::with_capacity(n);
    for a in 0..n as u8 {
        if checkpoint() {
            return Err(ComponentError::Cancelled);
        }
        let coord = chart.coord(a);
        solved.push(solve_for_second_derivative(&equations[a as usize], coord, param)?);
    }
    Ok(solved)
}

/// Isolates `equation` (assumed to be `coeff*coord''(param) + remainder
/// = 0`, e.g. one of [`geodesic_equations`]'s own outputs) for
/// `coord''(param)`, returning the already-`normalize()`-d
/// `-remainder/coeff`. Factored out of [`accel_equations_checkpointed`]
/// specifically so the one real, catchable failure mode -- a coefficient
/// that normalizes to identically zero -- is independently testable
/// against a hand-built `equation`, not only reachable (in practice,
/// never, for the reason [`accel_equations`]'s own doc comment gives)
/// through `geodesic_equations`' real construction.
///
/// # Panics
///
/// If `equation` is not affine-linear in `coord''(param)` --
/// [`oderom_expr::isolate_linear`] returns `None`. This is a documented
/// precondition, not a `Result`: see [`accel_equations`]'s own doc
/// comment for why no legitimate geodesic equation can ever violate it
/// (a genuine mathematical fact about the equation's own shape, not an
/// assumption about this engine's construction of it) -- reaching this
/// panic means either a bug in `geodesic_equations` or a caller handing
/// this something that was never a geodesic equation. Exercised directly
/// in this module's own tests and in `oderom-components/tests/schwarzschild.rs`
/// via `#[should_panic]`, since there is no legitimate input to trigger
/// it any other way.
pub fn solve_for_second_derivative(equation: &Expr, coord: &str, param: &str) -> Result<Expr, ComponentError> {
    let target = Expr::Func { name: coord.to_string(), args: vec![Expr::var(param)], order: vec![2] };
    let (coeff, remainder) = isolate_linear(equation, &target).expect(
        "a geodesic equation is affine-linear in its own acceleration by definition -- \
         this equation isn't, which means a bug in geodesic_equations' own construction \
         or a caller passing something that was never a geodesic equation",
    );
    if coeff.is_zero() {
        return Err(ComponentError::GeodesicCoefficientVanishes { coordinate: coord.to_string() });
    }
    Ok(normalize(&(-remainder / coeff)))
}

/// `R^a_bcd`.
pub fn riemann_mixed(chart: &Chart, gamma: &Grid) -> Grid {
    // Safe: `&mut || false` never cancels, so `_checkpointed` can only
    // return `Ok` -- this function stays infallible exactly as before.
    riemann_mixed_checkpointed(chart, gamma, &mut || false).expect("never cancelled")
}

/// Same as [`riemann_mixed`], but calls `checkpoint` once per
/// independent `(a,b,c,d)` component -- see [`christoffel_checkpointed`]
/// for the measured latency this buys.
pub fn riemann_mixed_checkpointed(chart: &Chart, gamma: &Grid, checkpoint: Checkpoint) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let mut riem = Grid::new(n, 4);
    for a in 0..n as u8 {
        for b in 0..n as u8 {
            for c in 0..n as u8 {
                for d in 0..n as u8 {
                    if checkpoint() {
                        return Err(ComponentError::Cancelled);
                    }
                    let mut val = diff(&gamma.get(&[a, b, d]), chart.coord(c))
                        + Expr::int(-1) * diff(&gamma.get(&[a, b, c]), chart.coord(d));
                    for e in 0..n as u8 {
                        val = val
                            + gamma.get(&[a, c, e]) * gamma.get(&[e, b, d])
                            + Expr::int(-1) * gamma.get(&[a, d, e]) * gamma.get(&[e, b, c]);
                    }
                    riem.set(&[a, b, c, d], normalize(&val));
                }
            }
        }
    }
    Ok(riem)
}

/// Same as [`riemann_mixed`], but reduces via `ctx`'s localization
/// generators (see [`christoffel_localized`]) instead of the general
/// engine. This is the stage section 7.1 measured never terminating
/// within a 180s budget for Kerr under the general engine.
pub fn riemann_mixed_localized(chart: &Chart, gamma: &Grid, ctx: &mut LocalizationContext) -> Result<Grid, ComponentError> {
    riemann_mixed_localized_checkpointed(chart, gamma, ctx, &mut || false)
}

/// Same as [`riemann_mixed_localized`], checkpointed per independent
/// `(a,b,c,d)` component, plus the mandatory check at the fallback
/// boundary -- see [`christoffel_localized_checkpointed`]'s own doc
/// comment for why both exist and neither substitutes for the other.
pub fn riemann_mixed_localized_checkpointed(chart: &Chart, gamma: &Grid, ctx: &mut LocalizationContext, checkpoint: Checkpoint) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let mut riem = Grid::new(n, 4);
    for a in 0..n as u8 {
        for b in 0..n as u8 {
            for c in 0..n as u8 {
                for d in 0..n as u8 {
                    if checkpoint() {
                        return Err(ComponentError::Cancelled);
                    }
                    let mut val = diff(&gamma.get(&[a, b, d]), chart.coord(c)) + Expr::int(-1) * diff(&gamma.get(&[a, b, c]), chart.coord(d));
                    for e in 0..n as u8 {
                        val = val + gamma.get(&[a, c, e]) * gamma.get(&[e, b, d]) + Expr::int(-1) * gamma.get(&[a, d, e]) * gamma.get(&[e, b, c]);
                    }
                    let reduced = normalize_localized(&val, ctx, checkpoint).map_err(|e| budget_exceeded_error(format!("R^{a}_{{{b}{c}{d}}}"), e))?;
                    riem.set(&[a, b, c, d], reduced);
                }
            }
        }
    }
    Ok(riem)
}

/// Same as [`lower_index`], but reduces via `ctx`'s localization
/// generators (see [`christoffel_localized`]).
pub fn lower_index_localized(registry: &Registry, chart: &Chart, grid: &Grid, g: &ComponentTensor, position: usize, ctx: &mut LocalizationContext) -> Result<Grid, ComponentError> {
    lower_index_localized_checkpointed(registry, chart, grid, g, position, ctx, &mut || false)
}

/// Same as [`lower_index_localized`], checkpointed per raw index tuple,
/// plus the mandatory check at the fallback boundary -- see
/// [`christoffel_localized_checkpointed`]'s own doc comment.
pub fn lower_index_localized_checkpointed(
    registry: &Registry,
    chart: &Chart,
    grid: &Grid,
    g: &ComponentTensor,
    position: usize,
    ctx: &mut LocalizationContext,
    checkpoint: Checkpoint,
) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let rank = grid.rank();
    let mut out = Grid::new(n, rank);
    for_each_index_tuple(n, rank, |idx| {
        if checkpoint() {
            return Err(ComponentError::Cancelled);
        }
        let target = idx[position];
        let mut sum = Expr::zero();
        for a in 0..n as u8 {
            let g_ea = g.get(registry, &[target, a])?;
            if g_ea.is_zero() {
                continue;
            }
            let mut src = idx.to_vec();
            src[position] = a;
            sum = sum + g_ea * grid.get(&src);
        }
        let reduced = normalize_localized(&sum, ctx, checkpoint).map_err(|e| budget_exceeded_error(format!("indice {idx:?} (abaixando posicao {position})"), e))?;
        out.set(idx, reduced);
        Ok(())
    })?;
    Ok(out)
}

/// Same as [`lower_first_index`], but reduces via `ctx`'s localization
/// generators (see [`christoffel_localized`]).
pub fn lower_first_index_localized(registry: &Registry, chart: &Chart, grid: &Grid, g: &ComponentTensor, ctx: &mut LocalizationContext) -> Result<Grid, ComponentError> {
    lower_index_localized(registry, chart, grid, g, 0, ctx)
}

/// Same as [`lower_first_index_localized`], checkpointed -- see
/// [`lower_index_localized_checkpointed`].
pub fn lower_first_index_localized_checkpointed(
    registry: &Registry,
    chart: &Chart,
    grid: &Grid,
    g: &ComponentTensor,
    ctx: &mut LocalizationContext,
    checkpoint: Checkpoint,
) -> Result<Grid, ComponentError> {
    lower_index_localized_checkpointed(registry, chart, grid, g, 0, ctx, checkpoint)
}

/// Same as [`raise_index`], but reduces via `ctx`'s localization
/// generators (see [`christoffel_localized`]).
pub fn raise_index_localized(chart: &Chart, grid: &Grid, ginv: &Grid, position: usize, ctx: &mut LocalizationContext) -> Result<Grid, ComponentError> {
    raise_index_localized_checkpointed(chart, grid, ginv, position, ctx, &mut || false)
}

/// Same as [`raise_index_localized`], checkpointed per raw index tuple,
/// plus the mandatory check at the fallback boundary -- see
/// [`christoffel_localized_checkpointed`]'s own doc comment.
pub fn raise_index_localized_checkpointed(
    chart: &Chart,
    grid: &Grid,
    ginv: &Grid,
    position: usize,
    ctx: &mut LocalizationContext,
    checkpoint: Checkpoint,
) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let rank = grid.rank();
    let mut out = Grid::new(n, rank);
    for_each_index_tuple(n, rank, |idx| {
        if checkpoint() {
            return Err(ComponentError::Cancelled);
        }
        let target = idx[position];
        let mut sum = Expr::zero();
        for a in 0..n as u8 {
            let coeff = ginv.get(&[target, a]);
            if coeff.is_zero() {
                continue;
            }
            let mut src = idx.to_vec();
            src[position] = a;
            sum = sum + coeff * grid.get(&src);
        }
        let reduced = normalize_localized(&sum, ctx, checkpoint).map_err(|e| budget_exceeded_error(format!("indice {idx:?} (subindo posicao {position})"), e))?;
        out.set(idx, reduced);
        Ok(())
    })?;
    Ok(out)
}

/// Same as [`kretschmann`], but reduces via `ctx`'s localization
/// generators (see [`christoffel_localized`]) -- the fix for
/// DESIGN-RATIONAL-FORM.md section 7.1.
pub fn kretschmann_localized(chart: &Chart, riemann_cov: &Grid, ginv: &Grid, ctx: &mut LocalizationContext) -> Result<Expr, ComponentError> {
    kretschmann_localized_checkpointed(chart, riemann_cov, ginv, ctx, &mut || false)
}

/// Same as [`kretschmann_localized`], checkpointed through the four
/// [`raise_index_localized_checkpointed`] calls and once per summed
/// term, plus the mandatory check at the final `normalize_localized`'s
/// own fallback boundary -- see [`christoffel_localized_checkpointed`]'s
/// own doc comment.
pub fn kretschmann_localized_checkpointed(chart: &Chart, riemann_cov: &Grid, ginv: &Grid, ctx: &mut LocalizationContext, checkpoint: Checkpoint) -> Result<Expr, ComponentError> {
    let raised0 = raise_index_localized_checkpointed(chart, riemann_cov, ginv, 0, ctx, checkpoint)?;
    let raised1 = raise_index_localized_checkpointed(chart, &raised0, ginv, 1, ctx, checkpoint)?;
    let raised2 = raise_index_localized_checkpointed(chart, &raised1, ginv, 2, ctx, checkpoint)?;
    let riemann_contra = raise_index_localized_checkpointed(chart, &raised2, ginv, 3, ctx, checkpoint)?;

    let n = chart.dim();
    let mut sum = Expr::zero();
    for_each_index_tuple(n, 4, |idx| {
        if checkpoint() {
            return Err(ComponentError::Cancelled);
        }
        let term = riemann_cov.get(idx) * riemann_contra.get(idx);
        sum = std::mem::replace(&mut sum, Expr::zero()) + term;
        Ok(())
    })?;
    normalize_localized(&sum, ctx, checkpoint).map_err(|e| budget_exceeded_error("Kretschmann", e))
}

/// Lowers a `Grid`'s first index through `g`: `T_{e...} = g_{ea} T^a_{...}`.
/// A thin wrapper around [`lower_index`] at `position` 0 -- every
/// existing caller of this function keeps its exact current behavior
/// (same code path, same result), see [`lower_index`]'s own doc comment
/// for the general form this delegates to.
pub fn lower_first_index(
    registry: &Registry,
    chart: &Chart,
    grid: &Grid,
    g: &ComponentTensor,
) -> Result<Grid, ComponentError> {
    lower_index(registry, chart, grid, g, 0)
}

/// Same as [`lower_first_index`], but calls `checkpoint` once per raw
/// index tuple -- see [`christoffel_checkpointed`] for the measured
/// latency this buys.
pub fn lower_first_index_checkpointed(
    registry: &Registry,
    chart: &Chart,
    grid: &Grid,
    g: &ComponentTensor,
    checkpoint: Checkpoint,
) -> Result<Grid, ComponentError> {
    lower_index_checkpointed(registry, chart, grid, g, 0, checkpoint)
}

/// Lowers one index of a `Grid` (0-based `position`) through `g`:
/// `T_{..e..} = g_{ea} T^{..a..}` -- the general form
/// [`lower_first_index`] (always `position` 0) delegates to, and the
/// covariant-lowering counterpart to [`raise_index`]'s own `position`
/// parameter (Rodada Variancia: [`change_variance`] needs to lower an
/// arbitrary slot, not only the first, the same way it can already raise
/// an arbitrary one).
pub fn lower_index(registry: &Registry, chart: &Chart, grid: &Grid, g: &ComponentTensor, position: usize) -> Result<Grid, ComponentError> {
    lower_index_checkpointed(registry, chart, grid, g, position, &mut || false)
}

/// Same as [`lower_index`], but calls `checkpoint` once per raw index
/// tuple -- see [`christoffel_checkpointed`] for the measured latency
/// this buys.
pub fn lower_index_checkpointed(
    registry: &Registry,
    chart: &Chart,
    grid: &Grid,
    g: &ComponentTensor,
    position: usize,
    checkpoint: Checkpoint,
) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let rank = grid.rank();
    let mut out = Grid::new(n, rank);
    for_each_index_tuple(n, rank, |idx| {
        if checkpoint() {
            return Err(ComponentError::Cancelled);
        }
        let target = idx[position];
        let mut sum = Expr::zero();
        for a in 0..n as u8 {
            let g_ea = g.get(registry, &[target, a])?;
            if g_ea.is_zero() {
                continue;
            }
            let mut src = idx.to_vec();
            src[position] = a;
            sum = sum + g_ea * grid.get(&src);
        }
        out.set(idx, normalize(&sum));
        Ok(())
    })?;
    Ok(out)
}

/// Raises one index of a `Grid` (0-based `position`) through `ginv`:
/// `T^{..a..} = g^{ab} T_{..b..}`.
pub fn raise_index(chart: &Chart, grid: &Grid, ginv: &Grid, position: usize) -> Grid {
    // Safe: `&mut || false` never cancels, so `_checkpointed` can only
    // return `Ok` -- this function stays infallible exactly as before.
    raise_index_checkpointed(chart, grid, ginv, position, &mut || false).expect("never cancelled")
}

/// Same as [`raise_index`], but calls `checkpoint` once per raw index
/// tuple -- see [`christoffel_checkpointed`] for the measured latency
/// this buys.
pub fn raise_index_checkpointed(
    chart: &Chart,
    grid: &Grid,
    ginv: &Grid,
    position: usize,
    checkpoint: Checkpoint,
) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let rank = grid.rank();
    let mut out = Grid::new(n, rank);
    for_each_index_tuple(n, rank, |idx| {
        if checkpoint() {
            return Err(ComponentError::Cancelled);
        }
        let target = idx[position];
        let mut sum = Expr::zero();
        for a in 0..n as u8 {
            let coeff = ginv.get(&[target, a]);
            if coeff.is_zero() {
                continue;
            }
            let mut src = idx.to_vec();
            src[position] = a;
            sum = sum + coeff * grid.get(&src);
        }
        out.set(idx, normalize(&sum));
        Ok(())
    })?;
    Ok(out)
}

/// Changes `grid`'s per-slot variance from `current` to `desired`, one
/// slot at a time, via [`raise_index`]/[`lower_index`] -- the shared
/// primitive behind index-variance queries (`riemann [up,down,down,down]`,
/// `oderom-cli::parser::parse_query`'s bracket notation, Rodada
/// Variancia). A slot already at its desired variance is left untouched
/// (no `raise`/`lower` call for it at all, not even a no-op one) --
/// `current[i] == desired[i]` for every `i` returns `grid` itself
/// (cloned, never mutated in place: every other function in this module
/// returns a fresh `Grid`, and this one does too, for the same reason).
///
/// Only ever meaningful for genuine tensors: `g`/`ginv` are the metric's
/// own covariant/contravariant components, and raising/lowering with
/// them is what makes the operation produce another tensor, in the same
/// sense the original `grid` was one. Christoffel is explicitly NOT a
/// tensor (see `oderom-cli::error::CliError::ChristoffelNotATensor`'s
/// own doc comment) -- callers must refuse a variance change on it
/// before ever reaching this function, not rely on this function to
/// refuse for them; there is nothing about a bare `Grid`/`current`/
/// `desired` here that could detect "this happens to hold Christoffel
/// symbols" to refuse on its own.
///
/// # Panics
///
/// If `current.len() != desired.len()` or either differs from `grid`'s
/// own `rank()` -- a caller bug (every real call site validates arity
/// against the tensor's actual rank first, e.g.
/// `oderom-session::run::run_command_query`'s own check against
/// `CommandName`'s known rank, so a legitimate query can never reach
/// this), not a condition a legitimate query can trigger.
pub fn change_variance(
    registry: &Registry,
    chart: &Chart,
    grid: &Grid,
    current: &[Variance],
    desired: &[Variance],
    g: &ComponentTensor,
    ginv: &Grid,
) -> Result<Grid, ComponentError> {
    change_variance_checkpointed(registry, chart, grid, current, desired, g, ginv, &mut || false)
}

/// Same as [`change_variance`], but threads `checkpoint` through to
/// every [`raise_index_checkpointed`]/[`lower_index_checkpointed`] call
/// it makes (each of which already checkpoints once per raw index
/// tuple, `christoffel_checkpointed`'s own doc comment has the measured
/// latency that buys) -- no separate checkpoint of its own around the
/// outer per-slot loop, which never exceeds `rank` (at most 4 in every
/// fixture this project has) and does no work of its own beyond
/// dispatching to those two already-checkpointed primitives.
pub fn change_variance_checkpointed(
    registry: &Registry,
    chart: &Chart,
    grid: &Grid,
    current: &[Variance],
    desired: &[Variance],
    g: &ComponentTensor,
    ginv: &Grid,
    checkpoint: Checkpoint,
) -> Result<Grid, ComponentError> {
    assert_eq!(current.len(), desired.len(), "change_variance: current and desired must have the same length");
    assert_eq!(current.len(), grid.rank(), "change_variance: current/desired must match the grid's own rank");
    let mut result = grid.clone();
    for (position, (&from, &to)) in current.iter().zip(desired.iter()).enumerate() {
        result = match (from, to) {
            (Variance::Co, Variance::Contra) => raise_index_checkpointed(chart, &result, ginv, position, checkpoint)?,
            (Variance::Contra, Variance::Co) => lower_index_checkpointed(registry, chart, &result, g, position, checkpoint)?,
            (Variance::Co, Variance::Co) | (Variance::Contra, Variance::Contra) => result,
        };
    }
    Ok(result)
}

/// `R_bd = R^a_bad` (Ricci tensor, from the mixed Riemann tensor).
pub fn ricci_tensor(chart: &Chart, riemann_mixed: &Grid) -> Grid {
    // Safe: `&mut || false` never cancels, so `_checkpointed` can only
    // return `Ok` -- this function stays infallible exactly as before.
    ricci_tensor_checkpointed(chart, riemann_mixed, &mut || false).expect("never cancelled")
}

/// Same as [`ricci_tensor`], but calls `checkpoint` once per independent
/// `(b,d)` component -- see [`christoffel_checkpointed`] for the
/// measured latency this buys (this stage is rank 2, so it is already
/// cheap -- `n^2` components, not `n^4` -- but the hook is here for
/// consistency with every other stage function).
pub fn ricci_tensor_checkpointed(chart: &Chart, riemann_mixed: &Grid, checkpoint: Checkpoint) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let mut ricci = Grid::new(n, 2);
    for b in 0..n as u8 {
        for d in 0..n as u8 {
            if checkpoint() {
                return Err(ComponentError::Cancelled);
            }
            let mut sum = Expr::zero();
            for a in 0..n as u8 {
                sum = sum + riemann_mixed.get(&[a, b, a, d]);
            }
            ricci.set(&[b, d], normalize(&sum));
        }
    }
    Ok(ricci)
}

/// Same as [`ricci_tensor`], but reduces via `ctx`'s localization
/// generators (see [`christoffel_localized`]).
pub fn ricci_tensor_localized(chart: &Chart, riemann_mixed: &Grid, ctx: &mut LocalizationContext) -> Result<Grid, ComponentError> {
    ricci_tensor_localized_checkpointed(chart, riemann_mixed, ctx, &mut || false)
}

/// Same as [`ricci_tensor_localized`], checkpointed per independent
/// `(b,d)` component, plus the mandatory check at the fallback boundary
/// -- see [`christoffel_localized_checkpointed`]'s own doc comment.
pub fn ricci_tensor_localized_checkpointed(chart: &Chart, riemann_mixed: &Grid, ctx: &mut LocalizationContext, checkpoint: Checkpoint) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let mut ricci = Grid::new(n, 2);
    for b in 0..n as u8 {
        for d in 0..n as u8 {
            if checkpoint() {
                return Err(ComponentError::Cancelled);
            }
            let mut sum = Expr::zero();
            for a in 0..n as u8 {
                sum = sum + riemann_mixed.get(&[a, b, a, d]);
            }
            let reduced = normalize_localized(&sum, ctx, checkpoint).map_err(|e| budget_exceeded_error(format!("R_{{{b}{d}}}"), e))?;
            ricci.set(&[b, d], reduced);
        }
    }
    Ok(ricci)
}

/// `R = g^bd R_bd` (Ricci scalar).
pub fn ricci_scalar(chart: &Chart, ricci: &Grid, ginv: &Grid) -> Expr {
    let n = chart.dim();
    let mut sum = Expr::zero();
    for b in 0..n as u8 {
        for d in 0..n as u8 {
            let coeff = ginv.get(&[b, d]);
            if coeff.is_zero() {
                continue;
            }
            sum = sum + coeff * ricci.get(&[b, d]);
        }
    }
    normalize(&sum)
}

/// `R_ab R^ab` (the Ricci-squared scalar invariant, Marco 6 step 6) --
/// the Ricci tensor contracted with itself after raising both indices.
pub fn ricci_squared(chart: &Chart, ricci: &Grid, ginv: &Grid) -> Expr {
    // Safe: `&mut || false` never cancels, so `_checkpointed` can only
    // return `Ok` -- this function stays infallible exactly as before.
    ricci_squared_checkpointed(chart, ricci, ginv, &mut || false).expect("never cancelled")
}

/// Same as [`ricci_squared`], but calls `checkpoint` once per raised-index
/// component (through the two [`raise_index_checkpointed`] calls) and
/// once per summed term -- same density [`kretschmann_checkpointed`]/
/// [`weyl_squared_checkpointed`] already use.
///
/// Used to be a diagonal-only shortcut (`sum_{a,b} g^aa g^bb (R_ab)^2`),
/// valid only because every `ginv` this crate could produce was
/// diagonal before this round (D-M2.1). A non-diagonal `ginv` (Kerr,
/// Godel) makes that shortcut silently wrong -- it drops every
/// off-diagonal `g^ac`/`g^bd` cross term the real contraction
/// `sum_{a,b,c,d} g^ac g^bd R_ab R_cd` has -- so it is replaced here with
/// the same raise-then-contract shape [`kretschmann_checkpointed`]/
/// [`weyl_squared_checkpointed`] already use, general in `ginv`'s own
/// structure and correct for any of [`metric_inverse_checkpointed`]'s
/// three tiers.
pub fn ricci_squared_checkpointed(chart: &Chart, ricci: &Grid, ginv: &Grid, checkpoint: Checkpoint) -> Result<Expr, ComponentError> {
    let raised0 = raise_index_checkpointed(chart, ricci, ginv, 0, checkpoint)?;
    let ricci_contra = raise_index_checkpointed(chart, &raised0, ginv, 1, checkpoint)?;

    let n = chart.dim();
    let mut sum = Expr::zero();
    for_each_index_tuple(n, 2, |idx| {
        if checkpoint() {
            return Err(ComponentError::Cancelled);
        }
        let term = ricci.get(idx) * ricci_contra.get(idx);
        sum = std::mem::replace(&mut sum, Expr::zero()) + term;
        Ok(())
    })?;
    Ok(normalize(&sum))
}

/// `R_abcd R^abcd - 4 R_ab R^ab + R^2` (the Gauss-Bonnet/Euler density,
/// Marco 6 step 6) -- pure arithmetic on three scalars this crate
/// already produces (`kretschmann`, [`ricci_squared`], `ricci_scalar`),
/// never a fourth curvature computation. In exactly 4 dimensions this is
/// the integrand whose integral is (proportional to) the Euler
/// characteristic; the name/formula make sense in any dimension, this
/// function does not restrict to `n=4`.
pub fn gauss_bonnet(kretschmann: &Expr, ricci_squared: &Expr, ricci_scalar: &Expr) -> Expr {
    normalize(&(kretschmann.clone() - Expr::int(4) * ricci_squared.clone() + ricci_scalar.clone().pow(2)))
}

/// `G_ab = R_ab - (1/2) g_ab R` (the Einstein tensor, fully covariant) --
/// literal arithmetic over three quantities this crate already computes
/// (`ricci_tensor`/`ricci_scalar`, and the metric's own covariant
/// components), never a new curvature computation of its own. Covariant
/// only, matching `ricci`'s own variance -- raising an index is a
/// different, not-yet-built capability (see `christoffel`/`riemann`'s
/// own `_mixed` vs `_cov` split for what that would require), out of
/// scope here.
///
/// Needs a metric explicitly, unlike `christoffel`/`riemann`/`ricci`:
/// `g_ab` appears literally in the formula, and `R` is `g^bd R_bd` --
/// both require a real metric, never just a bare connection's Gamma.
/// Callers refuse a connection-only source before this is ever called,
/// the same way `scalar`/`kretschmann` already do (`require_metric`,
/// `oderom-cli::commands`).
pub fn einstein_tensor(registry: &Registry, chart: &Chart, metric: &ComponentTensor, ricci: &Grid, ricci_scalar: &Expr) -> Grid {
    // Safe: `&mut || false` never cancels, so `_checkpointed` can only
    // return `Ok` -- this function stays infallible exactly as before.
    einstein_tensor_checkpointed(registry, chart, metric, ricci, ricci_scalar, &mut || false).expect("never cancelled")
}

/// Same as [`einstein_tensor`], but calls `checkpoint` once per
/// independent `(a,b)` component -- same rank-2 granularity
/// `ricci_tensor_checkpointed` already uses. Cheap by construction (the
/// expensive stages -- Riemann, Ricci -- already ran to produce `ricci`/
/// `ricci_scalar`; this is one subtraction per component), but every new
/// computation path gets a checkpoint from its first version regardless
/// (DESIGN-M6-PREP.md section 4).
pub fn einstein_tensor_checkpointed(
    registry: &Registry,
    chart: &Chart,
    metric: &ComponentTensor,
    ricci: &Grid,
    ricci_scalar: &Expr,
    checkpoint: Checkpoint,
) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let mut einstein = Grid::new(n, 2);
    for_each_index_tuple(n, 2, |idx| {
        if checkpoint() {
            return Err(ComponentError::Cancelled);
        }
        let g_ab = metric.get(registry, idx)?;
        let value = ricci.get(idx) - (g_ab * ricci_scalar.clone()) / Expr::int(2);
        einstein.set(idx, normalize(&value));
        Ok(())
    })?;
    Ok(einstein)
}

/// `R_abcd R^abcd` (the Kretschmann scalar), given the fully covariant
/// Riemann tensor and the inverse metric.
pub fn kretschmann(chart: &Chart, riemann_cov: &Grid, ginv: &Grid) -> Expr {
    // Safe: `&mut || false` never cancels, so `_checkpointed` can only
    // return `Ok` -- this function stays infallible exactly as before.
    kretschmann_checkpointed(chart, riemann_cov, ginv, &mut || false).expect("never cancelled")
}

/// Same as [`kretschmann`], but calls `checkpoint` once per raised-index
/// component (the four [`raise_index_checkpointed`] calls -- the
/// expensive part, see [`christoffel_checkpointed`] for the measured
/// latency this buys) and once per summed term. The final `normalize`
/// over the whole accumulated sum is, like every other single
/// `normalize()` call in this crate, still one opaque unit of work with
/// no checkpoint *inside* it -- same documented limit as any other
/// GCD-heavy call (DESIGN-UI-SESSION.md, DESIGN-RATIONAL-FORM.md
/// section 6).
pub fn kretschmann_checkpointed(
    chart: &Chart,
    riemann_cov: &Grid,
    ginv: &Grid,
    checkpoint: Checkpoint,
) -> Result<Expr, ComponentError> {
    let raised0 = raise_index_checkpointed(chart, riemann_cov, ginv, 0, checkpoint)?;
    let raised1 = raise_index_checkpointed(chart, &raised0, ginv, 1, checkpoint)?;
    let raised2 = raise_index_checkpointed(chart, &raised1, ginv, 2, checkpoint)?;
    let riemann_contra = raise_index_checkpointed(chart, &raised2, ginv, 3, checkpoint)?;

    let n = chart.dim();
    let mut sum = Expr::zero();
    for_each_index_tuple(n, 4, |idx| {
        if checkpoint() {
            return Err(ComponentError::Cancelled);
        }
        let term = riemann_cov.get(idx) * riemann_contra.get(idx);
        sum = std::mem::replace(&mut sum, Expr::zero()) + term;
        Ok(())
    })?;
    Ok(normalize(&sum))
}

/// `C_abcd`, the Weyl (conformal) tensor, fully covariant (Marco 6 step
/// 6):
///
/// ```text
/// C_abcd = R_abcd - 1/(n-2) (g_ac R_db - g_ad R_cb - g_bc R_da + g_bd R_ca)
///          + R/((n-1)(n-2)) (g_ac g_db - g_ad g_cb)
/// ```
///
/// the standard formula's antisymmetrization brackets (`g_a[c R_d]b`,
/// `g_a[c g_d]b`) expanded out by hand (`X_[cd] = (1/2)(X_cd - X_dc)`)
/// -- Riemann minus the trace parts `Ricci`/the Ricci scalar carry,
/// leaving only the "purely conformal" part. Covariant only, matching
/// [`einstein_tensor`]'s own choice not to raise an index for display;
/// same reasoning.
///
/// # Panics -- no: refuses instead
///
/// Undefined at `n=2` (the `1/(n-2)` term itself blows up there, not a
/// computation this engine merely fails at) -- returns
/// [`ComponentError::WeylUndefinedInDimension2`] instead of ever
/// dividing by zero. At `n=3` the formula is well-defined and (by a real
/// theorem, not a special case coded here) evaluates identically to
/// zero on its own; nothing here forces that, `oderom-components/tests/`
/// confirms it comes out zero for a genuine, non-maximally-symmetric 3D
/// metric rather than assuming the theorem holds.
pub fn weyl_tensor(registry: &Registry, chart: &Chart, metric: &ComponentTensor, riemann_cov: &Grid, ricci_cov: &Grid, ricci_scalar: &Expr) -> Result<Grid, ComponentError> {
    // Safe: `&mut || false` never cancels, so `_checkpointed` can only
    // return `Ok(Ok(_))`/`Ok(Err(WeylUndefinedInDimension2))` -- the
    // `Cancelled` arm is genuinely unreachable, but the dimension
    // refusal is a real, non-cancellation error this still has to
    // surface, so this stays `Result`-returning rather than `.expect()`.
    weyl_tensor_checkpointed(registry, chart, metric, riemann_cov, ricci_cov, ricci_scalar, &mut || false)
}

/// Same as [`weyl_tensor`], but calls `checkpoint` once per independent
/// `(a,b,c,d)` component -- the same rank-4 granularity
/// `riemann_mixed_checkpointed` already uses (`christoffel_checkpointed`'s
/// own doc comment has the measured latency this buys).
pub fn weyl_tensor_checkpointed(
    registry: &Registry,
    chart: &Chart,
    metric: &ComponentTensor,
    riemann_cov: &Grid,
    ricci_cov: &Grid,
    ricci_scalar: &Expr,
    checkpoint: Checkpoint,
) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    if n == 2 {
        return Err(ComponentError::WeylUndefinedInDimension2);
    }
    let n_expr = Expr::int(n as i64);
    let denom1 = n_expr.clone() - Expr::int(2);
    let denom2 = (n_expr.clone() - Expr::int(1)) * denom1.clone();
    let mut weyl = Grid::new(n, 4);
    for_each_index_tuple(n, 4, |idx| {
        if checkpoint() {
            return Err(ComponentError::Cancelled);
        }
        let (a, b, c, d) = (idx[0], idx[1], idx[2], idx[3]);
        let g_ac = metric.get(registry, &[a, c])?;
        let g_ad = metric.get(registry, &[a, d])?;
        let g_bc = metric.get(registry, &[b, c])?;
        let g_bd = metric.get(registry, &[b, d])?;
        let r_db = ricci_cov.get(&[d, b]);
        let r_cb = ricci_cov.get(&[c, b]);
        let r_da = ricci_cov.get(&[d, a]);
        let r_ca = ricci_cov.get(&[c, a]);

        let correction = g_ac.clone() * r_db - g_ad.clone() * r_cb - g_bc.clone() * r_da + g_bd.clone() * r_ca;
        let trace_term = g_ac * g_bd - g_ad * g_bc;

        let value = riemann_cov.get(idx) - correction / denom1.clone() + ricci_scalar.clone() * trace_term / denom2.clone();
        weyl.set(idx, normalize(&value));
        Ok(())
    })?;
    Ok(weyl)
}

/// `C_abcd C^abcd` (the Weyl invariant, Marco 6 step 6) -- same
/// four-index raise-then-contract shape [`kretschmann`] already uses,
/// with the Weyl tensor in place of Riemann. Inherits [`weyl_tensor`]'s
/// dimension-2 refusal automatically (computes the Weyl tensor first;
/// if that refuses, this does too, via `?`) -- never duplicates the
/// check.
pub fn weyl_squared(registry: &Registry, chart: &Chart, metric: &ComponentTensor, riemann_cov: &Grid, ricci_cov: &Grid, ricci_scalar: &Expr, ginv: &Grid) -> Result<Expr, ComponentError> {
    weyl_squared_checkpointed(registry, chart, metric, riemann_cov, ricci_cov, ricci_scalar, ginv, &mut || false)
}

/// Same as [`weyl_squared`], but calls `checkpoint` once per raised-index
/// component (through [`weyl_tensor_checkpointed`] and the four
/// [`raise_index_checkpointed`] calls) and once per summed term -- same
/// density [`kretschmann_checkpointed`] already uses.
#[allow(clippy::too_many_arguments)]
pub fn weyl_squared_checkpointed(
    registry: &Registry,
    chart: &Chart,
    metric: &ComponentTensor,
    riemann_cov: &Grid,
    ricci_cov: &Grid,
    ricci_scalar: &Expr,
    ginv: &Grid,
    checkpoint: Checkpoint,
) -> Result<Expr, ComponentError> {
    let weyl_cov = weyl_tensor_checkpointed(registry, chart, metric, riemann_cov, ricci_cov, ricci_scalar, checkpoint)?;
    let raised0 = raise_index_checkpointed(chart, &weyl_cov, ginv, 0, checkpoint)?;
    let raised1 = raise_index_checkpointed(chart, &raised0, ginv, 1, checkpoint)?;
    let raised2 = raise_index_checkpointed(chart, &raised1, ginv, 2, checkpoint)?;
    let weyl_contra = raise_index_checkpointed(chart, &raised2, ginv, 3, checkpoint)?;

    let n = chart.dim();
    let mut sum = Expr::zero();
    for_each_index_tuple(n, 4, |idx| {
        if checkpoint() {
            return Err(ComponentError::Cancelled);
        }
        let term = weyl_cov.get(idx) * weyl_contra.get(idx);
        sum = std::mem::replace(&mut sum, Expr::zero()) + term;
        Ok(())
    })?;
    Ok(normalize(&sum))
}

/// Converts a raw `Grid` into a [`ComponentTensor`] stored under `head`'s
/// declared symmetry (one entry per orbit, not per raw index tuple).
/// `grid`'s rank must equal `head`'s arity -- true of every call site in
/// this crate by construction, so an arity mismatch here is an internal
/// bug, not a caller error.
pub fn grid_to_component_tensor(registry: &Registry, head: HeadId, grid: &Grid) -> ComponentTensor {
    let mut ct = ComponentTensor::new(head);
    for_each_index_tuple(grid.dim(), grid.rank(), |idx| {
        ct.set(registry, idx, grid.get(idx)).expect("grid rank matches head arity by construction");
        Ok(())
    })
    .expect("closure above never returns Err");
    ct
}

#[cfg(test)]
mod tests {
    use super::*;
    use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};

    /// A minimal Schwarzschild-shaped setup -- not the full acceptance
    /// fixture, just enough `Chart`/`ComponentTensor`/`Grid` to exercise
    /// `christoffel_checkpointed`/`riemann_mixed_checkpointed` directly.
    fn schwarzschild() -> (Registry, Chart, ComponentTensor, Grid, Grid) {
        let mut registry = Registry::new();
        let manifold = registry.declare_manifold("M", 4).unwrap();
        let tm = registry.declare_bundle("TM", manifold, 4).unwrap();
        let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };
        let metric_slots: smallvec::SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
        let head = registry.declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();

        let chart = Chart::new(["t", "r", "theta", "phi"]);
        let m = Expr::var("M");
        let r = Expr::var("r");
        let theta = Expr::var("theta");
        let f = Expr::one() - Expr::int(2) * m * r.clone().pow(-1);

        let mut g = ComponentTensor::new(head);
        g.set(&registry, &[0, 0], normalize(&(Expr::int(-1) * f.clone()))).unwrap();
        g.set(&registry, &[1, 1], normalize(&f.pow(-1))).unwrap();
        g.set(&registry, &[2, 2], normalize(&r.clone().pow(2))).unwrap();
        g.set(&registry, &[3, 3], normalize(&(r.pow(2) * theta.sin().pow(2)))).unwrap();

        let ginv = metric_inverse_diagonal(&registry, &chart, &g).unwrap();
        let gamma = christoffel(&registry, &chart, &g, &ginv).unwrap();
        (registry, chart, g, ginv, gamma)
    }

    /// Proves the checkpoint actually stops the loop early -- not just
    /// that it compiles and returns `Cancelled` after doing the full
    /// amount of work anyway. Fires after the 5th call (well inside
    /// christoffel's 64 `(a,b,c)` components for dim 4) and confirms the
    /// call count observed by the checkpoint itself never exceeds 5,
    /// i.e. `christoffel_checkpointed` returned as soon as it was told
    /// to, not after finishing the loop and discarding the result.
    #[test]
    fn christoffel_checkpointed_stops_the_loop_immediately() {
        let (registry, chart, g, ginv, _) = schwarzschild();
        let mut calls = 0u32;
        let result = christoffel_checkpointed(&registry, &chart, &g, &ginv, &mut || {
            calls += 1;
            calls >= 5
        });
        assert!(matches!(result, Err(ComponentError::Cancelled)));
        assert_eq!(calls, 5, "checkpoint should have been called exactly once more after reporting cancelled, never after");
    }

    #[test]
    fn riemann_mixed_checkpointed_stops_the_loop_immediately() {
        let (_, chart, _, _, gamma) = schwarzschild();
        let mut calls = 0u32;
        let result = riemann_mixed_checkpointed(&chart, &gamma, &mut || {
            calls += 1;
            calls >= 5
        });
        assert!(matches!(result, Err(ComponentError::Cancelled)));
        assert_eq!(calls, 5);
    }

    /// The non-`_checkpointed` wrappers are unaffected: same result as
    /// before this change, every time -- the `.expect("never
    /// cancelled")`/`?` plumbing added alongside the checkpoint hook
    /// does not change their behavior for any existing caller.
    #[test]
    fn uncheckpointed_wrappers_still_compute_the_full_result() {
        let (registry, chart, g, ginv, gamma_from_fixture) = schwarzschild();
        let gamma = christoffel(&registry, &chart, &g, &ginv).unwrap();
        assert_eq!(gamma.get(&[1, 2, 2]), gamma_from_fixture.get(&[1, 2, 2]));
        let riem = riemann_mixed(&chart, &gamma);
        assert!(!riem.get(&[0, 1, 0, 1]).is_zero());
    }
}
