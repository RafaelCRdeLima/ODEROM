//! Interned declarations: manifolds, bundles, and tensor heads. Every
//! `ManifoldId`/`BundleId`/`HeadId` is an index into a [`Registry`]; none of
//! these types exist outside one, and comparing them is comparing `u32`s,
//! never strings.

use crate::error::CoreError;
use crate::head::{HeadId, SlotSig, TensorHead};
use crate::perm::SignedPerm;
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
