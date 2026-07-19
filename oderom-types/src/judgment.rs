//! The geometric type judgment `Gamma |- T : Section(E1^+- ⊗ .. ⊗ En^+- -> M, U)`.
//!
//! Marco 1 checks only what is syntactically decidable: that every
//! contraction pairs a slot of some bundle `E` with a slot of its dual
//! `E*` over the same manifold, and that a sum's terms all carry the same
//! free indices with the same bundle/variance. Domain obligations beyond
//! `Everywhere` are Marco 3's job.

use crate::domain::Domain;
use crate::error::{BundleDescription, TypeError};
use oderom_core::{AbstractIndex, ManifoldId, Monomial, Polynomial, Registry, SlotId, SlotSig};

/// The type of a well-formed expression: which manifold it lives over,
/// the bundle/variance of each of its free indices, and its domain of
/// validity.
#[derive(Clone, Debug)]
pub struct ExprType {
    pub manifold: ManifoldId,
    pub free_signature: Vec<(AbstractIndex, SlotSig)>,
    pub domain: Domain,
}

fn slot_sig(m: &Monomial, registry: &Registry, slot: SlotId) -> SlotSig {
    let head = registry.head(m.factors()[slot.factor as usize].head);
    head.slots[slot.slot as usize]
}

fn head_name(m: &Monomial, registry: &Registry, slot: SlotId) -> String {
    registry.head(m.factors()[slot.factor as usize].head).name.clone()
}

/// Type-checks a single monomial: every contraction must join a bundle
/// slot to its dual over the same manifold, and every factor of the
/// monomial must live over the same base manifold.
pub fn typecheck_monomial(
    term: usize,
    m: &Monomial,
    registry: &Registry,
) -> Result<ExprType, TypeError> {
    if m.factors().is_empty() {
        return Err(TypeError::EmptyMonomial);
    }

    for &(a, b) in m.contractions().pairs() {
        let sa = slot_sig(m, registry, a);
        let sb = slot_sig(m, registry, b);
        if sa.bundle != sb.bundle || sa.variance != sb.variance.dual() {
            return Err(TypeError::IncompatibleContraction {
                left_slot: a,
                left_head: head_name(m, registry, a),
                left_kind: BundleDescription::new(&registry.bundle(sa.bundle).name, sa.variance, sa),
                right_slot: b,
                right_head: head_name(m, registry, b),
                right_kind: BundleDescription::new(&registry.bundle(sb.bundle).name, sb.variance, sb),
            });
        }
    }

    let mut manifold: Option<ManifoldId> = None;
    for factor in m.factors() {
        for sig in &registry.head(factor.head).slots {
            let bm = registry.bundle(sig.bundle).base;
            match manifold {
                None => manifold = Some(bm),
                Some(existing) if existing == bm => {}
                Some(existing) => {
                    return Err(TypeError::ManifoldMismatch {
                        term,
                        expected: registry.manifold(existing).name.clone(),
                        found: registry.manifold(bm).name.clone(),
                    })
                }
            }
        }
    }

    let free_signature =
        m.free().iter().map(|(slot, label)| (label.clone(), slot_sig(m, registry, *slot))).collect();

    Ok(ExprType {
        manifold: manifold.expect("checked non-empty factors above"),
        free_signature,
        domain: Domain::Everywhere,
    })
}

/// Type-checks a sum: every term must type-check individually and all
/// terms must carry the same free indices, each with the same
/// bundle/variance.
pub fn typecheck_polynomial(p: &Polynomial, registry: &Registry) -> Result<ExprType, TypeError> {
    let mut terms = p.terms.iter().enumerate();
    let (_, first) = terms.next().ok_or(TypeError::EmptyPolynomial)?;
    let baseline = typecheck_monomial(0, first, registry)?;

    let sorted_signature = |sig: &[(AbstractIndex, SlotSig)]| -> Vec<(String, SlotSig)> {
        let mut v: Vec<(String, SlotSig)> =
            sig.iter().map(|(l, s)| (l.name().to_string(), *s)).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    let baseline_sorted = sorted_signature(&baseline.free_signature);

    for (i, term) in terms {
        let t = typecheck_monomial(i, term, registry)?;
        let sorted = sorted_signature(&t.free_signature);

        let expected_names: Vec<String> = baseline_sorted.iter().map(|(n, _)| n.clone()).collect();
        let found_names: Vec<String> = sorted.iter().map(|(n, _)| n.clone()).collect();
        if expected_names != found_names {
            return Err(TypeError::FreeIndexMismatch { term: i, expected: expected_names, found: found_names });
        }
        for ((label, expected_sig), (_, found_sig)) in baseline_sorted.iter().zip(sorted.iter()) {
            if expected_sig != found_sig {
                return Err(TypeError::FreeSlotSigMismatch { term: i, label: label.clone() });
            }
        }
    }

    Ok(baseline)
}
