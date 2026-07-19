//! Rewriting to a normal form: flatten associative `Add`/`Mul`, fold
//! rational constants, and collect like terms (`Add`) / like bases
//! (`Mul`, by summing exponents of matching bases) by structural
//! equality once children are canonically ordered (see [`crate::Expr`]'s
//! `Ord` impl).
//!
//! # Distribution happens, but only *after* cancellation
//!
//! `simplify_mul` groups factors by base and sums their exponents
//! treating *any* expression as a base -- including a bare sum, and,
//! via [`canonical_sum_sign`], a sum's algebraic negation as the *same*
//! base -- before it ever distributes anything. That means `f * f^-1`
//! and `f * (-f)^-1` (`f` an `Add`) both collapse for free, without
//! expanding `f`.
//!
//! Two earlier versions got this wrong in opposite directions. One
//! distributed products over sums eagerly; that broke the cancellation
//! above, because replacing `f` with its terms before the grouping step
//! ran permanently hid the `f^-1` it should have met. Another (once sign
//! canonicalization was added, represented by literally rewriting `-f`
//! to `Mul([-1, f])` in the *output*) removed distribution at exponent 1
//! to stop that rewrite from being immediately undone -- which broke
//! expanding `(x+1)^2`, since peeling one copy of the square leaves a
//! bare exponent-1 sum (`x * (x+1)`, from multiplying the peeled `x+1`
//! by its own `x` term) that needs distributing too. Both are caught by
//! the Schwarzschild acceptance test in `oderom-components`, whose
//! Christoffel/Riemann formulas need both sums multiplied out *and*
//! `(1-2M/r)`-shaped factors to cancel against their negations.
//!
//! The fix neither version had: canonicalize a sum's sign only as part
//! of the *comparison* used for grouping (folding the resulting
//! `(-1)^exp` into the running rational coefficient instead), never by
//! rewriting the sum itself in the output. With that, distribution can
//! run unconditionally on any base surviving with a positive exponent,
//! one copy at a time (`sum^n` peels off a single factor of `sum`,
//! leaving `sum^(n-1)` alongside it, rather than expanding all at once),
//! and [`normalize`]'s outer fixed-point loop repeats
//! cancel-including-by-sign-then-peel-one-copy until no sum-typed base
//! has a positive exponent left -- so a later pass's distribution still
//! gets a chance to cancel against whatever an earlier pass exposed.
//! cancel against whatever an earlier pass exposed.
//!
//! [`normalize`] iterates one bottom-up rewrite pass to a fixed point;
//! each pass strictly reduces the tree's node count (folding/collecting),
//! so this terminates.

use crate::Expr;
use oderom_core::Scalar;
use std::collections::BTreeMap;

const MAX_ITERS: usize = 64;

/// Rewrites `e` to normal form (see module docs).
pub fn normalize(e: &Expr) -> Expr {
    let mut cur = e.clone();
    for _ in 0..MAX_ITERS {
        let next = step(&cur);
        if next == cur {
            return next;
        }
        cur = next;
    }
    cur
}

fn step(e: &Expr) -> Expr {
    match e {
        Expr::Rational(_) | Expr::Var(_) => e.clone(),
        Expr::Add(terms) => simplify_add(terms.iter().map(step).collect()),
        Expr::Mul(factors) => simplify_mul(factors.iter().map(step).collect()),
        Expr::Pow(base, n) => simplify_pow(step(base), *n),
        Expr::Sin(inner) => Expr::Sin(Box::new(step(inner))),
        Expr::Cos(inner) => Expr::Cos(Box::new(step(inner))),
    }
}

/// Splits a (already-simplified) term into a rational coefficient and the
/// remaining "shape", e.g. `Mul([Rational(3), x])` -> `(3, x)`, a bare
/// `Rational(3)` -> `(3, one())`, and anything else -> `(1, term)`.
fn split_coeff(term: Expr) -> (Scalar, Expr) {
    match term {
        Expr::Rational(s) => (s, Expr::one()),
        Expr::Mul(factors) => {
            let mut coeff = Scalar::ONE;
            let mut rest = Vec::with_capacity(factors.len());
            for f in factors {
                if let Expr::Rational(s) = f {
                    coeff = coeff * s;
                } else {
                    rest.push(f);
                }
            }
            let rest = match rest.len() {
                0 => Expr::one(),
                1 => rest.into_iter().next().expect("checked len==1"),
                _ => Expr::Mul(rest),
            };
            (coeff, rest)
        }
        other => (Scalar::ONE, other),
    }
}

