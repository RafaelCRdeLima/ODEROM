//! The compute cache: memoizes the expensive intermediate `Grid`/`Expr`
//! stages so two entries against the same still-current metric never
//! redo Christoffel (or Riemann-mixed, or Ricci) from scratch --
//! DESIGN-UI-SESSION.md section 3, all three verification-2 corrections:
//! keyed by composite fingerprint (covers every name a query used, not
//! just the metric), stores the semantic object (never a rendered
//! string), bounded by a hand-rolled LRU (no dependency -- same "build
//! it, don't pull it in" reasoning as this project's e-graph/
//! Schreier-Sims).

use crate::fingerprint::DefFingerprint;
use oderom_components::Grid;
use oderom_expr::Expr;
use std::collections::HashMap;
use std::hash::Hash;

/// Eviction is a linear scan for the oldest `last_used`, not the classic
/// O(1) doubly-linked-list scheme -- a session realistically holds a
/// cap in the tens-to-low-hundreds of entries (a handful of distinct
/// metrics x a handful of stages), where O(n) eviction costs nothing
/// measurable and the simpler implementation has less room for a bug to
/// hide in.
pub struct LruCache<K, V> {
    entries: HashMap<K, (V, u64)>,
    clock: u64,
    capacity: usize,
}

impl<K: Eq + Hash + Clone, V> LruCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        LruCache { entries: HashMap::new(), clock: 0, capacity: capacity.max(1) }
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.clock += 1;
        let clock = self.clock;
        if let Some(entry) = self.entries.get_mut(key) {
            entry.1 = clock;
        }
        self.entries.get(key).map(|(v, _)| v)
    }

    /// Computes and caches the value for `key` on a miss, returns the
    /// (now cached) value either way -- the single entry point callers
    /// use, so "check, then maybe compute, then insert" is never spread
    /// across two call sites that could disagree.
    pub fn get_or_try_insert_with<E>(&mut self, key: K, compute: impl FnOnce() -> Result<V, E>) -> Result<&V, E> {
        if self.get(&key).is_none() {
            let value = compute()?;
            self.insert(key.clone(), value);
        }
        Ok(&self.entries[&key].0)
    }

    fn insert(&mut self, key: K, value: V) {
        self.clock += 1;
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            if let Some(oldest) = self.entries.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| k.clone()) {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, (value, self.clock));
    }

    /// Test-only: whether a stage was actually recomputed vs. reused is
    /// exactly what the cache exists to make invisible to production
    /// code (a caller never needs to ask "was that a hit"), but it's the
    /// one thing a test verifying the cache *works* needs to observe.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Per-stage capacity: a small, deliberately unoptimized default (see
/// module docs -- revisit with real memory numbers once the app has
/// actual usage to measure, not guessed now).
const DEFAULT_STAGE_CAPACITY: usize = 32;

/// One `LruCache` per curvature stage, each keyed by the composite
/// fingerprint of everything the query that produced it used
/// (`fingerprint::composite_fingerprint`). Every stage is a pure
/// function of that fingerprint, so a hit here is always correct: the
/// same fingerprint can only arise from the same chart/metric content.
///
/// No `christoffel` entry, deliberately: `oderom_cli::commands::
/// resolve_gamma_source` (reused as-is by `run.rs`, not reimplemented,
/// so name resolution stays in exactly one place) always computes it
/// eagerly as part of resolving which metric/connection a query uses --
/// caching it here could never produce a hit under that design. Measured
/// (`oderom-components/tests/diagnostic_cancel_latency.rs`): Christoffel
/// itself is the cheap stage (~50-65ms on Reissner-Nordstrom) next to
/// Riemann-mixed (~500ms), so this costs little -- and it is registered
/// here, not silently accepted, precisely so it doesn't get
/// "rediscovered" as a surprise. No `riemann_contravariant` entry
/// either: `kretschmann` is cached as one opaque stage (calling
/// `curvature::kretschmann`, which does its own four `raise_index`
/// calls internally), matching how the plain library function already
/// works, so there is nothing separate to key.
pub struct ComputeCache {
    pub riemann_mixed: LruCache<DefFingerprint, Grid>,
    pub riemann_cov: LruCache<DefFingerprint, Grid>,
    pub ricci_tensor: LruCache<DefFingerprint, Grid>,
    pub ricci_scalar: LruCache<DefFingerprint, Expr>,
    pub kretschmann: LruCache<DefFingerprint, Expr>,
    /// Marco 6 step 6: cached for the same reason `riemann_cov`/
    /// `kretschmann` are (a genuine per-component, `n^4`-shaped stage,
    /// not a cheap combination the way `einstein_tensor`'s own
    /// uncached subtraction is) -- `weyl` and `weylsquare` both key off
    /// this one slot, `weylsquare` never redoing the tensor itself.
    pub weyl_tensor: LruCache<DefFingerprint, Grid>,
    /// Same reasoning as `kretschmann`'s own slot: `weylsquare` is a
    /// full `n^4` raise-then-contract on top of `weyl_tensor`, cheap to
    /// look up again but not cheap to redo.
    pub weyl_squared: LruCache<DefFingerprint, Expr>,
    // No `ricci_squared`/`gauss_bonnet` entries: both stay uncached,
    // matching `ricci_scalar`'s own precedent (`ricci_squared`: an
    // `n^2` contraction on top of the already-cached `ricci_tensor`,
    // cheap to redo) and `einstein_tensor`'s (`gauss_bonnet`: an O(1)
    // recombination of three already-cached/computed scalars).
}

impl Default for ComputeCache {
    fn default() -> Self {
        ComputeCache {
            riemann_mixed: LruCache::new(DEFAULT_STAGE_CAPACITY),
            riemann_cov: LruCache::new(DEFAULT_STAGE_CAPACITY),
            ricci_tensor: LruCache::new(DEFAULT_STAGE_CAPACITY),
            ricci_scalar: LruCache::new(DEFAULT_STAGE_CAPACITY),
            kretschmann: LruCache::new(DEFAULT_STAGE_CAPACITY),
            weyl_tensor: LruCache::new(DEFAULT_STAGE_CAPACITY),
            weyl_squared: LruCache::new(DEFAULT_STAGE_CAPACITY),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_or_try_insert_with_only_computes_once() {
        let mut cache: LruCache<u64, i32> = LruCache::new(4);
        let mut calls = 0;
        for _ in 0..3 {
            cache
                .get_or_try_insert_with(1, || {
                    calls += 1;
                    Ok::<_, ()>(42)
                })
                .unwrap();
        }
        assert_eq!(calls, 1);
    }

    #[test]
    fn capacity_evicts_the_least_recently_used() {
        let mut cache: LruCache<u64, i32> = LruCache::new(2);
        cache.get_or_try_insert_with(1, || Ok::<_, ()>(1)).unwrap();
        cache.get_or_try_insert_with(2, || Ok::<_, ()>(2)).unwrap();
        cache.get(&1); // touch 1 so 2 becomes the least recently used
        cache.get_or_try_insert_with(3, || Ok::<_, ()>(3)).unwrap(); // evicts 2, not 1
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&1), Some(&1));
        assert_eq!(cache.get(&3), Some(&3));
        let mut recomputed = false;
        cache
            .get_or_try_insert_with(2, || {
                recomputed = true;
                Ok::<_, ()>(2)
            })
            .unwrap();
        assert!(recomputed, "2 should have been evicted and need recomputing");
    }
}
