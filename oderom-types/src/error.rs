//! Type errors, reported in geometric language (which bundle, which
//! variance, which manifold) rather than compiler jargon.

use oderom_core::{SlotId, SlotSig, Variance};
use thiserror::Error;

/// Failures of the type judgment `Gamma |- T : Section(...)`, one variant
/// per rule in [`crate::judgment`].
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum TypeError {
    #[error(
        "slot {left_slot:?} of `{left_head}` is a section of {left_kind}, and cannot contract with slot {right_slot:?} of `{right_head}`, a section of {right_kind}"
    )]
    IncompatibleContraction {
        left_slot: SlotId,
        left_head: String,
        left_kind: BundleDescription,
        right_slot: SlotId,
        right_head: String,
        right_kind: BundleDescription,
    },

    #[error(
        "term {term} mixes geometry from manifold `{expected}` with a slot on manifold `{found}`; every factor of one monomial must live over the same base manifold"
    )]
    ManifoldMismatch { term: usize, expected: String, found: String },

    #[error("a monomial with no tensor factors has no manifold to be a section over")]
    EmptyMonomial,

    #[error("a sum of zero terms has no type")]
    EmptyPolynomial,

    #[error(
        "term {term} has free indices {found:?}, but the sum requires {expected:?} (every term of a sum must carry the same free indices)"
    )]
    FreeIndexMismatch { term: usize, expected: Vec<String>, found: Vec<String> },

    #[error(
        "term {term}'s free index `{label}` is a section of a different bundle/variance than the same index in earlier terms"
    )]
    FreeSlotSigMismatch { term: usize, label: String },
}

/// Human-readable "section of E, upper/lower index" description used in
/// error messages, independent of the bundle's interned name.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct BundleDescription(pub String);

impl BundleDescription {
    pub fn new(bundle_name: &str, variance: Variance, slot: SlotSig) -> Self {
        let dual_marker = match variance {
            Variance::Contra => bundle_name.to_string(),
            Variance::Co => format!("{bundle_name}*"),
        };
        BundleDescription(format!("{dual_marker} (dim {})", slot.dim))
    }
}

impl std::fmt::Display for BundleDescription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