/// Rebuilds `coeff * rest`, merging into an existing `Mul` rather than
/// nesting one, and collapsing away a coefficient of 1 / a `rest` of 1.
fn scale(coeff: Scalar, rest: Expr) -> Expr {
    if coeff.is_zero() {
        return Expr::zero();
    }
    if rest == Expr::one() {
        return Expr::Rational(coeff);
    }
    if coeff == Scalar::ONE {
        return rest;
    }
    match rest {
        Expr::Mul(mut factors) => {
            factors.insert(0, Expr::Rational(coeff));
            Expr::Mul(factors)
        }
        other => Expr::Mul(vec![Expr::Rational(coeff), other]),
    }
}

/// Given the (already-simplified, sorted) terms of an `Add`, returns the
/// terms `simplify_add` would have produced for its algebraic negation
/// -- i.e. every term's coefficient flipped, re-sorted -- together with
/// `-1` if a flip was actually needed (the original's leading term had a
/// negative coefficient) or `1` if `terms` was already in that canonical
/// form. Used only to compare/group a sum as a `Mul` base; see
/// `simplify_mul`.
fn canonical_sum_sign(terms: &[Expr]) -> (Vec<Expr>, i32) {
    match terms.first() {
        None => (terms.to_vec(), 1),
        Some(first) if split_coeff(first.clone()).0.numerator() >= 0 => (terms.to_vec(), 1),
        Some(_) => {
            let mut negated: Vec<Expr> = terms
                .iter()
                .cloned()
                .map(|t| {
                    let (c, rest) = split_coeff(t);
                    scale(-c, rest)
                })
                .collect();
            negated.sort();
            (negated, -1)
        }
    }
}

fn simplify_add(terms: Vec<Expr>) -> Expr {
    finish_add(combine_over_common_denominators(simplify_add_basic(terms)))
}

fn finish_add(mut out: Vec<Expr>) -> Expr {
    out.retain(|t| !t.is_zero());
    out.sort();
    match out.len() {
        0 => Expr::zero(),
        1 => out.into_iter().next().expect("checked len==1"),
        _ => Expr::Add(out),
    }
}

/// Flattens nested `Add`s and collects like terms by exact structural
/// match of their non-coefficient part. Does *not* attempt the
/// common-denominator combination [`combine_over_common_denominators`]
/// does -- kept separate so that function can call this one for its own
/// numerator combination without recursing back into itself.
fn simplify_add_basic(terms: Vec<Expr>) -> Vec<Expr> {
    let mut flat = Vec::with_capacity(terms.len());
    for t in terms {
        match t {
            Expr::Add(inner) => flat.extend(inner),
            other => flat.push(other),
        }
    }

    let mut grouped: BTreeMap<Expr, Scalar> = BTreeMap::new();
    for t in flat {
        let (coeff, rest) = split_coeff(t);
        let entry = grouped.entry(rest).or_insert(Scalar::ZERO);
        *entry = *entry + coeff;
    }

    grouped.into_iter().filter(|(_, c)| !c.is_zero()).map(|(rest, coeff)| scale(coeff, rest)).collect()
}

/// Splits `term` (a `Mul` or a single factor) into its factors other than
/// one `Pow(sum, k)` with `k < 0` -- the first such factor found -- and
/// that factor's `(sum, k)`, if any.
fn extract_negative_power_of_sum(term: &Expr) -> (Vec<Expr>, Option<(Expr, i32)>) {
    let factors: Vec<Expr> = match term {
        Expr::Mul(fs) => fs.clone(),
        other => vec![other.clone()],
    };
    let mut numerator = Vec::with_capacity(factors.len());
    let mut denominator = None;
    for f in factors {
        if denominator.is_none() {
            if let Expr::Pow(base, k) = &f {
                if *k < 0 && matches!(base.as_ref(), Expr::Add(_)) {
                    denominator = Some(((**base).clone(), *k));
                    continue;
                }
            }
        }
        numerator.push(f);
    }
    (numerator, denominator)
}

