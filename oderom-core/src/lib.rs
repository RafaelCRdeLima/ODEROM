//! `oderom-core` -- Marco 1.1: the contraction-graph representation of
//! tensor monomials, tensor head declarations, and their symmetry groups.
//!
//! This crate is purely combinatorial. It has no notion of a manifold's
//! chart, a domain of validity, or a type judgment -- those live in
//! `oderom-types`. It only knows about permutation groups and graphs of
//! slot contractions.

pub mod error;
pub mod head;
pub mod monomial;
pub mod perm;
pub mod registry;
pub mod scalar;
pub mod symmetry;

pub use error::CoreError;
pub use head::{HeadId, SlotSig, TensorHead, Variance};
pub use monomial::{AbstractIndex, Factor, Matching, Monomial, Polynomial, SlotId};
pub use perm::{Perm, SignedPerm};
pub use registry::{BundleDecl, BundleId, ManifoldDecl, ManifoldId, Registry};
pub use scalar::Scalar;
pub use symmetry::{totally_antisymmetric_generators, Bsgs, SchreierLevel};
