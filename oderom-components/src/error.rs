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

    /// `metric_inverse_checkpointed`'s block/general inversion path
    /// (Rodada Metrica Nao-Diagonal, D-M2.1 revisited): a block's
    /// determinant normalized to identically zero, so its adjugate
    /// cannot be divided by it. Real for a genuinely degenerate
    /// metric/chart (e.g. a coordinate singularity written as if it were
    /// regular); never expected from any metric in this project's own
    /// gallery, all of which are non-degenerate in their stated
    /// coordinate range.
    #[error("metric block containing coordinate index {index} has a determinant that normalizes to identically zero -- cannot invert a singular metric")]
    SingularMetric { index: u8 },

    /// [`crate::curvature::verify_metric_inverse`] (the `g_ab g^bc =
    /// delta^a_c` check every inversion path -- diagonal, block, general
    /// -- must satisfy regardless of which one ran): component `(a,c)`
    /// of the product did not normalize to the expected Kronecker delta
    /// entry. This is the specific failure mode block detection being
    /// unsound would produce (a block wrongly treated as decoupled),
    /// which is exactly why it is a distinct, nameable error rather than
    /// a generic assertion failure -- see `metric_block_structure`'s own
    /// doc comment for why detection is conservative by construction and
    /// should never actually produce this.
    #[error("metric inverse verification failed: (g_ab * g^bc) at (a={a}, c={c}) did not normalize to the expected Kronecker delta entry")]
    InverseVerificationFailed { a: u8, c: u8 },

    #[error(transparent)]
    Core(#[from] oderom_core::CoreError),

    #[error(transparent)]
    Jit(#[from] oderom_jit::JitError),

    /// A `*_checkpointed` stage function's checkpoint returned false
    /// mid-loop -- `oderom-session`'s `cancel_running()`
    /// (DESIGN-UI-SESSION.md) firing between components, not just
    /// between whole stages. Never produced by a no-op checkpoint (the
    /// plain, non-`_checkpointed` functions' own internal one), so the
    /// existing infallible/fallible signatures those wrap keep their
    /// current contract exactly.
    #[error("cancelled")]
    Cancelled,

    /// `accel_equations` (Marco 6 step 4, round C): the coordinate's
    /// geodesic equation, once isolated for its own second derivative,
    /// had a coefficient that normalizes to identically zero -- dividing
    /// by it would silently produce a meaningless result, so this is
    /// refused instead. Real for a degenerate metric/chart; never
    /// expected from Schwarzschild or any other well-behaved fixture in
    /// this project's own test suite.
    #[error("the geodesic equation for coordinate `{coordinate}` has a second-derivative coefficient that is identically zero -- cannot solve for its acceleration by dividing by zero")]
    GeodesicCoefficientVanishes { coordinate: String },

    /// `weyl_tensor`/`weyl_squared` (Marco 6 step 6): the standard
    /// formula's `1/(n-2)` and `1/((n-1)(n-2))` factors are undefined at
    /// `n=2` -- not a computation that happens to fail there, a genuine
    /// mathematical fact (the Weyl tensor itself is not a meaningful
    /// object in 2 dimensions, where the Riemann tensor has only one
    /// independent component and carries no conformal information to
    /// separate out). Refused before ever dividing by zero, never
    /// produced as a bogus value. `n=3` is NOT refused here -- the Weyl
    /// tensor is well-defined there and (by a real theorem) identically
    /// zero, which the ordinary formula already produces on its own
    /// without needing a special case.
    #[error("the Weyl tensor is not defined in 2 dimensions (the standard formula's 1/(n-2) term is undefined at n=2)")]
    WeylUndefinedInDimension2,

    // No variant for "the equation is not affine-linear in its own second
    // derivative": that is not a property of the metric/chart the way a
    // vanishing coefficient is -- it is a mathematical fact about what a
    // geodesic equation IS (x_a'' + Gamma^a_bc x_b' x_c' = 0 is affine,
    // coefficient exactly 1, in x_a'', by the definition of the equation,
    // not by an implementation choice this engine happens to make). A
    // future degenerate metric can genuinely zero out that coefficient;
    // no future *correct* geodesic equation can ever be non-linear in its
    // own acceleration. Seeing that would mean either a bug in
    // `geodesic_equations`' own construction or a caller handing
    // `solve_for_second_derivative` something that was never a geodesic
    // equation to begin with -- both are upstream bugs, not legitimate
    // input, so this is a hard `.expect()` panic at the call site
    // (`curvature::solve_for_second_derivative`), not a `Result` a caller
    // could catch and "handle".
}
