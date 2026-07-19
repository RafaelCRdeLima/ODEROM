//! Errors from `oderom-core`. These are purely structural/combinatorial:
//! this crate does not know what a "type" is (see `oderom-types`), so
//! nothing here reports bundle or variance incompatibility -- only
//! violations of the contraction-graph invariants themselves.

use crate::monomial::SlotId;
use thiserror::Error;

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
}
