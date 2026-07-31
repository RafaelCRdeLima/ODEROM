//! Interned declarations: manifolds, bundles, and tensor heads. Every
//! `ManifoldId`/`BundleId`/`HeadId` is an index into a [`Registry`]; none of
//! these types exist outside one, and comparing them is comparing `u32`s,
//! never strings.

use crate::error::CoreError;
use crate::head::{HeadId, SlotSig, TensorHead, Variance};
use crate::perm::{Perm, SignedPerm};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

/// An index into a [`Registry`]'s declared manifolds. Meaningless outside
/// the `Registry` that produced it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ManifoldId(u32);

/// An index into a [`Registry`]'s declared bundles.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BundleId(u32);

/// A declared manifold: just a name and a literal dimension in Marco 1
/// (no charts yet).
#[derive(Clone, Debug)]
pub struct ManifoldDecl {
    pub name: String,
    pub dim: u32,
}

/// A declared vector bundle over a manifold. Its dual is not a separate
/// declaration: a slot's [`crate::head::Variance`] says whether it is a
/// section of this bundle or of its dual.
#[derive(Clone, Debug)]
pub struct BundleDecl {
    pub name: String,
    pub base: ManifoldId,
    pub dim: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum NameEntry {
    Manifold(ManifoldId),
    Bundle(BundleId),
    Head(HeadId),
}

/// The interner and declaration store for one session's manifolds,
/// bundles, and tensor heads. See the module docs.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    manifolds: Vec<ManifoldDecl>,
    bundles: Vec<BundleDecl>,
    heads: Vec<TensorHead>,
    names: FxHashMap<String, NameEntry>,
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    /// Declares a new manifold. Errors if `name` is already taken by any
    /// manifold, bundle, or head.
    pub fn declare_manifold(&mut self, name: &str, dim: u32) -> Result<ManifoldId, CoreError> {
        self.check_free_name(name)?;
        let id = ManifoldId(self.manifolds.len() as u32);
        self.manifolds.push(ManifoldDecl { name: name.to_string(), dim });
        self.names.insert(name.to_string(), NameEntry::Manifold(id));
        Ok(id)
    }

    /// Declares a new bundle over `base`. Errors if `name` is taken.
    pub fn declare_bundle(
        &mut self,
        name: &str,
        base: ManifoldId,
        dim: u32,
    ) -> Result<BundleId, CoreError> {
        self.check_free_name(name)?;
        let id = BundleId(self.bundles.len() as u32);
        self.bundles.push(BundleDecl { name: name.to_string(), base, dim });
        self.names.insert(name.to_string(), NameEntry::Bundle(id));
        Ok(id)
    }

    /// Declares a new tensor head, computing and memoizing its symmetry
    /// group's BSGS (see [`crate::symmetry::Bsgs::from_generators`]).
    /// Errors if `name` is taken or a generator's permutation length
    /// doesn't match `slots.len()`.
    pub fn declare_head(
        &mut self,
        name: &str,
        slots: SmallVec<[SlotSig; 4]>,
        symmetry_generators: Vec<SignedPerm>,
    ) -> Result<HeadId, CoreError> {
        self.check_free_name(name)?;
        for g in &symmetry_generators {
            if g.perm.len() != slots.len() {
                return Err(CoreError::GeneratorArityMismatch {
                    head: name.to_string(),
                    expected: slots.len(),
                    found: g.perm.len(),
                });
            }
        }
        let id = HeadId(self.heads.len() as u32);
        self.heads.push(TensorHead::new(id, name.to_string(), slots, symmetry_generators));
        self.names.insert(name.to_string(), NameEntry::Head(id));
        Ok(id)
    }

