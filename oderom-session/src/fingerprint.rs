//! Content-hash fingerprint of one named `.od` declaration -- the single
//! piece DESIGN-UI-SESSION.md section 3 builds both entry obsolescence
//! (compare a name's fingerprint across two `Document`s) and the compute
//! cache (key stage results by the fingerprint of everything a query
//! used) on top of.

use oderom_cli::model::Model;
use rustc_hash::FxHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Hash of one declaration's own already-normalized semantic content
/// (never raw source text, never any internal id -- `Grid`/
/// `ComponentTensor::canonical_hash`, `Chart`'s derived `Hash`, all
/// order-independent of how the declaration was built). Two declarations
/// with the same fingerprint are the same value, whether or not they
/// share a name, which is a feature (DESIGN-UI-SESSION.md section 3):
/// the compute cache below can safely be shared between them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DefFingerprint(pub u64);

/// One fingerprint per name across `model.charts`/`metrics`/
/// `connections` -- one flat namespace, matching `used`'s own
/// `BTreeSet<String>` (`ExecutionContext::record_use` doesn't
/// distinguish which kind of declaration a name came from either, since
/// `.od`'s three maps are independently keyed and nothing today collides
/// two different kinds under the same name in practice).
pub fn compute_fingerprints(model: &Model) -> HashMap<String, DefFingerprint> {
    let mut out = HashMap::new();
    for (name, chart) in &model.charts {
        let mut hasher = FxHasher::default();
        chart.hash(&mut hasher);
        out.insert(name.clone(), DefFingerprint(hasher.finish()));
    }
    for (name, (_, _, tensor)) in &model.metrics {
        out.insert(name.clone(), DefFingerprint(tensor.canonical_hash()));
    }
    for (name, (_, gamma)) in &model.connections {
        out.insert(name.clone(), DefFingerprint(gamma.canonical_hash()));
    }
    out
}

/// A composite fingerprint over every name in `used` (`BTreeSet` already
/// iterates sorted, so this is order-independent by construction) --
/// the compute-cache key, never a single declaration's fingerprint alone
/// (DESIGN-UI-SESSION.md section 3, verification 2, point 1: a query's
/// result depends on everything it touched -- a chart as much as a
/// metric -- not just the metric).
pub fn composite_fingerprint(fingerprints: &HashMap<String, DefFingerprint>, used: &std::collections::BTreeSet<String>) -> DefFingerprint {
    let mut hasher = FxHasher::default();
    for name in used {
        name.hash(&mut hasher);
        if let Some(fp) = fingerprints.get(name) {
            fp.hash(&mut hasher);
        }
    }
    DefFingerprint(hasher.finish())
}
