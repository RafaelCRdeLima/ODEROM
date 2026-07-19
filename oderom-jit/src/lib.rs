//! `oderom-jit` -- Marco 5: a linear SSA intermediate representation
//! compiled once from an [`oderom_expr::Expr`] and interpreted many
//! times, for the numerical work Marco 5 needs (evaluating Christoffel
//! symbols at thousands of points while integrating the geodesic and
//! parallel-transport equations) without re-walking the symbolic
//! expression tree on every call.
//!
//! "IR em SSA... JIT" in the original roadmap named the mechanism, not
//! literally "generate native machine code": confirmed with the user
//! that this crate stops at a fast interpreter over the compiled form
//! (`Program::eval`), rather than adding a code-generation backend
//! (`cranelift` or similar) -- a categorically heavier dependency than
//! anything used through Marco 4, for a job a plain interpreter already
//! does fast enough at the scale a hand-rolled RK4 integrator needs.
//! See DESIGN-M5.md, D5.1.
//!
//! [`compile`] performs common-subexpression elimination via hash-consing
//! during lowering (same technique `oderom-egraph` uses for its e-nodes,
//! simpler here since there is no need for a union-find: nothing gets
//! merged after the fact, only deduplicated as it's built): two
//! structurally-equal subexpressions become the same instruction.
//!
//! [`Op`] is deliberately small -- exactly what [`oderom_expr::Expr`]
//! has (`Add`, `Mul`, `Pow`, `Sin`, `Cos`, plus `Const`/`Var` leaves).
//! `Expr::Rational` becomes a floating-point `Const` here: Marco 5 is
//! the first place in this project that evaluates rather than reasons
//! about expressions exactly, so the loss of exactness is the point,
//! not a regression.

mod compile;
mod error;
mod program;

pub use compile::compile;
pub use error::JitError;
pub use program::{Op, Program};
