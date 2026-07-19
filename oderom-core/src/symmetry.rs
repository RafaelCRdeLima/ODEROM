//! Base and strong generating set (BSGS) for a signed permutation group,
//! built by (deterministic) Schreier-Sims.
//!
//! Reference: J. J. Cannon & D. F. Holt's exposition of Schreier-Sims, and
//! Butler, "Fundamental Algorithms for Permutation Groups", LNCS 559
//! (1991), sec. 4. The construction below is the textbook
//! orbit/transversal and Schreier-generator recursion; it is not the
//! randomized (Monte Carlo) variant, which is unnecessary at the group
//! orders this project deals with (tensor symmetry groups of order at
//! most a few hundred).
//!
//! # Sign bookkeeping
//!
//! Generators here are [`SignedPerm`]: a permutation of slot positions plus
//! the factor `+/-1` a tensor component picks up under that reordering. The
//! projection `pi: G -> S_n` forgetting the sign is a group homomorphism
//! (composition multiplies signs, matching composition of the point
//! permutations), so `ker(pi)` is a normal subgroup of order 1 or 2 -- it
//! can only be `{(id,+1)}` or `{(id,+1),(id,-1)}`. The latter case means the
//! group forces some rearrangement of the tensor's own slots that changes
//! nothing structurally (net permutation = identity) yet flips its sign:
//! the tensor equals its own negative, i.e. it is identically zero. This is
//! recorded as [`Bsgs::global_negation`] rather than folded into the
//! orbit/transversal machinery, because `(id,-1)` fixes every point and so
//! never contributes to picking a base point.

use crate::perm::{Perm, SignedPerm};
use rustc_hash::{FxHashMap, FxHashSet};

/// One level of the stabilizer chain: a base point together with the
/// Schreier transversal of its orbit under the *current* level's generating
/// set (i.e. under the stabilizer of all earlier base points).
#[derive(Clone, Debug)]
pub struct SchreierLevel {
    pub base_point: u16,
    /// orbit point -> coset representative sending `base_point` to it.
    pub transversal: FxHashMap<u16, SignedPerm>,
}

/// A base and strong generating set for a signed permutation group,
/// computed once (at `TensorHead` declaration time) and memoized.
#[derive(Clone, Debug)]
pub struct Bsgs {
    pub degree: usize,
    pub levels: Vec<SchreierLevel>,
    pub strong_generators: Vec<SignedPerm>,
    /// Whether `(identity permutation, sign -1)` belongs to the group.
    pub global_negation: bool,
}

impl Bsgs {
    /// The BSGS of the trivial (order-1) group on `degree` points.
    pub fn trivial(degree: usize) -> Self {
        Bsgs { degree, levels: Vec::new(), strong_generators: Vec::new(), global_negation: false }
    }

    /// Runs Schreier-Sims on `generators` (a generating set for the group;
    /// duplicates and the identity are fine).
    pub fn from_generators(degree: usize, generators: &[SignedPerm]) -> Self {
        let mut levels: Vec<SchreierLevel> = Vec::new();
        let mut all_strong_gens: Vec<SignedPerm> = Vec::new();
        let mut global_negation = false;
        let mut current: Vec<SignedPerm> = generators.to_vec();

        loop {
            let mut movers: Vec<SignedPerm> = Vec::new();
            for g in &current {
                if g.perm.is_identity() {
                    if g.sign == -1 {
                        global_negation = true;
                    }
                } else {
                    movers.push(g.clone());
                }
            }
            if movers.is_empty() {
                break;
            }

            let base_point = (0..degree as u16)
                .find(|&p| movers.iter().any(|g| g.perm.image(p) != p))
                .expect("movers is nonempty, so some generator moves some point");

            // Orbit of base_point under `movers`, with Schreier transversal.
            let mut transversal: FxHashMap<u16, SignedPerm> = FxHashMap::default();
            transversal.insert(base_point, SignedPerm::identity(degree));
            let mut queue = vec![base_point];
            let mut qi = 0;
            while qi < queue.len() {
                let delta = queue[qi];
                qi += 1;
                let u_delta = transversal[&delta].clone();
                for s in &movers {
                    let delta2 = s.perm.image(delta);
                    if let std::collections::hash_map::Entry::Vacant(e) = transversal.entry(delta2) {
                        e.insert(u_delta.then(s));
                        queue.push(delta2);
                    }
                }
            }

            // Schreier generators for the stabilizer of base_point (Schreier's lemma).
            let mut next_gens: Vec<SignedPerm> = Vec::new();
            let mut seen: FxHashSet<SignedPerm> = FxHashSet::default();
            for &delta in &queue {
                let u_delta = &transversal[&delta];
                for s in &movers {
                    let delta2 = s.perm.image(delta);
                    let u_delta2 = &transversal[&delta2];
                    let gamma = u_delta.then(s).then(&u_delta2.inverse());
                    if !gamma.is_identity() && seen.insert(gamma.clone()) {
                        next_gens.push(gamma);
                    }
                }
            }

            all_strong_gens.extend(movers.iter().cloned());
            levels.push(SchreierLevel { base_point, transversal });
            current = next_gens;
        }

        Bsgs { degree, levels, strong_generators: all_strong_gens, global_negation }
    }