/// Terms that are only reciprocals of the *same* sum at different negative
/// powers (`A*f^-2 + B*f^-4`) never share a `rest` key in the grouping
/// above, so they'd otherwise sit side by side unsimplified forever no
/// matter how many `normalize` passes run -- there is no single common
/// "rest" to group them by until they're brought to a common denominator
/// first. This is the rational-function-normal-form step the
/// Kretschmann-of-Schwarzschild acceptance test (`oderom-components`)
/// actually needs: Christoffel/Riemann terms accumulate several distinct
/// negative powers of `(1 - 2M/r)` that only cancel once combined this
/// way and the resulting numerator is (via `normalize`'s outer loop,
/// which reprocesses the combined numerator) fully expanded.
fn combine_over_common_denominators(terms: Vec<Expr>) -> Vec<Expr> {
    let mut by_denominator: BTreeMap<Expr, Vec<(Vec<Expr>, i32)>> = BTreeMap::new();
    let mut plain = Vec::new();
    for t in terms {
        match extract_negative_power_of_sum(&t) {
            (numerator, Some((base, k))) => by_denominator.entry(base).or_default().push((numerator, k)),
            (_, None) => plain.push(t),
        }
    }

    // If exactly one sum is acting as a denominator anywhere in this sum,
    // every denominator-free term can join its group too (at exponent 0),
    // which is what lets e.g. `16*M^2/r^6 + (stuff)/f^4` recognize that
    // `16*M^2/r^6` is `16*M^2/r^6 * f^4 / f^4` and combine into the same
    // numerator -- without this, a term that needs no denominator of its
    // own never gets a chance to cancel against one a *different* term
    // introduced. Two or more distinct denominator sums in the same sum
    // is left uncombined (out of scope: nothing in the Christoffel/
    // Riemann/Kretschmann pipeline this exists for produces that case).
    if by_denominator.len() == 1 {
        let base = by_denominator.keys().next().expect("checked len==1").clone();
        let group = by_denominator.get_mut(&base).expect("just read this key");
        for t in plain.drain(..) {
            group.push((vec![t], 0));
        }
    }

    for (base, mut group) in by_denominator {
        // A lone term with this denominator is already in as reduced a
        // form as this function knows how to produce; rebuilding it
        // through simplify_mul/simplify_add anyway risks re-triggering
        // this same combination on the very term it just built (its
        // numerator can itself carry a *positive* power of `base` left
        // over from an earlier combination, and multiplying that back
        // against `base^min_k` can net out positive again, which
        // simplify_mul's distribution then expands right back into a sum
        // with a negative power of `base` in it) -- an oscillation that,
        // unlike the sign-canonicalization one this module already
        // documents, recurses instead of just failing to converge.
        if group.len() == 1 {
            let (numerator, k) = group.remove(0);
            let mut factors = numerator;
            factors.push(Expr::Pow(Box::new(base), k));
            factors.sort();
            plain.push(if factors.len() == 1 {
                factors.into_iter().next().expect("checked len==1")
            } else {
                Expr::Mul(factors)
            });
            continue;
        }
        let min_k = group.iter().map(|(_, k)| *k).min().expect("checked len > 1 above");
        let numerator_terms: Vec<Expr> = group
            .into_iter()
            .map(|(mut numerator, k)| {
                if k > min_k {
                    numerator.push(Expr::Pow(Box::new(base.clone()), k - min_k));
                }
                simplify_mul(numerator)
            })
            .collect();
        // simplify_add_basic, not simplify_add: this only needs to collect
        // like terms among the numerators, not re-run common-denominator
        // combination on them, which would recurse into this same
        // function without ever reducing the problem.
        let combined_numerator = finish_add(simplify_add_basic(numerator_terms));

        // The numerator is now fully expanded into monomials (each k >
        // min_k adjustment above pushed a *positive* power of `base`,
        // which simplify_mul's distribution expands unconditionally), so
        // it can no longer be compared against `base` by matching bases
        // the way ordinary cancellation does. But a numerator that is an
        // exact multiple of `base^(-min_k)` once *that* is expanded the
        // same way is exactly what a fully-collapsing rational function
        // (like the Kretschmann scalar) produces, term for term -- so
        // check for that directly instead of hoping some other rewrite
        // stumbles onto it.
        if let Some(q) = divide_by_expanded_power(&combined_numerator, &base, -min_k) {
            plain.push(q);
            continue;
        }

        // Assembled directly, *not* via simplify_mul: if combined_numerator
        // still carries `base` at a positive exponent (left over from one
        // of the k > min_k adjustments above), simplify_mul's distribution
        // branch would expand it back into a sum containing a negative
        // power of `base` -- feeding this exact function again, inside the
        // same call stack, with no smaller a problem than it started with.
        // Any further reduction that needs is picked up on `normalize`'s
        // *next* top-level pass instead, which is bounded by MAX_ITERS
        // rather than the call stack.
        let mut factors = match combined_numerator {
            Expr::Mul(fs) => fs,
            other => vec![other],
        };
        factors.push(Expr::Pow(Box::new(base), min_k));
        factors.sort();
        plain.push(Expr::Mul(factors));
    }
    plain
}

