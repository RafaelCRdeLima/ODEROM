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
use oderom_core::{HeadId, Registry};
use oderom_expr::{diff, normalize, Expr};

fn for_each_index_tuple(dim: usize, rank: usize, mut f: impl FnMut(&[u8])) {
    fn rec(dim: usize, rank: usize, pos: usize, idx: &mut Vec<u8>, f: &mut impl FnMut(&[u8])) {
        if pos == rank {
            f(idx);
            return;
        }
        for v in 0..dim as u8 {
            idx[pos] = v;
            rec(dim, rank, pos + 1, idx, f);
        }
    }
    let mut idx = vec![0u8; rank];
    rec(dim, rank, 0, &mut idx, &mut f);
}

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

/// `Gamma^a_bc`.
pub fn christoffel(
    registry: &Registry,
    chart: &Chart,
    g: &ComponentTensor,
    ginv: &Grid,
) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let mut gamma = Grid::new(n, 3);
    for a in 0..n as u8 {
        for b in 0..n as u8 {
            for c in 0..n as u8 {
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

/// `R^a_bcd`.
pub fn riemann_mixed(chart: &Chart, gamma: &Grid) -> Grid {
    let n = chart.dim();
    let mut riem = Grid::new(n, 4);
    for a in 0..n as u8 {
        for b in 0..n as u8 {
            for c in 0..n as u8 {
                for d in 0..n as u8 {
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
    riem
}

/// Lowers a `Grid`'s first index through `g`: `T_{e...} = g_{ea} T^a_{...}`.
pub fn lower_first_index(
    registry: &Registry,
    chart: &Chart,
    grid: &Grid,
    g: &ComponentTensor,
) -> Result<Grid, ComponentError> {
    let n = chart.dim();
    let rank = grid.rank();
    let mut out = Grid::new(n, rank);
    let mut err = None;
    for_each_index_tuple(n, rank, |idx| {
        if err.is_some() {
            return;
        }
        let target = idx[0];
        let mut sum = Expr::zero();
        for a in 0..n as u8 {
            let g_ea = match g.get(registry, &[target, a]) {
                Ok(v) => v,
                Err(err_val) => {
                    err = Some(err_val);
                    return;
                }
            };
            if g_ea.is_zero() {
                continue;
            }
            let mut src = idx.to_vec();
            src[0] = a;
            sum = sum + g_ea * grid.get(&src);
        }
        out.set(idx, normalize(&sum));
    });
    if let Some(e) = err {
        return Err(e);
    }
    Ok(out)
}

/// Raises one index of a `Grid` (0-based `position`) through `ginv`:
/// `T^{..a..} = g^{ab} T_{..b..}`.
pub fn raise_index(chart: &Chart, grid: &Grid, ginv: &Grid, position: usize) -> Grid {
    let n = chart.dim();
    let rank = grid.rank();
    let mut out = Grid::new(n, rank);
    for_each_index_tuple(n, rank, |idx| {
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
    });
    out
}

/// `R_bd = R^a_bad` (Ricci tensor, from the mixed Riemann tensor).
pub fn ricci_tensor(chart: &Chart, riemann_mixed: &Grid) -> Grid {
    let n = chart.dim();
    let mut ricci = Grid::new(n, 2);
    for b in 0..n as u8 {
        for d in 0..n as u8 {
            let mut sum = Expr::zero();
            for a in 0..n as u8 {
                sum = sum + riemann_mixed.get(&[a, b, a, d]);
            }
            ricci.set(&[b, d], normalize(&sum));
        }
    }
    ricci
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

/// `R_abcd R^abcd` (the Kretschmann scalar), given the fully covariant
/// Riemann tensor and the inverse metric.
pub fn kretschmann(chart: &Chart, riemann_cov: &Grid, ginv: &Grid) -> Expr {
    let raised0 = raise_index(chart, riemann_cov, ginv, 0);
    let raised1 = raise_index(chart, &raised0, ginv, 1);
    let raised2 = raise_index(chart, &raised1, ginv, 2);
    let riemann_contra = raise_index(chart, &raised2, ginv, 3);

    let n = chart.dim();
    let mut sum = Expr::zero();
    for_each_index_tuple(n, 4, |idx| {
        let term = riemann_cov.get(idx) * riemann_contra.get(idx);
        sum = std::mem::replace(&mut sum, Expr::zero()) + term;
    });
    normalize(&sum)
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
    });
    ct
}