    /// `|G|`, the product of orbit sizes at each level, doubled if
    /// `(id,-1)` is in the group.
    pub fn order(&self) -> u128 {
        let base: u128 = self.levels.iter().map(|l| l.transversal.len() as u128).product();
        if self.global_negation {
            base * 2
        } else {
            base
        }
    }

    /// Strips `g` through the stabilizer chain, returning the residue.
    /// `g` belongs to the group iff the residue's permutation is the
    /// identity (and, if the residue's sign is -1, only if the group
    /// contains the global negation).
    pub fn strip(&self, g: &SignedPerm) -> SignedPerm {
        let mut h = g.clone();
        for level in &self.levels {
            let img = h.perm.image(level.base_point);
            match level.transversal.get(&img) {
                Some(rep) => h = h.then(&rep.inverse()),
                None => return h,
            }
        }
        h
    }

    /// Whether `g` belongs to the group.
    pub fn contains(&self, g: &SignedPerm) -> bool {
        let h = self.strip(g);
        h.perm.is_identity() && (h.sign == 1 || self.global_negation)
    }

    /// Calls `visit` once for every element of the group, in an
    /// unspecified but complete, non-repeating order.
    ///
    /// Every element factors uniquely as a product of one transversal
    /// representative per stabilizer-chain level,
    /// `g = t_{L-1}.then(..).then(t_0)` (deepest level applied first,
    /// level 0 last -- the reverse of the order [`Bsgs::strip`] peels
    /// them off in; get this backwards and elements silently go missing
    /// from the enumeration without any error, which is exactly the bug
    /// this method's first draft, living in `oderom-canon`, had). If
    /// [`Bsgs::global_negation`] holds, every element enumerated this way
    /// also occurs with the opposite sign, so each is additionally
    /// visited with its sign flipped.
    pub fn for_each_element(&self, mut visit: impl FnMut(&SignedPerm)) {
        fn recurse(levels: &[SchreierLevel], acc: SignedPerm, visit: &mut impl FnMut(&SignedPerm)) {
            match levels.split_first() {
                None => visit(&acc),
                Some((level, rest)) => {
                    for rep in level.transversal.values() {
                        recurse(rest, rep.then(&acc), visit);
                    }
                }
            }
        }
        recurse(&self.levels, SignedPerm::identity(self.degree), &mut visit);
        if self.global_negation {
            recurse(&self.levels, SignedPerm::identity(self.degree), &mut |g| {
                visit(&SignedPerm { perm: g.perm.clone(), sign: -g.sign });
            });
        }
    }
}