fn as_term_list(expr: &Expr) -> Vec<Expr> {
    match expr {
        Expr::Add(terms) => terms.clone(),
        other => vec![other.clone()],
    }
}

/// If `numerator` (already a sum of monomials) equals `Q * base^n` for
/// some monomial `Q` (a rational times, e.g., `M^2 * r^-6` -- not
/// necessarily just a rational constant: the Kretschmann scalar's
/// numerator over `(1-2M/r)^4` is `48*M^2/r^6` times the expansion of
/// `(1-2M/r)^4`, not a bare number), returns `Q`, term for term once
/// `base^n` is itself expanded the same way. `n` must be positive (a
/// negative one can't be expanded into a finite polynomial to compare
/// against in the first place).
///
/// Pairs terms by position after independently sorting both term lists,
/// rather than by matching "rest" keys directly (which `Q` scaling
/// necessarily changes for every term): this assumes multiplying every
/// term of `base^n`'s expansion by the same `Q` doesn't reorder them
/// relative to each other, true of the monomial `Q`s this exists for.
fn divide_by_expanded_power(numerator: &Expr, base: &Expr, n: i32) -> Option<Expr> {
    if n <= 0 {
        return None;
    }
    let expanded = normalize(&Expr::Pow(Box::new(base.clone()), n));
    let mut num_terms = as_term_list(numerator);
    let mut exp_terms = as_term_list(&expanded);
    if num_terms.len() != exp_terms.len() {
        return None;
    }
    // Sort by each term's *rest* (its shape, ignoring the coefficient),
    // not by the term as a whole: `Expr`'s `Ord` compares a `Mul`'s
    // leading `Rational` factor first, so sorting terms directly orders
    // them by coefficient magnitude/sign -- unrelated to which power of
    // `base` each one corresponds to, and useless for lining the two
    // lists up. Sorting by rest instead orders monomials by degree
    // (`Pow`'s `Ord` compares matching bases by exponent), which is
    // exactly the correspondence multiplying every term of `base^n` by
    // the same monomial `Q` preserves.
    num_terms.sort_by(|a, b| split_coeff(a.clone()).1.cmp(&split_coeff(b.clone()).1));
    exp_terms.sort_by(|a, b| split_coeff(a.clone()).1.cmp(&split_coeff(b.clone()).1));

    let mut ratio: Option<Expr> = None;
    for (nt, et) in num_terms.iter().zip(exp_terms.iter()) {
        let (et_coeff, et_rest) = split_coeff(et.clone());
        let mut factors = vec![nt.clone(), Expr::Rational(et_coeff.recip()?)];
        if et_rest != Expr::one() {
            factors.push(Expr::Pow(Box::new(et_rest), -1));
        }
        let candidate = normalize(&Expr::Mul(factors));
        match &ratio {
            None => ratio = Some(candidate),
            Some(existing) if *existing == candidate => {}
            Some(_) => return None,
        }
    }
    ratio
}

