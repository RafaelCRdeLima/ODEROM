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
use oderom_core::{Monomial, Registry, Scalar};
use std::hash::Hash;
use rustc_hash::FxHashMap;
use std::collections::BTreeMap;

/// An index into an [`EGraph`]'s e-classes. Not stable across `union`:
/// always pass it through [`EGraph::find`] (or a method that already
/// does, like [`EGraph::add`] and [`EGraph::union`]) before comparing two
/// ids for equivalence.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EClassId(u32);

/// A sum of e-classes, each weighted by an exact rational.
///
/// Three properties fall out of the representation rather than out of a
/// pass over it:
///
/// 1. **Collection is construction.** Inserting a term that is already
///    present adds the coefficients, so like terms can never coexist
///    uncollected. An earlier attempt to collect *before* the e-graph
///    failed, and "it ran at the wrong layer" was only the symptom --
///    the cause is that the old representation had no collection key at
///    all, because the coefficient lived inside `Term` and so
///    `3*R[a,b,c,d]` and `R[a,b,c,d]` were different e-classes.
/// 2. **AC normal form for free.** Keying by `EClassId` gives
///    associativity, commutativity and a deterministic output order
///    with no extra pass.
/// 3. **Zero terms disappear.** A coefficient that reaches zero removes
///    its entry rather than being kept as a zero value, so the empty
///    map is the one representation of zero.
///
/// The flattening guarantee is **construction-time, not closure**:
/// nested sums are spliced when built, but an e-class can become equal
/// to a sum *later* via `union`, and nothing re-flattens it. Doing that
/// by congruence is R2/R3 (DESIGN-TENSOR-ALGEBRA.md), not this round.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SumNode {
    terms: BTreeMap<EClassId, Scalar>,
}

impl SumNode {
    pub fn new() -> Self {
        SumNode::default()
    }

    /// Adds `coeff * term`, collecting into any entry already present
    /// and removing the entry entirely if the total reaches zero.
    pub fn insert(&mut self, term: EClassId, coeff: Scalar) {
        if coeff.is_zero() {
            return;
        }
        let total = self.terms.get(&term).copied().unwrap_or(Scalar::ZERO) + coeff;
        if total.is_zero() {
            self.terms.remove(&term);
        } else {
            self.terms.insert(term, total);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (EClassId, Scalar)> + '_ {
        self.terms.iter().map(|(&id, &c)| (id, c))
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn len(&self) -> usize {
        self.terms.len()
    }
}

impl FromIterator<(EClassId, Scalar)> for SumNode {
    fn from_iter<I: IntoIterator<Item = (EClassId, Scalar)>>(iter: I) -> Self {
        let mut s = SumNode::new();
        for (id, c) in iter {
            s.insert(id, c);
        }
        s
    }
}

/// `Hash` by hand because `BTreeMap` hashes in key order, which is what
/// hash-consing needs, but `Scalar` is only `Hash` as a value -- the
/// derive would be correct and this is written out only so that the
/// ordering dependence is visible at the point it matters.
impl std::hash::Hash for SumNode {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for (id, c) in &self.terms {
            id.hash(state);
            c.hash(state);
        }
    }
}

/// One way to build a value: either a single already-canonical monomial
/// **with coefficient 1**, or a coefficient-carrying sum of e-classes.
/// `Sum(empty)` is the canonical representation of zero.
///
/// The coefficient-1 rule on `Term` is the whole point of R1b: it is
/// what gives two scalings of the same monomial a common key.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ENode {
    Term(Monomial),
    Sum(SumNode),
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
    /// Returns the term's e-class together with its coefficient.
    ///
    /// R1a: the coefficient is *also* returned, not yet moved out of
    /// `ENode::Term` -- `Term` still carries it, so behaviour is
    /// unchanged and every caller takes `.0`. Splitting the signature
    /// change away from the representation change keeps R1b to the
    /// semantics alone.
    ///
    /// A canonicalization that proves the monomial equal to its own
    /// negative contributes the zero e-class and a zero coefficient.
    pub fn add_monomial(&mut self, registry: &Registry, m: &Monomial) -> (EClassId, Scalar) {
        match canonicalize(m, registry).expect("m was already validated by Monomial::try_new") {
            oderom_canon::CanonResult::Zero => (self.zero(), Scalar::ZERO),
            oderom_canon::CanonResult::Value(c) => {
                // R1b: the coefficient leaves the term. The e-class key
                // is the coefficient-1 monomial, so every scaling of the
                // same shape lands in one e-class and a `SumNode` can
                // collect them.
                let coeff = c.monomial.coeff();
                // A zero coefficient contributes no term at all, the
                // same as `CanonResult::Zero` -- and it is also the one
                // case with no reciprocal to normalise by.
                let Some(inv) = coeff.recip() else { return (self.zero(), Scalar::ZERO) };
                let key = c.monomial.scaled(inv);
                (self.add(ENode::Term(key)), coeff)
            }
        }
    }

    /// The e-class for the empty sum, i.e. zero.
    pub fn zero(&mut self) -> EClassId {
        self.add(ENode::Sum(SumNode::new()))
    }

