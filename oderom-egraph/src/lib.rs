//! `oderom-egraph` -- Marco 4: an e-graph over abstract tensor
//! monomials/polynomials, equality saturation, and cost-based
//! extraction, for multi-term identities a pure permutation-symmetry
//! canonicalization can never capture.
//!
//! `oderom-canon` (Marco 1) canonicalizes a single monomial under its
//! head's declared *slot-permutation* symmetry group. The first Bianchi
//! identity, `R[a,b,c,d] + R[a,c,d,b] + R[a,d,b,c] = 0`, is not that kind
//! of fact -- it relates *three* monomials, and the cyclic permutation it
//! uses is not itself a symmetry of Riemann (Riemann's slot-symmetry
//! group has order 8; Bianchi's cyclic permutation has order 3; 3 does
//! not divide 8, so by Lagrange's theorem it simply cannot be a member,
//! not just isn't one by coincidence). See `bianchi` for how it's
//! asserted instead, and `egraph` for why this crate hand-rolls a small
//! e-graph rather than depending on `egg`.

mod bianchi;
mod egraph;
mod extract;
mod union_find;

pub use bianchi::apply_bianchi;
pub use egraph::{EClassId, EGraph, ENode};
pub use extract::extract;
