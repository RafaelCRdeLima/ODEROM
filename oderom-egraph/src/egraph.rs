//! An e-graph over abstract tensor monomials/polynomials: e-classes of
//! equivalent [`oderom_core::Polynomial`]-shaped values, related by
//! `union` (asserting two things are equal) and kept consistent by
//! [`EGraph::rebuild`] (congruence closure -- if two `Sum` nodes' children
//! become equivalent, the sums themselves must too).
//!
//! Deliberately not the `egg` crate: that is a general-purpose e-graph
//! library (pattern-rewrite rules, e-matching, a much larger surface than
//! this project needs) and a heavy new dependency for what turns out to
//! be a small, specific job -- asserting that a handful of Riemann-
//! monomial triples sum to zero (see `bianchi.rs`) and extracting a
//! minimal-cost representative afterward. Same reasoning as building
//! Schreier-Sims and the scalar CAS by hand in earlier marcos rather than
//! reaching for an external library.

use crate::union_find::UnionFind;
use oderom_canon::canonicalize;
use oderom_core::{Monomial, Registry};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

/// An index into an [`EGraph`]'s e-classes. Not stable across `union`:
/// always pass it through [`EGraph::find`] (or a method that already
/// does, like [`EGraph::add`] and [`EGraph::union`]) before comparing two
/// ids for equivalence.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct EClassId(u32);

/// One way to build a value: either a single already-canonical monomial,
/// or the sum of several e-classes' values. `Sum([])` is the canonical
/// representation of zero.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ENode {
    Term(Monomial),
    Sum(SmallVec<[EClassId; 4]>),
}

/// A set of e-classes over [`ENode`]s, with hash-consing and
/// (after [`EGraph::rebuild`]) congruence closure.
#[derive(Default)]
pub struct EGraph {
    uf: UnionFind,
    hashcons: FxHashMap<ENode, EClassId>,
    classes: FxHashMap<EClassId, Vec<ENode>>,
}

impl EGraph {
    /// An empty e-graph.
    pub fn new() -> Self {
        EGraph::default()
    }

    /// The current representative of the e-class containing `id`.
    pub fn find(&mut self, id: EClassId) -> EClassId {
        EClassId(self.uf.find(id.0))
    }

    /// Adds `node`, or returns the existing e-class if an equivalent node
    /// (same variant, same canonicalized children) is already present.
    pub fn add(&mut self, node: ENode) -> EClassId {
        let canon = self.canonicalize_node(&node);
        if let Some(&id) = self.hashcons.get(&canon) {
            return self.find(id);
        }
        let id = EClassId(self.uf.make_set());
        self.hashcons.insert(canon.clone(), id);
        self.classes.entry(id).or_default().push(canon);
        id
    }

    /// Canonicalizes `m` (via `oderom_canon::canonicalize`) and adds it
    /// as a [`ENode::Term`] -- or, if `m` is forced to zero by its own
    /// symmetry (see `oderom-canon`), returns [`EGraph::zero`] directly.
    pub fn add_monomial(&mut self, registry: &Registry, m: &Monomial) -> EClassId {
        match canonicalize(m, registry).expect("m was already validated by Monomial::try_new") {
            oderom_canon::CanonResult::Zero => self.zero(),
            oderom_canon::CanonResult::Value(c) => self.add(ENode::Term(c.monomial)),
        }
    }

    /// The e-class for the empty sum, i.e. zero.
    pub fn zero(&mut self) -> EClassId {
        self.add(ENode::Sum(SmallVec::new()))
    }