    /// The head representing `k` covariant derivatives of `base`, e.g.
    /// `∇_c T_ab` for `k = 1` -- created on first use and reused after,
    /// so the same derivative in two different monomials is the same
    /// head and therefore comparable.
    ///
    /// Arity is `arity(base) + k`, with the derivative slots last.
    /// `base`'s symmetry generators are extended to fix those trailing
    /// slots (a permutation of the first `n` becomes a permutation of
    /// `n + k` fixing the tail), so the differentiated tensor keeps
    /// exactly the symmetry it had and gains none. In particular nothing
    /// is declared between two derivative slots: `∇_a ∇_b` and
    /// `∇_b ∇_a` differ by the Riemann tensor, so calling them symmetric
    /// would be asserting flatness.
    ///
    /// The synthesized name is `base;k`, which is not a legal
    /// user-declared name (`;` cannot appear in an identifier), so this
    /// can never collide with something the user declared.
    pub fn derivative_head(&mut self, base: HeadId, k: u8) -> Result<HeadId, CoreError> {
        if k == 0 {
            return Ok(base);
        }
        let name = format!("{};{k}", self.head(base).name);
        if let Some(NameEntry::Head(id)) = self.names.get(&name) {
            return Ok(*id);
        }

        let base_head = self.head(base);
        let base_arity = base_head.arity();
        let total = base_arity + k as usize;

        // Derivative slots carry the same signature as the manifold's
        // own covariant index: a derivative index is a lower index on
        // the same bundle. Taking it from the base head's first slot
        // keeps bundle/dimension consistent without inventing one.
        let proto = *base_head.slots.first().ok_or(CoreError::UnknownHead(name.clone()))?;
        let mut slots: SmallVec<[SlotSig; 4]> = base_head.slots.clone();
        for _ in 0..k {
            slots.push(SlotSig { variance: Variance::Co, ..proto });
        }

        let generators: Vec<SignedPerm> = base_head
            .symmetry_generators
            .iter()
            .map(|g| {
                let mut images: Vec<u16> = (0..total as u16).collect();
                for i in 0..base_arity {
                    images[i] = g.perm.image(i as u16);
                }
                SignedPerm::new(Perm::try_from_images(&images).expect("extending a permutation with fixed points stays a permutation"), g.sign)
            })
            .collect();

        let id = HeadId(self.heads.len() as u32);
        let mut head = TensorHead::new(id, name.clone(), slots, generators);
        head.derivative_of = Some((base, k));
        self.heads.push(head);
        self.names.insert(name, NameEntry::Head(id));
        Ok(id)
    }

    fn check_free_name(&self, name: &str) -> Result<(), CoreError> {
        if self.names.contains_key(name) {
            Err(CoreError::DuplicateName(name.to_string()))
        } else {
            Ok(())
        }
    }

    pub fn manifold(&self, id: ManifoldId) -> &ManifoldDecl {
        &self.manifolds[id.0 as usize]
    }

    pub fn bundle(&self, id: BundleId) -> &BundleDecl {
        &self.bundles[id.0 as usize]
    }

    pub fn head(&self, id: HeadId) -> &TensorHead {
        &self.heads[id.0 as usize]
    }

    /// Resolves a declared manifold's name to its id.
    pub fn lookup_manifold(&self, name: &str) -> Result<ManifoldId, CoreError> {
        match self.names.get(name) {
            Some(NameEntry::Manifold(id)) => Ok(*id),
            _ => Err(CoreError::UnknownManifold(name.to_string())),
        }
    }

    /// Resolves a declared bundle's name to its id.
    pub fn lookup_bundle(&self, name: &str) -> Result<BundleId, CoreError> {
        match self.names.get(name) {
            Some(NameEntry::Bundle(id)) => Ok(*id),
            _ => Err(CoreError::UnknownBundle(name.to_string())),
        }
    }

