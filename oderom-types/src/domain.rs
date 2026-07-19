//! Domain of validity of a typed expression.
//!
//! Marco 1 has no charts and no SMT obligations, so the only inhabitant is
//! `Everywhere`. The field exists on [`crate::judgment::ExprType`] already
//! so that Marco 3 (atlases with domains proved by SMT) does not require
//! touching every call site that builds an `ExprType`.

#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub enum Domain {
    #[default]
    Everywhere,
}