/// Convenience: the totally antisymmetric generator set for an `n`-index
/// head (Levi-Civita), namely adjacent transpositions each carrying sign
/// -1. Exposed here because both the prelude parser and unit tests need it.
pub fn totally_antisymmetric_generators(n: usize) -> Vec<SignedPerm> {
    (0..n.saturating_sub(1))
        .map(|i| SignedPerm::new(Perm::transposition(n, i as u16, i as u16 + 1), -1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen(n: usize, a: u16, b: u16, sign: i8) -> SignedPerm {
        SignedPerm::new(Perm::transposition(n, a, b), sign)
    }

    #[test]
    fn trivial_group_has_order_one() {
        let bsgs = Bsgs::from_generators(4, &[]);
        assert_eq!(bsgs.order(), 1);
        assert!(bsgs.contains(&SignedPerm::identity(4)));
    }

    #[test]
    fn metric_symmetry_order_two() {
        // g[a,b] = g[b,a]: single symmetric generator on 2 slots.
        let bsgs = Bsgs::from_generators(2, &[gen(2, 0, 1, 1)]);
        assert_eq!(bsgs.order(), 2);
        assert!(bsgs.contains(&gen(2, 0, 1, 1)));
        assert!(!bsgs.global_negation);
    }

    #[test]
    fn levi_civita_3_has_order_six_all_signs_consistent_with_parity() {
        let gens = totally_antisymmetric_generators(3);
        let bsgs = Bsgs::from_generators(3, &gens);
        assert_eq!(bsgs.order(), 6);
        assert!(!bsgs.global_negation);
        // Every element's sign must equal the parity of its permutation,
        // since this is the alternating (sign) representation of S_3.
        for perm_images in permutations_of(3) {
            let p = Perm::try_from_images(&perm_images).unwrap();
            let expected_sign = p.parity();
            assert!(bsgs.contains(&SignedPerm::new(p.clone(), expected_sign)), "{p:?}");
            assert!(!bsgs.contains(&SignedPerm::new(p, -expected_sign)));
        }
    }

    #[test]
    fn riemann_symmetry_order_eight() {
        // <(0 1)-, (2 3)-, (0 2)(1 3)+>
        let pair_swap_02_13 = Perm::try_from_images(&[2, 3, 0, 1]).unwrap();
        let gens = vec![
            gen(4, 0, 1, -1),
            gen(4, 2, 3, -1),
            SignedPerm::new(pair_swap_02_13, 1),
        ];
        let bsgs = Bsgs::from_generators(4, &gens);
        assert_eq!(bsgs.order(), 8);
        assert!(!bsgs.global_negation);
        // R_{abcd} = R_{bacd} * (-1)
        assert!(bsgs.contains(&gen(4, 0, 1, -1)));
        // R_{abcd} = R_{cdab} * (+1)
        let swap_pairs = Perm::try_from_images(&[2, 3, 0, 1]).unwrap();
        assert!(bsgs.contains(&SignedPerm::new(swap_pairs, 1)));
    }

    #[test]
    fn inconsistent_symmetry_forces_global_negation() {
        // Declaring the same transposition both symmetric and antisymmetric
        // forces the object to equal its own negative.
        let bsgs = Bsgs::from_generators(2, &[gen(2, 0, 1, 1), gen(2, 0, 1, -1)]);
        assert!(bsgs.global_negation);
        assert_eq!(bsgs.order(), 4); // {id+, id-, swap+, swap-}
    }

    #[test]
    fn for_each_element_visits_every_group_element_exactly_once() {
        let gens = totally_antisymmetric_generators(3);
        let bsgs = Bsgs::from_generators(3, &gens);
        let mut seen: FxHashSet<SignedPerm> = FxHashSet::default();
        let mut count = 0u128;
        bsgs.for_each_element(|g| {
            assert!(bsgs.contains(g));
            assert!(seen.insert(g.clone()), "visited {g:?} twice");
            count += 1;
        });
        assert_eq!(count, bsgs.order());
        assert_eq!(seen.len() as u128, bsgs.order());
    }

    #[test]
    fn for_each_element_doubles_signs_under_global_negation() {
        let bsgs = Bsgs::from_generators(2, &[gen(2, 0, 1, 1), gen(2, 0, 1, -1)]);
        let mut seen: FxHashSet<SignedPerm> = FxHashSet::default();
        let mut count = 0u128;
        bsgs.for_each_element(|g| {
            assert!(bsgs.contains(g));
            assert!(seen.insert(g.clone()));
            count += 1;
        });
        assert_eq!(count, 4);
        // Both signs of the identity permutation and both signs of the swap.
        assert!(seen.contains(&SignedPerm::identity(2)));
        assert!(seen.contains(&SignedPerm::new(Perm::identity(2), -1)));
        assert!(seen.contains(&gen(2, 0, 1, 1)));
        assert!(seen.contains(&gen(2, 0, 1, -1)));
    }

    #[test]
    fn strong_generating_set_generates_full_group_order() {
        // Sanity check against brute-force enumeration of <gens> for a
        // small case, by closing the generator set under multiplication.
        let gens = totally_antisymmetric_generators(3);
        let bsgs = Bsgs::from_generators(3, &gens);

        let mut elems: FxHashSet<SignedPerm> = FxHashSet::default();
        elems.insert(SignedPerm::identity(3));
        let mut frontier: Vec<SignedPerm> = vec![SignedPerm::identity(3)];
        while let Some(g) = frontier.pop() {
            for s in &gens {
                let h = g.then(s);
                if elems.insert(h.clone()) {
                    frontier.push(h);
                }
            }
        }
        assert_eq!(elems.len() as u128, bsgs.order());
        for e in &elems {
            assert!(bsgs.contains(e));
        }
    }

    // Small brute-force permutation generator for tests only.
    fn permutations_of(n: usize) -> Vec<Vec<u16>> {
        fn permute(prefix: &mut Vec<u16>, remaining: &mut Vec<u16>, out: &mut Vec<Vec<u16>>) {
            if remaining.is_empty() {
                out.push(prefix.clone());
                return;
            }
            for i in 0..remaining.len() {
                let x = remaining.remove(i);
                prefix.push(x);
                permute(prefix, remaining, out);
                prefix.pop();
                remaining.insert(i, x);
            }
        }
        let mut out = Vec::new();
        permute(&mut Vec::new(), &mut (0..n as u16).collect(), &mut out);
        out
    }
}