    /// Resolves a declared tensor head's name to its id.
    pub fn lookup_head(&self, name: &str) -> Result<HeadId, CoreError> {
        match self.names.get(name) {
            Some(NameEntry::Head(id)) => Ok(*id),
            _ => Err(CoreError::UnknownHead(name.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::head::Variance;

    #[test]
    fn declares_and_looks_up_manifold() {
        let mut reg = Registry::new();
        let m = reg.declare_manifold("M", 4).unwrap();
        assert_eq!(reg.manifold(m).dim, 4);
        assert_eq!(reg.lookup_manifold("M").unwrap(), m);
    }

    #[test]
    fn duplicate_name_across_kinds_is_rejected() {
        let mut reg = Registry::new();
        reg.declare_manifold("M", 4).unwrap();
        let err = reg.declare_manifold("M", 4).unwrap_err();
        assert_eq!(err, CoreError::DuplicateName("M".to_string()));

        let err2 = reg.declare_bundle("M", reg.lookup_manifold("M").unwrap(), 4).unwrap_err();
        assert_eq!(err2, CoreError::DuplicateName("M".to_string()));
    }

    #[test]
    fn unknown_lookups_error() {
        let reg = Registry::new();
        assert!(matches!(reg.lookup_manifold("M"), Err(CoreError::UnknownManifold(_))));
        assert!(matches!(reg.lookup_bundle("TM"), Err(CoreError::UnknownBundle(_))));
        assert!(matches!(reg.lookup_head("R"), Err(CoreError::UnknownHead(_))));
    }

    #[test]
    fn head_generator_arity_mismatch_is_rejected() {
        use crate::perm::{Perm, SignedPerm};
        let mut reg = Registry::new();
        let m = reg.declare_manifold("M", 4).unwrap();
        let tm = reg.declare_bundle("TM", m, 4).unwrap();
        let slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![
            SlotSig { bundle: tm, variance: Variance::Co, dim: 4 },
            SlotSig { bundle: tm, variance: Variance::Co, dim: 4 },
        ];
        let bad_gen = SignedPerm::new(Perm::identity(3), 1);
        let err = reg.declare_head("g", slots, vec![bad_gen]).unwrap_err();
        assert_eq!(
            err,
            CoreError::GeneratorArityMismatch { head: "g".to_string(), expected: 2, found: 3 }
        );
    }
}

#[cfg(test)]
mod derivative_head_tests {
    use super::*;

    fn riemann() -> (Registry, HeadId) {
        let mut reg = Registry::new();
        let m = reg.declare_manifold("M", 4).unwrap();
        let tm = reg.declare_bundle("TM", m, 4).unwrap();
        let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };
        let slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co, co, co];
        let gens = vec![
            SignedPerm::new(Perm::transposition(4, 0, 1), -1),
            SignedPerm::new(Perm::transposition(4, 2, 3), -1),
            SignedPerm::new(Perm::try_from_images(&[2, 3, 0, 1]).unwrap(), 1),
        ];
        let r = reg.declare_head("R", slots, gens).unwrap();
        (reg, r)
    }

    #[test]
    fn one_derivative_adds_exactly_one_trailing_slot() {
        let (mut reg, r) = riemann();
        let dr = reg.derivative_head(r, 1).unwrap();
        assert_eq!(reg.head(dr).arity(), 5);
        assert_eq!(reg.head(dr).derivative_count(), 1);
        assert_eq!(reg.head(dr).base_head(), r);
    }

    /// The differentiated tensor keeps exactly the base's symmetry and
    /// gains none: every generator must fix the derivative slot.
    #[test]
    fn the_derivative_slot_is_fixed_by_every_inherited_generator() {
        let (mut reg, r) = riemann();
        let dr = reg.derivative_head(r, 1).unwrap();
        let head = reg.head(dr);
        assert_eq!(head.symmetry_generators.len(), reg.head(r).symmetry_generators.len());
        for g in &head.symmetry_generators {
            assert_eq!(g.perm.len(), 5);
            assert_eq!(g.perm.image(4), 4, "a derivative slot must never be permuted with a tensor slot");
        }
    }

    /// Load-bearing absence: nothing may relate two derivative slots to
    /// each other. `∇_a ∇_b` and `∇_b ∇_a` differ by the Riemann tensor,
    /// so declaring them symmetric would silently assert flatness. If a
    /// future change starts generating a symmetry here, this fails.
    #[test]
    fn two_derivative_slots_are_not_declared_symmetric() {
        let (mut reg, r) = riemann();
        let ddr = reg.derivative_head(r, 2).unwrap();
        let head = reg.head(ddr);
        assert_eq!(head.arity(), 6);
        for g in &head.symmetry_generators {
            assert_eq!(g.perm.image(4), 4, "derivative slots must stay fixed");
            assert_eq!(g.perm.image(5), 5, "derivative slots must stay fixed");
        }
    }

    /// The same derivative asked for twice is the same head, so two
    /// monomials mentioning it are comparable.
    #[test]
    fn the_same_derivative_is_cached_not_redeclared() {
        let (mut reg, r) = riemann();
        assert_eq!(reg.derivative_head(r, 1).unwrap(), reg.derivative_head(r, 1).unwrap());
        assert_ne!(reg.derivative_head(r, 1).unwrap(), reg.derivative_head(r, 2).unwrap());
    }

    #[test]
    fn zero_derivatives_is_the_base_head_itself() {
        let (mut reg, r) = riemann();
        assert_eq!(reg.derivative_head(r, 0).unwrap(), r);
    }
}
