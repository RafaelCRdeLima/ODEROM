//! Tensor heads: the declared "vocabulary" (Riemann, metric, Levi-Civita,
//! ...) that factors in a [`crate::monomial::Monomial`] refer to.

use crate::perm::SignedPerm;
use crate::registry::BundleId;
use crate::symmetry::Bsgs;
use smallvec::SmallVec;

/// Whether a slot is a contravariant (upper) or covariant (lower) index.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Variance {
    Contra,
    Co,
}

impl Variance {
    /// The variance a slot must have to be contractible with `self`.
    pub fn dual(self) -> Variance {
        match self {
            Variance::Contra => Variance::Co,
            Variance::Co => Variance::Contra,
        }
    }
}

/// The bundle and variance of a single tensor slot. Dimension is a literal
/// integer in Marco 1 (no symbolic dimensions yet).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SlotSig {
    pub bundle: BundleId,
    pub variance: Variance,
    pub dim: u32,
}

/// Ordered by declaration order in the `Registry`. `oderom-canon` relies on
/// this to build a canonical, input-order-independent factor layout.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct HeadId(pub(crate) u32);

/// A declared tensor head: its slot signature and the symmetry group
/// acting on its slots, with the group's BSGS precomputed once here.
#[derive(Clone, Debug)]
pub struct TensorHead {
    pub id: HeadId,
    pub name: String,
    pub slots: SmallVec<[SlotSig; 4]>,
    pub symmetry_generators: Vec<SignedPerm>,
    pub symmetry: Bsgs,
    /// `Some((base, k))` when this head is `k` covariant derivatives of
    /// `base` -- what `T[a,b;c]` (standard GR notation: comma for
    /// partial, semicolon for covariant) declares. `None` for an
    /// ordinary declared head.
    ///
    /// The derivative slots are the last `k`, and the base head's
    /// symmetry generators are extended to fix them: `∇_c T_ab` inherits
    /// whatever symmetry `T_ab` has between `a` and `b`, and has none at
    /// all involving `c`. Nothing relates two derivative slots to each
    /// other either, and that absence is load-bearing rather than an
    /// omission: `∇_a ∇_b` and `∇_b ∇_a` genuinely differ, and their
    /// difference is exactly the Riemann tensor. Declaring them
    /// symmetric would silently assert flatness.
    pub derivative_of: Option<(HeadId, u8)>,
}

impl TensorHead {
    /// Number of slots.
    pub fn arity(&self) -> usize {
        self.slots.len()
    }

    /// How many of this head's trailing slots are covariant-derivative
    /// indices (0 for an ordinary head).
    pub fn derivative_count(&self) -> usize {
        self.derivative_of.map_or(0, |(_, k)| k as usize)
    }

    /// The head this one differentiates, or itself when it is not a
    /// derivative.
    pub fn base_head(&self) -> HeadId {
        self.derivative_of.map_or(self.id, |(base, _)| base)
    }

    pub(crate) fn new(
        id: HeadId,
        name: String,
        slots: SmallVec<[SlotSig; 4]>,
        symmetry_generators: Vec<SignedPerm>,
    ) -> Self {
        let symmetry = Bsgs::from_generators(slots.len(), &symmetry_generators);
        TensorHead { id, name, slots, symmetry_generators, symmetry, derivative_of: None }
    }
}
