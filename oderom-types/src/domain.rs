//! Domain of validity of a typed expression.
//!
//! Marco 1 had no charts, so the only inhabitant was `Everywhere`. Marco 3
//! adds `Restricted`: a chart's domain as a conjunction of symbolic
//! predicates (e.g. a stereographic projection excluding its pole). These
//! are structural data, not proof obligations -- no solver consumes them.
//! Confirmed with the user 2026-07-19: the actual Marco 3 acceptance
//! criterion (metric invariance across a chart transition) is a pointwise
//! symbolic identity, not a claim about inequalities, so it does not need
//! one; a real SMT backend for domain obligations that *do* need
//! automated proof (e.g. "this atlas covers the manifold") is deferred
//! until an acceptance test actually requires it, rather than paid for
//! (a heavy new external dependency) speculatively.

use oderom_expr::Expr;

/// A single constraint a point's coordinates must satisfy for a chart (or
/// an expression) to be valid there.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Predicate {
    /// `expr != 0`.
    Ne(Expr),
    /// `expr > 0`.
    Gt(Expr),
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub enum Domain {
    #[default]
    Everywhere,
    /// The conjunction (logical AND) of every predicate.
    Restricted(Vec<Predicate>),
}
