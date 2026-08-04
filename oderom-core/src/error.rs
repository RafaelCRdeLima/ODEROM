//! Errors from `oderom-core`. These are purely structural/combinatorial:
//! this crate does not know what a "type" is (see `oderom-types`), so
//! nothing here reports bundle or variance incompatibility -- only
//! violations of the contraction-graph invariants themselves.

use crate::monomial::SlotId;
use thiserror::Error;

/// Structural errors from building a [`crate::monomial::Monomial`],
/// [`crate::monomial::Matching`], or [`crate::head::TensorHead`].
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    #[error("name `{0}` is already declared")]
    DuplicateName(String),

    #[error("unknown manifold `{0}`")]
    UnknownManifold(String),

    #[error("unknown bundle `{0}`")]
    UnknownBundle(String),

    #[error("unknown tensor head `{0}`")]
    UnknownHead(String),

    #[error(
        "symmetry generator for `{head}` has permutation of length {found}, but the head has arity {expected}"
    )]
    GeneratorArityMismatch { head: String, expected: usize, found: usize },

    #[error("factor {factor} has head `{head}` of arity {arity}, but slot {slot} was referenced")]
    SlotOutOfRange { factor: usize, head: String, arity: usize, slot: usize },

    #[error("slot {0:?} is used more than once across contractions and free indices")]
    SlotUsedTwice(SlotId),

    #[error("slot {0:?} is neither contracted nor marked free")]
    UnmatchedSlot(SlotId),

    #[error("free index label is bound to more than one slot: {0:?} and {1:?}")]
    DuplicateFreeLabel(SlotId, SlotId),

    #[error(
        "head `{head}` spans two manifolds: slot {first_slot} lives over `{first_manifold}` but slot {other_slot} lives over `{other_manifold}` -- a tensor head belongs to one manifold"
    )]
    HeadSpansManifolds {
        head: String,
        first_slot: usize,
        first_manifold: String,
        other_slot: usize,
        other_manifold: String,
    },

    /// A head with no slots has no manifold to infer, and inventing one
    /// would be guessing. Rank-0 heads need to name their manifold; how
    /// they do is a language decision, deliberately not settled here.
    #[error("head `{head}` has no slots, so there is no manifold to infer -- a rank-0 head must name its manifold")]
    HeadWithoutManifold { head: String },

    /// Which bundle `∇`'s index lives in cannot be decided. Deliberately
    /// an error and not a choice: with one candidate there is nothing to
    /// decide, and with more than one, guessing would put the derivative
    /// index in a bundle of possibly the wrong dimension.
    #[error(
        "cannot tell which bundle a covariant derivative acts on over manifold `{manifold}`: {} candidates ({}) -- mark exactly one with `tangent`, as in `bundle {} on {manifold} dim N tangent`",
        if *marked { "more than one is marked `tangent`" } else { "more than one bundle and none is marked" },
        candidates.join(", "),
        candidates.first().map(String::as_str).unwrap_or("TM")
    )]
    AmbiguousTangentBundle { manifold: String, candidates: Vec<String>, marked: bool },

    #[error("manifold `{manifold}` has no declared bundle, so a covariant derivative has no index to live in")]
    NoTangentBundle { manifold: String },
}