fn simplify_mul(factors: Vec<Expr>) -> Expr {
    let mut flat = Vec::with_capacity(factors.len());
    for f in factors {
        match f {
            Expr::Mul(inner) => flat.extend(inner),
            other => flat.push(other),
        }
    }

    let mut coeff = Scalar::ONE;
    let mut bases: BTreeMap<Expr, i32> = BTreeMap::new();
    for f in flat {
        let (base, exp) = match f {
            Expr::Pow(b, n) => (*b, n),
            other => (other, 1),
        };
        if let Expr::Rational(s) = &base {
            match scalar_pow(*s, exp) {
                Some(folded) => {
                    coeff = coeff * folded;
                    continue;
                }
                None => {
                    // 0^negative: leave opaque rather than guess.
                    *bases.entry(base).or_insert(0) += exp;
                    continue;
                }
            }
        }
        // A sum and its algebraic negation must group as the same base
        // (`f^-1 * (-f) = -1`, not two unrelated opaque factors), since
        // `(-f)^n = (-1)^n * f^n`. Canonicalize `base` to the sign
        // `simplify_add` would have produced had it built this exact sum
        // itself, folding the resulting `(-1)^exp` into `coeff` -- rather
        // than wrapping `-f` as `Mul([-1, f])` in the *output*, which
        // would hand `f` straight back to the exponent-1 distribution
        // branch below and undo this on the very next pass.
        if let Expr::Add(terms) = &base {
            let (canon_terms, sign) = canonical_sum_sign(terms);
            if sign < 0 {
                if let Some(folded) = scalar_pow(Scalar::new(-1, 1), exp) {
                    coeff = coeff * folded;
                }
                *bases.entry(Expr::Add(canon_terms)).or_insert(0) += exp;
                continue;
            }
        }
        *bases.entry(base).or_insert(0) += exp;
    }

    if coeff.is_zero() {
        return Expr::zero();
    }
    bases.retain(|_, e| *e != 0);

    // Same-base cancellation above already resolved every case it can --
    // including `f * f^-1 -> 1` for `f` a sum, and, via
    // `canonical_sum_sign`, `f * (-f)^-1 -> -1` -- all without expanding
    // `f`, because grouping treats a sum as just another base *before*
    // this point ever runs. Only *after* that do we expand: a sum
    // surviving with a positive exponent gets exactly one copy peeled
    // off and distributed over the rest of the product, leaving
    // `normalize`'s outer fixed-point loop to repeat this
    // (cancel-including-by-sign, then peel one more copy) until no
    // sum-typed base has a positive exponent left. Peeling one copy at a
    // time, rather than expanding `sum^n` all at once, keeps giving
    // cancellation a chance against whatever the *next* pass's
    // distribution exposes -- including a copy peeled off `sum^2` down
    // to `sum^1` multiplying one of `sum`'s own terms, which needs
    // exponent 1 to still distribute to fully expand (e.g. `(x+1)^2`);
    // an earlier version stopped distributing at exponent 1 to dodge an
    // infinite oscillation with sign canonicalization, and that broke
    // exactly this case instead -- fixed by moving sign canonicalization
    // into the grouping step above rather than the output, so it no
    // longer fights distribution.
    if let Some((sum_base, exp)) = bases.iter().find(|(b, e)| matches!(b, Expr::Add(_)) && **e > 0) {
        let sum_base = sum_base.clone();
        let exp = *exp;
        let Expr::Add(sum_terms) = sum_base.clone() else { unreachable!() };

        let mut rest: Vec<Expr> = bases
            .into_iter()
            .filter(|(b, _)| *b != sum_base)
            .map(|(base, e)| if e == 1 { base } else { Expr::Pow(Box::new(base), e) })
            .collect();
        if exp > 1 {
            rest.push(Expr::Pow(Box::new(sum_base), exp - 1));
        }
        if coeff != Scalar::ONE {
            rest.push(Expr::Rational(coeff));
        }

        let distributed: Vec<Expr> = sum_terms
            .into_iter()
            .map(|term| {
                let mut factors = rest.clone();
                factors.push(term);
                simplify_mul(factors)
            })
            .collect();
        return simplify_add(distributed);
    }

    let mut out: Vec<Expr> = bases
        .into_iter()
        .map(|(base, exp)| if exp == 1 { base } else { Expr::Pow(Box::new(base), exp) })
        .collect();
    out.sort();

    if coeff != Scalar::ONE {
        out.insert(0, Expr::Rational(coeff));
    }

    match out.len() {
        0 => Expr::one(),
        1 => out.into_iter().next().expect("checked len==1"),
        _ => Expr::Mul(out),
    }
}

