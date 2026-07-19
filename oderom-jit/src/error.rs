use thiserror::Error;

/// Errors from [`crate::compile`].
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum JitError {
    #[error("variable `{0}` appears in the expression but is not listed in `vars`")]
    UnknownVariable(String),
}
