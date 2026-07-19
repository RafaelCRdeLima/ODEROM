//! Cost-based extraction: picking a concrete [`oderom_core::Polynomial`]
//! out of an e-class -- the minimal-cost one, where cost is just term
//! count (a `Term` costs 1, a `Sum` costs the sum of its children's
//! costs, so the empty sum -- zero -- always wins whenever an e-class
//! has been unioned with it).
//!
//! Extraction is bottom-up dynamic programming: repeatedly relax "the
//! best known cost for e-class X" using every node in X, over every
//! e-class, until nothing improves. A single top-down pass over a
//! DAG-shaped expression tree would do, but an e-graph's e-classes don't
//! have a fixed traversal order the way a plain tree's subexpressions
//! do (an e-class can be defined in terms of another that only becomes
//! cheap after *its own* best node is known), so the fixed point is
//! computed for the whole graph at once rather than assumed to fall out
//! of any single pass order. Optimal (not just locally-minimal) e-graph
//! extraction is NP-hard in general; this greedy relaxation is the
//! standard practical approach (`egg` uses the same idea), and at the
//! e-class counts this project's e-graphs reach, exhaustive optimality
//! isn't worth the complexity.

use crate::egraph::{EClassId, EGraph, ENode};
use oderom_core::Polynomial;
use rustc_hash::FxHashMap;

struct Best {
    cost: usize,
    node: ENode,
}

/// The lowest-cost [`Polynomial`] equivalent to `root`'s e-class.
pub fn extract(egraph: &mut EGraph, root: EClassId) -> Polynomial {
    let best = compute_best(egraph);
    let root = egraph.find(root);
    reconstruct(egraph, &best, root)
}

fn compute_best(egraph: &mut EGraph) -> FxHashMap<EClassId, Best> {
    let mut best: FxHashMap<EClassId, Best> = FxHashMap::default();
    let mut changed = true;
    while changed {
        changed = false;
        let snapshot: Vec<(EClassId, Vec<ENode>)> =
            egraph.classes().map(|(id, nodes)| (id, nodes.to_vec())).collect();
        for (class_id, nodes) in snapshot {
            for node in nodes {
                let Some(cost) = node_cost(&node, &best) else { continue };
                let is_better = match best.get(&class_id) {
                    None => true,
                    Some(b) => cost < b.cost,
                };
                if is_better {
                    best.insert(class_id, Best { cost, node });
                    changed = true;
                }
            }
        }
    }
    best
}

fn node_cost(node: &ENode, best: &FxHashMap<EClassId, Best>) -> Option<usize> {
    match node {
        ENode::Term(_) => Some(1),
        ENode::Sum(children) => {
            let mut total = 0;
            for c in children {
                total += best.get(c)?.cost;
            }
            Some(total)
        }
    }
}

fn reconstruct(egraph: &mut EGraph, best: &FxHashMap<EClassId, Best>, id: EClassId) -> Polynomial {
    let id = egraph.find(id);
    match best.get(&id) {
        // Only reachable for an e-class no path of additions ever
        // bottoms out at a Term -- doesn't arise from this crate's own
        // construction (every Sum's leaves are always Terms or the
        // empty sum), but degrading to zero is a safe default rather
        // than panicking on a graph built some other way.
        None => Polynomial { terms: vec![] },
        Some(b) => match &b.node {
            ENode::Term(m) => Polynomial { terms: vec![m.clone()] },
            ENode::Sum(children) => {
                let children = children.clone();
                let mut terms = Vec::new();
                for c in children {
                    terms.extend(reconstruct(egraph, best, c).terms);
                }
                Polynomial { terms }
            }
        },
    }
}