fn simplify_pow(base: Expr, n: i32) -> Expr {
    if n == 0 {
        return Expr::one();
    }
    if n == 1 {
        return base;
    }
    match base {
        Expr::Rational(s) => match scalar_pow(s, n) {
            Some(folded) => Expr::Rational(folded),
            None => Expr::Pow(Box::new(Expr::Rational(s)), n),
        },
        Expr::Pow(inner, m) => simplify_pow(*inner, n * m),
        // (f*g)^n = f^n * g^n, for any integer n. Distributing over the
        // factors -- rather than leaving `Pow(Mul(..), n)` opaque -- is
        // what lets e.g. `(-1*f)^-2` fold its `(-1)^-2 = 1` away and
        // combine with a bare `f^-2` elsewhere as the same base.
        Expr::Mul(factors) => simplify_mul(factors.into_iter().map(|f| simplify_pow(f, n)).collect()),
        // A positive power of a sum needs the same cancel-then-peel
        // treatment as inside a Mul (see the module docs) -- route it
        // through simplify_mul as a singleton product so it is.
        sum @ Expr::Add(_) if n > 1 => simplify_mul(vec![Expr::Pow(Box::new(sum), n)]),
        other => Expr::Pow(Box::new(other), n),
    }
}

fn scalar_pow(s: Scalar, n: i32) -> Option<Scalar> {
    if n == 0 {
        return Some(Scalar::ONE);
    }
    let (base, n) = if n < 0 { (s.recip()?, -n) } else { (s, n) };
    let mut acc = Scalar::ONE;
    for _ in 0..n {
        acc = acc * base;
    }
    Some(acc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Expr;

    #[test]
    fn folds_rational_arithmetic() {
        let e = Expr::int(2) + Expr::int(3) * Expr::int(4);
        assert_eq!(normalize(&e), Expr::int(14));
    }

    #[test]
    fn divide_by_expanded_power_finds_a_monomial_quotient() {
        // The exact numerator/denominator pair the Kretschmann-of-
        // Schwarzschild computation (oderom-components) produces just
        // before its final collapse: `48*M^2/r^6 * (1-2M/r)^4`, written
        // out as already-expanded monomials, divided by `(1-2M/r)^4`.
        // The quotient is a monomial (`48*M^2/r^6`), not a bare rational
        // -- this is what distinguishes it from a simpler C-only case.
        let m = Expr::var("M");
        let r = Expr::var("r");
        let f = Expr::one() - Expr::int(2) * m.clone() / r.clone();
        let numerator = Expr::int(-1536) * m.clone().pow(5) * r.clone().pow(-9)
            + Expr::int(-384) * m.clone().pow(3) * r.clone().pow(-7)
            + Expr::int(48) * m.clone().pow(2) * r.clone().pow(-6)
            + Expr::int(768) * m.clone().pow(6) * r.clone().pow(-10)
            + Expr::int(1152) * m.pow(4) * r.pow(-8);
        let numerator = normalize(&numerator);

        let quotient = divide_by_expanded_power(&numerator, &f, 4);
        let expected = Expr::int(48) * Expr::var("M").pow(2) * Expr::var("r").pow(-6);
        assert_eq!(quotient, Some(normalize(&expected)));
    }

    #[test]
    fn combines_like_terms_regardless_of_order() {
        let x = Expr::var("x");
        let a = normalize(&(x.clone() + x.clone()));
        let b = normalize(&(Expr::int(2) * x));
        assert_eq!(a, b);
        assert_eq!(a, Expr::int(2) * Expr::var("x"));
    }

    #[test]
    fn combines_like_powers() {
        let x = Expr::var("x");
        let e = x.clone() * x.clone().pow(2);
        assert_eq!(normalize(&e), Expr::var("x").pow(3));
    }

    #[test]
    fn cancels_reciprocal_factor() {
        // r * r^-1 = 1
        let r = Expr::var("r");
        let e = r.clone() * r.pow(-1);
        assert_eq!(normalize(&e), Expr::one());
    }

    #[test]
    fn cancels_reciprocal_of_a_sum() {
        // (1 - 2M/r) * (1 - 2M/r)^-1 = 1, without ever expanding the
        // negative power -- this is the mechanism the Schwarzschild
        // Kretschmann computation leans on.
        let f = Expr::one() - Expr::int(2) * Expr::var("M") / Expr::var("r");
        let e = f.clone() * Expr::Pow(Box::new(f), -1);
        assert_eq!(normalize(&e), Expr::one());
    }

    #[test]
    fn a_sum_and_its_algebraic_negation_are_recognized_as_the_same_base() {
        // f^-1 * (-f) = -1, where "-f" arrives already distributed out
        // (as `-1 + 2M/r`, not `-1 * (1 - 2M/r)`) -- e.g. as the literal
        // output of an earlier subtraction. Requires simplify_add's sign
        // canonicalization: without it, `1 - 2M/r` and `-1 + 2M/r` are
        // unrelated trees and this Pow can never find its match.
        let f = Expr::one() - Expr::int(2) * Expr::var("M") / Expr::var("r");
        let neg_f = Expr::int(-1) + Expr::int(2) * Expr::var("M") / Expr::var("r");
        let e = Expr::Pow(Box::new(f), -1) * neg_f;
        assert_eq!(normalize(&e), Expr::int(-1));
    }

    #[test]
    fn distributes_multiplication_over_addition() {
        let x = Expr::var("x");
        let e = Expr::int(2) * (x.clone() + Expr::int(3));
        let expected = Expr::int(2) * x + Expr::int(6);
        assert_eq!(normalize(&e), normalize(&expected));
    }

    #[test]
    fn expands_positive_integer_power_of_a_sum() {
        // (x + 1)^2 = x^2 + 2x + 1
        let x = Expr::var("x");
        let e = (x.clone() + Expr::one()).pow(2);
        let expected = x.clone().pow(2) + Expr::int(2) * x + Expr::one();
        assert_eq!(normalize(&e), normalize(&expected));
    }

    #[test]
    fn a_bare_sum_still_cancels_its_own_reciprocal_power_instead_of_expanding() {
        // f * f^-1 = 1 -- cancellation runs before distribution gets a
        // chance to expand f (see cancels_reciprocal_of_a_sum for the
        // same mechanism with the factors reversed).
        let x = Expr::var("x");
        let f = x + Expr::one();
        let e = Expr::Pow(Box::new(f.clone()), -1) * f;
        assert_eq!(normalize(&e), Expr::one());
    }

    #[test]
    fn cancellation_survives_alongside_unrelated_expansion() {
        // (x+1)^-1 * (x+1) * (y+2) = y + 2: the reciprocal pair cancels
        // instead of both being expanded, while the unrelated sum still
        // distributes over the constant coefficient implied by the rest
        // of a larger product.
        let x = Expr::var("x");
        let y = Expr::var("y");
        let f = x + Expr::one();
        let e = Expr::Pow(Box::new(f.clone()), -1) * f * (y.clone() + Expr::int(2));
        assert_eq!(normalize(&e), normalize(&(y + Expr::int(2))));
    }

    #[test]
    fn zero_coefficient_terms_vanish() {
        let x = Expr::var("x");
        let e = x.clone() - x;
        assert_eq!(normalize(&e), Expr::zero());
    }

    #[test]
    fn idempotent() {
        let x = Expr::var("x");
        let e = (x.clone() + Expr::one()).pow(2) * Expr::int(3);
        let once = normalize(&e);
        let twice = normalize(&once);
        assert_eq!(once, twice);
    }
}