    /// Asserts `a` and `b` denote the same value.
    pub fn union(&mut self, a: EClassId, b: EClassId) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        let new_root = EClassId(self.uf.union(ra.0, rb.0));
        let other = if new_root == ra { rb } else { ra };
        if let Some(moved) = self.classes.remove(&other) {
            self.classes.entry(new_root).or_default().extend(moved);
        }
    }

    fn canonicalize_node(&mut self, node: &ENode) -> ENode {
        match node {
            ENode::Term(m) => ENode::Term(m.clone()),
            ENode::Sum(children) => {
                let mut canon: SmallVec<[EClassId; 4]> = children.iter().map(|&c| self.find(c)).collect();
                canon.sort_by_key(|c| c.0);
                ENode::Sum(canon)
            }
        }
    }

    /// Restores congruence closure after one or more `union` calls: two
    /// `Sum` nodes whose children are now equivalent (per `find`) must
    /// themselves be unioned too, which can in turn make further nodes
    /// equivalent -- repeated until nothing changes.
    pub fn rebuild(&mut self) {
        loop {
            let mut new_hashcons: FxHashMap<ENode, EClassId> = FxHashMap::default();
            let mut to_union: Vec<(EClassId, EClassId)> = Vec::new();

            let class_ids: Vec<EClassId> = self.classes.keys().copied().collect();
            for class_id in class_ids {
                let root = self.find(class_id);
                let Some(nodes) = self.classes.get(&class_id).cloned() else { continue };
                for node in nodes {
                    let canon = self.canonicalize_node(&node);
                    match new_hashcons.get(&canon) {
                        Some(&existing) if existing != root => to_union.push((existing, root)),
                        _ => {
                            new_hashcons.insert(canon, root);
                        }
                    }
                }
            }

            if to_union.is_empty() {
                self.hashcons = new_hashcons;
                break;
            }
            for (a, b) in to_union {
                self.union(a, b);
            }
        }
    }

    /// All current e-classes (keyed by representative) and their member
    /// nodes.
    pub fn classes(&self) -> impl Iterator<Item = (EClassId, &[ENode])> {
        self.classes.iter().map(|(&id, nodes)| (id, nodes.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oderom_core::{AbstractIndex, Factor, HeadId, Matching, Scalar, SlotId, SlotSig, Variance};
    use smallvec::smallvec;

    /// A single-slot, unconstrained head "V", just for exercising the
    /// e-graph plumbing without dragging in a full Riemann setup.
    fn vector_registry() -> (Registry, HeadId) {
        let mut reg = Registry::new();
        let m = reg.declare_manifold("M", 4).unwrap();
        let tm = reg.declare_bundle("TM", m, 4).unwrap();
        let slots = smallvec![SlotSig { bundle: tm, variance: Variance::Co, dim: 4 }];
        let v = reg.declare_head("V", slots, vec![]).unwrap();
        (reg, v)
    }

    fn vector_monomial(head: HeadId, registry: &Registry, name: &str) -> Monomial {
        let factors: SmallVec<[Factor; 4]> = smallvec![Factor { head }];
        let free = vec![(SlotId { factor: 0, slot: 0 }, AbstractIndex::new(name))];
        Monomial::try_new(Scalar::ONE, factors, Matching::default(), free, registry).unwrap()
    }

    #[test]
    fn adding_the_same_monomial_twice_hashcons_to_one_class() {
        let (reg, v) = vector_registry();
        let mut eg = EGraph::new();
        let a1 = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x"));
        let a2 = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x"));
        assert_eq!(eg.find(a1), eg.find(a2));
    }

    #[test]
    fn distinct_monomials_start_in_distinct_classes() {
        let (reg, v) = vector_registry();
        let mut eg = EGraph::new();
        let a = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x"));
        let b = eg.add_monomial(&reg, &vector_monomial(v, &reg, "y"));
        assert_ne!(eg.find(a), eg.find(b));
    }

    #[test]
    fn union_merges_classes() {
        let (reg, v) = vector_registry();
        let mut eg = EGraph::new();
        let a = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x"));
        let b = eg.add_monomial(&reg, &vector_monomial(v, &reg, "y"));
        eg.union(a, b);
        assert_eq!(eg.find(a), eg.find(b));
    }

    #[test]
    fn rebuild_propagates_congruence_through_sum_nodes() {
        // If a == b, then Sum([a, c]) and Sum([b, c]) must become the
        // same e-class after rebuild, even though they were added as
        // syntactically different nodes.
        let (reg, v) = vector_registry();
        let mut eg = EGraph::new();
        let a = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x"));
        let b = eg.add_monomial(&reg, &vector_monomial(v, &reg, "y"));
        let c = eg.add_monomial(&reg, &vector_monomial(v, &reg, "z"));

        let sum_ac = eg.add(ENode::Sum(smallvec![a, c]));
        let sum_bc = eg.add(ENode::Sum(smallvec![b, c]));
        assert_ne!(eg.find(sum_ac), eg.find(sum_bc));

        eg.union(a, b);
        eg.rebuild();
        assert_eq!(eg.find(sum_ac), eg.find(sum_bc));
    }

    #[test]
    fn sum_node_children_hashcons_regardless_of_order() {
        let (reg, v) = vector_registry();
        let mut eg = EGraph::new();
        let a = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x"));
        let b = eg.add_monomial(&reg, &vector_monomial(v, &reg, "y"));
        let sum_ab = eg.add(ENode::Sum(smallvec![a, b]));
        let sum_ba = eg.add(ENode::Sum(smallvec![b, a]));
        assert_eq!(eg.find(sum_ab), eg.find(sum_ba));
    }

    #[test]
    fn zero_is_the_empty_sum_and_is_unique() {
        let mut eg = EGraph::new();
        let z1 = eg.zero();
        let z2 = eg.add(ENode::Sum(SmallVec::new()));
        assert_eq!(eg.find(z1), eg.find(z2));
    }
}
