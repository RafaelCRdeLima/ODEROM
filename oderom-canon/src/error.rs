use thiserror::Error;

/// Errors from [`crate::canonicalize`].
#[derive(Error, Debug)]
pub enum CanonError {
    /// The canonical monomial failed `oderom-core`'s structural validation.
    /// This should be unreachable if the search in [`crate::coset`] is
    /// correct; surfacing it as an error rather than panicking keeps a
    /// latent bug from corrupting output silently.
    #[error("internal canonicalization invariant violated: {0}")]
    Internal(#[from] oderom_core::CoreError),
}
