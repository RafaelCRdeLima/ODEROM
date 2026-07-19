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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
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
}

impl TensorHead {
    pub fn arity(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn new(
        id: HeadId,
        name: String,
        slots: SmallVec<[SlotSig; 4]>,
        symmetry_generators: Vec<SignedPerm>,
    ) -> Self {
        let symmetry = Bsgs::from_generators(slots.len(), &symmetry_generators);
        TensorHead { id, name, slots, symmetry_generators, symmetry }
    }
}
