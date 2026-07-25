//! `oderom-core` -- Marco 1.1: the contraction-graph representation of
//! tensor monomials, tensor head declarations, and their symmetry groups.
//!
//! This crate is purely combinatorial. It has no notion of a manifold's
//! chart, a domain of validity, or a type judgment -- those live in
//! `oderom-types`. It only knows about permutation groups and graphs of
//! slot contractions.

pub mod cancel;
pub mod error;
pub mod head;
pub mod monomial;
pub mod perm;
pub mod registry;
pub mod render;
pub mod scalar;
pub mod symmetry;

pub use cancel::CancelToken;
pub use error::CoreError;

#[cfg(test)]
mod tests {
    /// Permanent canary, not a one-off check: Rust's `release` profile
    /// does not check integer overflow by default (it wraps silently),
    /// which was the exact concern behind `BigScalar`/`num-bigint`
    /// (`oderom-expr/Cargo.toml`'s own comment). The workspace
    /// `Cargo.toml` sets `overflow-checks = true` in `[profile.release]`
    /// specifically so plain-integer arithmetic elsewhere in the
    /// codebase (loop counters, index/degree computations -- anything
    /// not already routed through `BigScalar`) still panics loudly on
    /// overflow instead of wrapping. This test catches the day someone
    /// edits that line away (or a new profile is added without it)
    /// silently: it fails under any profile where overflow-checks is
    /// off, in *any* `cargo test` invocation, not just one someone
    /// remembers to run with `--release`.
    ///
    /// `std::hint::black_box` on both operands: without it, `rustc`
    /// evaluates `u32::MAX + 1` as a compile-time constant and refuses
    /// to build at all (a *different* guarantee -- const-eval overflow
    /// detection, not the runtime `overflow-checks` flag this test is
    /// actually about) instead of panicking at runtime.
    #[test]
    #[should_panic(expected = "attempt to add with overflow")]
    fn overflow_checks_are_enabled_in_this_build() {
        let x = std::hint::black_box(u32::MAX);
        let y = std::hint::black_box(1u32);
        let _ = x + y;
    }
}
pub use head::{HeadId, SlotSig, TensorHead, Variance};
pub use monomial::{AbstractIndex, Factor, Matching, Monomial, Polynomial, SlotId};
pub use perm::{Perm, SignedPerm};
pub use registry::{BundleDecl, BundleId, ManifoldDecl, ManifoldId, Registry};
pub use render::{Render, Target};
pub use scalar::Scalar;
pub use symmetry::{totally_antisymmetric_generators, Bsgs, SchreierLevel};
