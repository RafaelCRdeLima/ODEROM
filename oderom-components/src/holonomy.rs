//! Numerical geodesic integration and parallel transport (Marco 5): the
//! first place in this project that evaluates rather than reasons about
//! tensor components exactly. The Christoffel symbols computed
//! symbolically by `curvature::christoffel` are compiled once, via
//! `oderom_jit::compile`, into fast-to-evaluate programs, then evaluated
//! thousands of times by a hand-rolled fixed-step RK4 integrator (see
//! DESIGN-M5.md, D5.2 for why no numerical-integration dependency was
//! added).
//!
//! Geodesic and parallel transport are integrated *together*, as one
//! coupled system, rather than tracing the geodesic first and
//! transporting a vector along the recorded path second: RK4's
//! intermediate stages need the trajectory's state at times between the
//! recorded steps, and computing those consistently for both `(x, v)`
//! and `w` in the same step is both simpler and more accurate than
//! trying to reconstruct them for a separately-run transport pass.

use crate::chart::Chart;
use crate::error::ComponentError;
use crate::grid::Grid;
use oderom_jit::Program;

/// Every component `Gamma^i_{jk}` of a Christoffel symbol, each compiled
/// into a [`Program`] over the chart's coordinates, for repeated
/// numerical evaluation.
pub struct ChristoffelPrograms {
    dim: usize,
    programs: Vec<Program>,
}

impl ChristoffelPrograms {
    /// Compiles every `Gamma^i_{jk}` in `gamma` (indexed `[i,j,k]`, as
    /// produced by `curvature::christoffel`) into a [`Program`] over
    /// `chart`'s coordinates.
    pub fn compile(chart: &Chart, gamma: &Grid) -> Result<Self, ComponentError> {
        let dim = chart.dim();
        let mut programs = Vec::with_capacity(dim * dim * dim);
        for i in 0..dim as u8 {
            for j in 0..dim as u8 {
                for k in 0..dim as u8 {
                    let expr = gamma.get(&[i, j, k]);
                    programs.push(oderom_jit::compile(&expr, &chart.coords)?);
                }
            }
        }
        Ok(ChristoffelPrograms { dim, programs })
    }

    fn eval(&self, i: usize, j: usize, k: usize, x: &[f64]) -> f64 {
        self.programs[(i * self.dim + j) * self.dim + k].eval(x)
    }
}

/// `d(x,v,w)/dt` for the coupled geodesic (`x`,`v`) and parallel-
/// transport (`w`) system:
/// `dx^i/dt = v^i`, `dv^i/dt = -Gamma^i_jk v^j v^k`, `dw^i/dt = -Gamma^i_jk v^j w^k`.
fn rhs(cp: &ChristoffelPrograms, x: &[f64], v: &[f64], w: &[f64]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let dim = cp.dim;
    let dxdt = v.to_vec();
    let mut dvdt = vec![0.0; dim];
    let mut dwdt = vec![0.0; dim];
    for i in 0..dim {
        let mut acc_v = 0.0;
        let mut acc_w = 0.0;
        for j in 0..dim {
            for k in 0..dim {
                let g = cp.eval(i, j, k, x);
                acc_v += g * v[j] * v[k];
                acc_w += g * v[j] * w[k];
            }
        }
        dvdt[i] = -acc_v;
        dwdt[i] = -acc_w;
    }
    (dxdt, dvdt, dwdt)
}

fn axpy(a: &[f64], scale: f64, b: &[f64]) -> Vec<f64> {
    a.iter().zip(b).map(|(ai, bi)| ai + scale * bi).collect()
}

fn combine4(dt: f64, k1: &[f64], k2: &[f64], k3: &[f64], k4: &[f64]) -> Vec<f64> {
    (0..k1.len()).map(|i| dt / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i])).collect()
}

/// One fixed-step RK4 step of the coupled system.
fn rk4_step(
    cp: &ChristoffelPrograms,
    x: &[f64],
    v: &[f64],
    w: &[f64],
    dt: f64,
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let (k1x, k1v, k1w) = rhs(cp, x, v, w);

    let x2 = axpy(x, dt / 2.0, &k1x);
    let v2 = axpy(v, dt / 2.0, &k1v);
    let w2 = axpy(w, dt / 2.0, &k1w);
    let (k2x, k2v, k2w) = rhs(cp, &x2, &v2, &w2);

    let x3 = axpy(x, dt / 2.0, &k2x);
    let v3 = axpy(v, dt / 2.0, &k2v);
    let w3 = axpy(w, dt / 2.0, &k2w);
    let (k3x, k3v, k3w) = rhs(cp, &x3, &v3, &w3);

    let x4 = axpy(x, dt, &k3x);
    let v4 = axpy(v, dt, &k3v);
    let w4 = axpy(w, dt, &k3w);
    let (k4x, k4v, k4w) = rhs(cp, &x4, &v4, &w4);

    let x_next = axpy(x, 1.0, &combine4(dt, &k1x, &k2x, &k3x, &k4x));
    let v_next = axpy(v, 1.0, &combine4(dt, &k1v, &k2v, &k3v, &k4v));
    let w_next = axpy(w, 1.0, &combine4(dt, &k1w, &k2w, &k3w, &k4w));
    (x_next, v_next, w_next)
}

/// Integrates the geodesic starting at `(x0, v0)` for `duration` (in the
/// affine parameter -- arc length, if `v0` has unit speed in the
/// metric), carrying a parallel-transported vector `w0` along for the
/// ride, over `steps` fixed-size RK4 steps. Returns the final
/// `(position, velocity, transported vector)`.
pub fn integrate_geodesic_with_transport(
    christoffel: &ChristoffelPrograms,
    x0: &[f64],
    v0: &[f64],
    w0: &[f64],
    duration: f64,
    steps: usize,
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let dt = duration / steps as f64;
    let mut x = x0.to_vec();
    let mut v = v0.to_vec();
    let mut w = w0.to_vec();
    for _ in 0..steps {
        let (xn, vn, wn) = rk4_step(christoffel, &x, &v, &w, dt);
        x = xn;
        v = vn;
        w = wn;
    }
    (x, v, w)
}