    /// Builds a sum from `(e-class, coefficient)` pairs, splicing any
    /// pair whose e-class is *already* a sum so that nested sums do not
    /// survive construction. Coefficients multiply through the splice.
    ///
    /// Construction-time only, per `SumNode`'s doc comment: an e-class
    /// that becomes a sum later via `union` is not re-spliced here.
    pub fn add_sum(&mut self, pairs: impl IntoIterator<Item = (EClassId, Scalar)>) -> EClassId {
        let mut acc = SumNode::new();
        for (id, coeff) in pairs {
            let root = self.find(id);
            let inner: Option<SumNode> = self
                .classes
                .get(&root)
                .and_then(|nodes| nodes.iter().find_map(|n| match n {
                    ENode::Sum(s) => Some(s.clone()),
                    _ => None,
                }));
            match inner {
                Some(s) => {
                    for (child, c) in s.iter() {
                        acc.insert(child, c * coeff);
                    }
                }
                None => acc.insert(root, coeff),
            }
        }
        self.add(ENode::Sum(acc))
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
            // `find` can map two distinct keys onto the same
            // representative, so this re-inserts rather than rebuilding
            // the map directly: the merge has to add coefficients, and
            // can cancel a pair to nothing.
            ENode::Sum(s) => {
                let pairs: Vec<(EClassId, Scalar)> = s.iter().collect();
                let mut canon = SumNode::new();
                for (c, k) in pairs {
                    let r = self.find(c);
                    canon.insert(r, k);
                }
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
    use smallvec::{smallvec, SmallVec};

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
        let a1 = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x")).0;
        let a2 = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x")).0;
        assert_eq!(eg.find(a1), eg.find(a2));
    }

    #[test]
    fn distinct_monomials_start_in_distinct_classes() {
        let (reg, v) = vector_registry();
        let mut eg = EGraph::new();
        let a = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x")).0;
        let b = eg.add_monomial(&reg, &vector_monomial(v, &reg, "y")).0;
        assert_ne!(eg.find(a), eg.find(b));
    }

    #[test]
    fn union_merges_classes() {
        let (reg, v) = vector_registry();
        let mut eg = EGraph::new();
        let a = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x")).0;
        let b = eg.add_monomial(&reg, &vector_monomial(v, &reg, "y")).0;
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
        let a = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x")).0;
        let b = eg.add_monomial(&reg, &vector_monomial(v, &reg, "y")).0;
        let c = eg.add_monomial(&reg, &vector_monomial(v, &reg, "z")).0;

        let sum_ac = eg.add_sum([(a, Scalar::ONE), (c, Scalar::ONE)]);
        let sum_bc = eg.add_sum([(b, Scalar::ONE), (c, Scalar::ONE)]);
        assert_ne!(eg.find(sum_ac), eg.find(sum_bc));

        eg.union(a, b);
        eg.rebuild();
        assert_eq!(eg.find(sum_ac), eg.find(sum_bc));
    }

    #[test]
    fn sum_node_children_hashcons_regardless_of_order() {
        let (reg, v) = vector_registry();
        let mut eg = EGraph::new();
        let a = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x")).0;
        let b = eg.add_monomial(&reg, &vector_monomial(v, &reg, "y")).0;
        let sum_ab = eg.add_sum([(a, Scalar::ONE), (b, Scalar::ONE)]);
        let sum_ba = eg.add_sum([(b, Scalar::ONE), (a, Scalar::ONE)]);
        assert_eq!(eg.find(sum_ab), eg.find(sum_ba));
    }

    /// `(T+S)+U` and `T+(S+U)` must build the same `SumNode`. The CLI's
    /// surface syntax cannot express a nested sum, so this is the only
    /// place the associativity half of "AC normal form for free" is
    /// checked.
    #[test]
    fn nested_sums_flatten_to_the_same_node_either_way() {
        let (reg, v) = vector_registry();
        let mut eg = EGraph::new();
        let a = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x")).0;
        let b = eg.add_monomial(&reg, &vector_monomial(v, &reg, "y")).0;
        let c = eg.add_monomial(&reg, &vector_monomial(v, &reg, "z")).0;

        let ab = eg.add_sum([(a, Scalar::ONE), (b, Scalar::ONE)]);
        let left = eg.add_sum([(ab, Scalar::ONE), (c, Scalar::ONE)]);

        let bc = eg.add_sum([(b, Scalar::ONE), (c, Scalar::ONE)]);
        let right = eg.add_sum([(a, Scalar::ONE), (bc, Scalar::ONE)]);

        assert_eq!(eg.find(left), eg.find(right));
    }

    /// Coefficients multiply through the splice, and a nested sum that
    /// cancels against an outer term leaves nothing behind.
    #[test]
    fn splicing_multiplies_coefficients_and_can_cancel_to_zero() {
        let (reg, v) = vector_registry();
        let mut eg = EGraph::new();
        let a = eg.add_monomial(&reg, &vector_monomial(v, &reg, "x")).0;

        // 2*(3*a) + (-6)*a  ==  0
        let inner = eg.add_sum([(a, Scalar::from_int(3))]);
        let outer = eg.add_sum([(inner, Scalar::from_int(2)), (a, Scalar::from_int(-6))]);
        let z = eg.zero();
        assert_eq!(eg.find(outer), eg.find(z));
    }

    #[test]
    fn zero_is_the_empty_sum_and_is_unique() {
        let mut eg = EGraph::new();
        let z1 = eg.zero();
        let z2 = eg.add(ENode::Sum(SumNode::new()));
        assert_eq!(eg.find(z1), eg.find(z2));
    }
}
