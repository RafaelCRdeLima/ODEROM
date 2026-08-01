//! Metric compatibility, `nabla_a g_bc = 0` -- the defining property of
//! the Levi-Civita connection.
//!
//! Declared, never inferred, for the same reason the first Bianchi
//! identity is (`bianchi.rs`): it is a property of a *choice of
//! connection*, not of the metric's shape. A metric carried by a
//! connection with torsion, or by any non-Levi-Civita connection, does
//! not satisfy it. Deducing it from "this head is the metric" would have
//! the engine assert a physical assumption on the user's behalf.
//!
//! Note what this is *not*: `oderom-cli`'s `--metric` (index
//! raising/lowering, `Monomial::eliminate_metric`) deliberately leaves a
//! *differentiated* metric alone, because `nabla_c g_ab` is a different
//! object from `g_ab` and eliminating it would be assuming exactly the
//! fact this module exists to let a user declare.

use crate::egraph::{EGraph, ENode};
use oderom_core::{HeadId, Monomial, Registry};

/// Unions with zero every e-class holding a monomial that has a
/// derivative of `metric_head` among its factors.
///
/// Any monomial *containing* `nabla...g` as a factor is zero, not just a
/// lone one: a product with a vanishing factor vanishes. That is why
/// this scans all factors rather than only the bare single-factor case,
/// and it is why the rule needs no Leibniz machinery to be useful --
/// `nabla_a g_bc R[d,e,f,g]` is zero for the same reason `nabla_a g_bc`
/// is.
///
/// An *undifferentiated* `g_bc` factor is untouched: the metric itself
/// is not zero. Only heads whose `derivative_count()` is at least one
/// and whose `base_head()` is `metric_head` qualify.
pub fn apply_metric_compatibility(egraph: &mut EGraph, registry: &Registry, metric_head: HeadId) {
    let vanishing: Vec<Monomial> = egraph
        .classes()
        .flat_map(|(_, nodes)| nodes.iter())
        .filter_map(|node| match node {
            ENode::Term(m) if has_differentiated_metric(m, registry, metric_head) => Some(m.clone()),
            _ => None,
        })
        .collect();

    for m in vanishing {
        let id = egraph.add_monomial(registry, &m);
        let zero = egraph.zero();
        egraph.union(id, zero);
    }
    egraph.rebuild();
}

fn has_differentiated_metric(m: &Monomial, registry: &Registry, metric_head: HeadId) -> bool {
    m.factors().iter().any(|f| {
        let head = registry.head(f.head);
        head.derivative_count() >= 1 && head.base_head() == metric_head
    })
}
