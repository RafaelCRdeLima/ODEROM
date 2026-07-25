//! The `.od` definitions that parsed and built successfully, plus what
//! entry-obsolescence (DESIGN-UI-SESSION.md section 3) needs to compare
//! "before" against "after".

use crate::fingerprint::{compute_fingerprints, DefFingerprint};
use oderom_cli::model::Model;
use std::collections::{BTreeSet, HashMap};

/// Newtype so a generation counter and an entry id (both bare `u64`
/// underneath) can't be swapped by accident at a call site.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Generation(pub u64);

#[derive(Clone)]
pub struct Document {
    pub model: Model,
    pub generation: Generation,
    pub fingerprints: HashMap<String, DefFingerprint>,
}

impl Document {
    /// Parses `source` and builds a fresh `Document` -- does not touch
    /// any existing `Session` state; the caller decides whether/when to
    /// swap it in (`Session::evaluate_definitions`, the atomic part).
    pub fn parse(source: &str, generation: Generation) -> Result<Self, oderom_cli::CliError> {
        let model = oderom_cli::parser::parse_model(source)?;
        let fingerprints = compute_fingerprints(&model);
        Ok(Document { model, generation, fingerprints })
    }
}

/// Names whose fingerprint changed between `old` and `new`, or that
/// existed in `old` and no longer do -- never names that are only *new*
/// in `new` (nothing could have depended on a name that didn't exist
/// yet). This is the set `Session::evaluate_definitions` checks every
/// `Entry`'s `used` against to decide which ones just went stale.
pub fn changed_or_removed_names(old: &Document, new: &Document) -> BTreeSet<String> {
    let mut changed = BTreeSet::new();
    for (name, old_fp) in &old.fingerprints {
        match new.fingerprints.get(name) {
            Some(new_fp) if new_fp == old_fp => {}
            _ => {
                changed.insert(name.clone());
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHWARZSCHILD: &str = "
manifold M dim 4
bundle TM on M dim 4
chart schw on M coords (t, r, theta, phi)
metric g on schw bundle TM {
  [t,t] = -(1 - 2*M/r),
  [r,r] = 1/(1 - 2*M/r),
  [theta,theta] = r^2,
  [phi,phi] = r^2 * sin(theta)^2
}
";

    #[test]
    fn reparsing_identical_source_produces_identical_fingerprints() {
        let a = Document::parse(SCHWARZSCHILD, Generation(0)).unwrap();
        let b = Document::parse(SCHWARZSCHILD, Generation(1)).unwrap();
        assert_eq!(a.fingerprints.get("g"), b.fingerprints.get("g"));
        assert_eq!(a.fingerprints.get("schw"), b.fingerprints.get("schw"));
        assert!(changed_or_removed_names(&a, &b).is_empty());
    }

    #[test]
    fn changing_the_metric_changes_only_its_own_fingerprint() {
        let a = Document::parse(SCHWARZSCHILD, Generation(0)).unwrap();
        let edited = SCHWARZSCHILD.replace("2*M/r", "3*M/r");
        let b = Document::parse(&edited, Generation(1)).unwrap();
        let changed = changed_or_removed_names(&a, &b);
        assert!(changed.contains("g"), "{changed:?}");
        assert!(!changed.contains("schw"), "{changed:?}");
    }

    #[test]
    fn changing_only_the_chart_changes_only_its_own_fingerprint() {
        let a = Document::parse(SCHWARZSCHILD, Generation(0)).unwrap();
        // Renaming "phi" everywhere it's a coordinate NAME (both the
        // chart's own list and the metric's `[phi,phi]` bracket
        // reference, which resolves by name, not position) changes the
        // chart's declared coordinate list without changing the
        // metric's *positional* content (`ComponentTensor` is keyed by
        // index position, not coordinate spelling) -- the one edit that
        // isolates "only the chart changed" from this grammar, since
        // `chart` has no other varying field a `.od` file can declare.
        let edited = SCHWARZSCHILD.replace("phi", "psi");
        let b = Document::parse(&edited, Generation(1)).unwrap();
        let changed = changed_or_removed_names(&a, &b);
        assert!(changed.contains("schw"), "{changed:?}");
        // `g`'s own fingerprint (a hash of its declared component
        // *values*, computed independently of which chart it's declared
        // against) is genuinely unaffected at THIS layer -- confirming
        // that is exactly why an entry's `used` set must include the
        // chart name too (recorded separately, at resolution time, see
        // `ExecutionContext::record_use` in `oderom-cli/src/commands.rs`)
        // for editing only the chart to correctly go on to invalidate an
        // entry that used `g`. That end-to-end behavior is covered at
        // the `Session`/`Entry` level, not provable from fingerprints
        // alone -- see `session.rs`'s tests.
        assert!(!changed.contains("g"), "{changed:?}");
        assert_eq!(a.fingerprints.get("g"), b.fingerprints.get("g"));
    }
}
