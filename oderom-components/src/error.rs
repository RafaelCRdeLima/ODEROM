use thiserror::Error;

/// Errors from `oderom-components`.
#[derive(Error, Debug)]
pub enum ComponentError {
    #[error("expected {expected} indices for this tensor's arity, got {found}")]
    ArityMismatch { expected: usize, found: usize },

    #[error("index {index} out of range for chart dimension {dim}")]
    IndexOutOfRange { index: u8, dim: usize },

    #[error(
        "metric has a nonzero off-diagonal component g[{i},{j}]; Marco 2 only inverts diagonal metrics (see DESIGN-M2.md, D-M2.1)"
    )]
    NonDiagonalMetric { i: u8, j: u8 },

    #[error(transparent)]
    Core(#[from] oderom_core::CoreError),

    #[error(transparent)]
    Jit(#[from] oderom_jit::JitError),
}
