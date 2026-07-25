use thiserror::Error;

/// Errors from [`crate::compile`].
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum JitError {
    #[error("variable `{0}` appears in the expression but is not listed in `vars`")]
    UnknownVariable(String),
    /// An indeterminate function (`f(r)`, Marco 6 step 4) has no known
    /// numeric value, so it cannot lower to an IR that evaluates one --
    /// unlike `sin`/`cos`/`exp`/`sinh`/`cosh`, which do. This is a clean
    /// refusal, not a wrong number: the geodesic/holonomy integrator
    /// (`oderom-components::holonomy`, the one real consumer of this
    /// crate) needs actual numbers to step forward, and an unevaluated
    /// symbol is not one.
    #[error("cannot compile indeterminate function `{0}` to numeric code -- it has no known value")]
    IndeterminateFunction(String),
}
