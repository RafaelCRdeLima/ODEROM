//! Linearizes a [`Monomial`] into a fixed point space, builds the acting
//! group `S ⋊ P` on that space (per-factor slot symmetry, direct product,
//! plus permutation of factors sharing a head), and scores a candidate
//! group element by the "word" it produces.
//!
//! # Layout
//!
//! The point space is *not* laid out in the monomial's original factor
//! order: factors are first sorted by [`HeadId`] (declaration order),
//! stably. This makes the whole construction independent of the order the
//! user typed factors in -- `"g[e,f] R[a,b,c,d]"` and `"R[a,b,c,d] g[e,f]"`
//! must produce identical canonical forms, and they do because both build
//! the same point space and the same generator set.
//!
//! # Word
//!
//! For a fully-specified group element `g` (a permutation of the point
//! space), `word(g)[p]` for `p = g(i)` is:
//! - `(0, rank)` if point `i` is a free index, `rank` being `i`'s label's
//!   alphabetical position among this monomial's free labels;
//! - `(1, g(partner(i)))` if point `i` is a dummy, `partner(i)` being the
//!   point it is contracted with.
//!
//! Note `Matching` already treats a contraction as an unordered pair (see
//! `oderom-core`), so there is no separate "orientation" group to search
//! over here: the dummy group `D` of the classical Butler-Portugal
//! formulation is already absorbed into the representation, exactly
//! because a dummy index has no name to reorient.

use oderom_core::{HeadId, Monomial, Registry, SignedPerm, SlotId};
use rustc_hash::FxHashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PointKind {
    Free(u16),
    Dummy(u16),
}

pub(crate) struct Layout {
    /// `canon_factor_order[k]` = original factor index placed at block `k`.
    pub canon_factor_order: Vec<usize>,
    /// `block_start[k]` = first point index of block `k`.
    pub block_start: Vec<u16>,
    pub degree: usize,
}

impl Layout {
    fn factor_to_block(&self) -> FxHashMap<usize, usize> {
        self.canon_factor_order.iter().enumerate().map(|(k, &f)| (f, k)).collect()
    }

    pub(crate) fn point_of(&self, factor_to_block: &FxHashMap<usize, usize>, slot: SlotId) -> u16 {
        let k = factor_to_block[&(slot.factor as usize)];
        self.block_start[k] + slot.slot as u16
    }
}

pub(crate) fn build_layout(m: &Monomial, registry: &Registry) -> Layout {
    let mut order: Vec<usize> = (0..m.factors().len()).collect();
    order.sort_by_key(|&i| registry.head(m.factors()[i].head).id);

    let mut block_start = Vec::with_capacity(order.len());
    let mut degree = 0u16;
    for &f in &order {
        block_start.push(degree);
        degree += registry.head(m.factors()[f].head).arity() as u16;
    }

    Layout { canon_factor_order: order, block_start, degree: degree as usize }
}

pub(crate) struct Labeling {
    pub kinds: Vec<PointKind>,
}

pub(crate) fn build_labeling(m: &Monomial, layout: &Layout) -> Labeling {
    let factor_to_block = layout.factor_to_block();
    let mut kinds = vec![PointKind::Free(0); layout.degree];

    let mut labels: Vec<&str> = m.free().iter().map(|(_, label)| label.name()).collect();
    labels.sort_unstable();

    for (slot, label) in m.free() {
        let p = layout.point_of(&factor_to_block, *slot);
        let rank = labels.binary_search(&label.name()).expect("label collected above") as u16;
        kinds[p as usize] = PointKind::Free(rank);
    }
    for &(a, b) in m.contractions().pairs() {
        let pa = layout.point_of(&factor_to_block, a);
        let pb = layout.point_of(&factor_to_block, b);
        kinds[pa as usize] = PointKind::Dummy(pb);
        kinds[pb as usize] = PointKind::Dummy(pa);
    }

    Labeling { kinds }
}

/// Builds the generating set for `S ⋊ P` on `layout`'s point space:
/// each factor's own declared symmetry generators, embedded at its block,
/// plus adjacent block-swap generators between consecutive blocks sharing
/// a head (sufficient to generate the full symmetric group permuting any
/// number of same-head factors, since adjacent transpositions generate
/// `S_k`). Swaps carry sign +1: tensor components commute.
pub(crate) fn build_generators(m: &Monomial, registry: &Registry, layout: &Layout) -> Vec<SignedPerm> {
    let degree = layout.degree;
    let mut gens = Vec::new();

    for (k, &f) in layout.canon_factor_order.iter().enumerate() {
        let head = registry.head(m.factors()[f].head);
        let off = layout.block_start[k];
        for g in &head.symmetry_generators {
            gens.push(embed(g, off, degree));
        }
    }

    let head_at = |k: usize| -> HeadId { registry.head(m.factors()[layout.canon_factor_order[k]].head).id };
    for k in 0..layout.canon_factor_order.len().saturating_sub(1) {
        if head_at(k) == head_at(k + 1) {
            let arity = registry.head(m.factors()[layout.canon_factor_order[k]].head).arity();
            gens.push(block_swap(layout.block_start[k], layout.block_start[k + 1], arity, degree));
        }
    }

    gens
}

fn embed(g: &SignedPerm, off: u16, degree: usize) -> SignedPerm {
    let mut images: Vec<u16> = (0..degree as u16).collect();
    for i in 0..g.perm.len() as u16 {
        images[(off + i) as usize] = off + g.perm.image(i);
    }
    SignedPerm::new(oderom_core::Perm::from_images(images), g.sign)
}

fn block_swap(off_a: u16, off_b: u16, arity: usize, degree: usize) -> SignedPerm {
    let mut images: Vec<u16> = (0..degree as u16).collect();
    for i in 0..arity as u16 {
        images[(off_a + i) as usize] = off_b + i;
        images[(off_b + i) as usize] = off_a + i;
    }
    SignedPerm::new(oderom_core::Perm::from_images(images), 1)
}

/// One entry per canonical position: `(0, rank)` for a free index,
/// `(1, partner_position)` for a dummy.
pub(crate) type Word = Vec<(u8, u16)>;

pub(crate) fn compute_word(g: &SignedPerm, labeling: &Labeling) -> Word {
    let n = labeling.kinds.len();
    let mut word: Word = vec![(0, 0); n];
    for i in 0..n as u16 {
        let p = g.perm.image(i);
        word[p as usize] = match labeling.kinds[i as usize] {
            PointKind::Free(rank) => (0, rank),
            PointKind::Dummy(partner) => (1, g.perm.image(partner)),
        };
    }
    word
}
