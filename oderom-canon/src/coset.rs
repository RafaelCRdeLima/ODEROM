//! Search over the group enumerated by [`oderom_core::Bsgs::for_each_element`]
//! for the lexicographically minimal word.
//!
//! Reference: Butler, "Fundamental Algorithms for Permutation Groups",
//! LNCS 559 (1991); R. Portugal, "An algorithm to simplify tensor
//! expressions", J. Phys. A 32 (1999) 7779, sec. 3.
//!
//! PERF: `for_each_element` does not prune. A representative chosen at an
//! early stabilizer-chain level already fixes the word entry at every
//! canonical position whose *both* endpoints (for a dummy pair) lie in
//! blocks resolved so far; comparing that partial prefix against the best
//! complete word seen so far and cutting the branch when it is already
//! worse is the classical Butler-Portugal pruning step ("podar com o
//! gerador forte"). Omitted here because full enumeration already meets
//! the Marco 1 performance budget at the group orders `oderom-canon`
//! deals with (a few times 10^5); revisit if a later marco pushes tensor
//! degree higher.

use crate::word::{compute_word, Labeling, Word};
use oderom_core::{Bsgs, SignedPerm};
use rustc_hash::FxHashSet;

pub(crate) struct SearchResult {
    pub representative: SignedPerm,
    pub is_zero: bool,
}

pub(crate) fn search_minimal(bsgs: &Bsgs, labeling: &Labeling) -> SearchResult {
    if bsgs.global_negation {
        // (identity, -1) belongs to the group: every reachable word is
        // reachable with both signs, so the object is identically zero
        // regardless of which word is minimal.
        return SearchResult { representative: SignedPerm::identity(bsgs.degree), is_zero: true };
    }

    let mut best_word: Option<Word> = None;
    let mut best_rep: Option<SignedPerm> = None;
    let mut signs_at_best: FxHashSet<i8> = FxHashSet::default();

    bsgs.for_each_element(|g| {
        let w = compute_word(g, labeling);
        match &best_word {
            None => {
                best_word = Some(w);
                best_rep = Some(g.clone());
                signs_at_best.insert(g.sign);
            }
            Some(best) if w < *best => {
                best_word = Some(w);
                best_rep = Some(g.clone());
                signs_at_best.clear();
                signs_at_best.insert(g.sign);
            }
            Some(best) if w == *best => {
                signs_at_best.insert(g.sign);
            }
            _ => {}
        }
    });

    SearchResult {
        representative: best_rep.expect("group always contains at least the identity"),
        is_zero: signs_at_best.len() > 1,
    }
}
