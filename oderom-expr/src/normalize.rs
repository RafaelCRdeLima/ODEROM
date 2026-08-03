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

thread_local! {
    static USE_LEGACY: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Cached once per thread (checking an env var on every `normalize()`
/// call would be wasteful, and this crate's own `ODEROM_REDUCE_STATS`
/// timing already establishes that pattern). Set `ODEROM_ENGINE=legacy`
/// to use the pre-rational-form engine instead of the default.
fn use_legacy() -> bool {
    USE_LEGACY.with(|c| {
        if let Some(v) = c.get() {
            return v;
        }
        let v = std::env::var("ODEROM_ENGINE").as_deref() == Ok("legacy");
        c.set(Some(v));
        v
    })
}

/// Rewrites `e` to normal form (see module docs and
/// DESIGN-RATIONAL-FORM.md): converts to a canonical
/// [`crate::rational_function::RationalFunction`] over one per-call
/// [`crate::poly::AtomTable`], reduces by polynomial GCD (subresultant
/// PRS, plus recursive multivariate GCD for content), converts back.
///
/// `legacy_v1` (below) is the original ad hoc rewrite-to-fixed-point
/// engine this replaced -- kept deliberately, not deleted, as an escape
/// hatch (`ODEROM_ENGINE=legacy`) and as a permanent differential oracle
/// (`v1_and_v2_agree`, in this module's tests, keeps comparing the two on
/// every CI run for as long as both exist). It cannot represent
/// Reissner-Nordstrom's Kretschmann scalar at all (the computation that
/// motivated the rational-form engine in the first place: naive
/// expression swell, no GCD reduction ever), which is why this is a
/// one-way default switch, not a coin flip -- but per DESIGN-RATIONAL-
/// FORM.md, some metrics still make the recursive multivariate GCD's
/// cost blow up (dense, not sparse/modular): not by free-parameter count
/// alone (an earlier note said "three or more" -- real usage falsified
/// that with a 2-parameter counterexample), but by how little structural
/// cancellation is available to it, e.g. a metric whose `g_tt`/`g_rr`
/// are not reciprocal loses cancellation the textbook `-f dt^2 + f^-1
/// dr^2` form gets for free. `ODEROM_ENGINE=legacy` does not help there
/// since legacy_v1 cannot handle even two-parameter metrics with real
/// GCD reduction.
pub fn normalize(e: &Expr) -> Expr {
    if use_legacy() {
        legacy_v1::normalize_v1(e)
    } else {
        crate::canonical::normalize_via_rational_form(e)
    }
}

/// The original ad hoc rewrite-to-fixed-point engine. Not the default
/// anymore (see `normalize` above), but deliberately not deleted --
/// reachable via `ODEROM_ENGINE=legacy` and permanently exercised by the
/// `v1_and_v2_agree` differential test below for as long as both engines
/// exist.
mod legacy_v1 {
    use super::*;
    use crate::BigScalar;
    use std::collections::BTreeMap;

    const MAX_ITERS: usize = 64;

    pub(crate) fn normalize_v1(e: &Expr) -> Expr {
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
        // legacy_v1 never had D-RF.7's sin^2+cos^2=1 rewrite either --
        // just recurse into the argument, same as Sin/Cos above. No
        // asymmetry introduced: this engine has never known any
        // cross-atom identity, for any transcendental.
        Expr::Exp(inner) => Expr::Exp(Box::new(step(inner))),
        Expr::Sinh(inner) => Expr::Sinh(Box::new(step(inner))),
        Expr::Cosh(inner) => Expr::Cosh(Box::new(step(inner))),
        // Same reasoning as Sin/Cos/Exp/Sinh/Cosh above: legacy_v1 knows
        // no cross-atom identity for any transcendental, and an
        // indeterminate function has no identity at all (not even a
        // deferred one) -- just recurse into each argument.
        Expr::Func { name, args, order } => {
            Expr::Func { name: name.clone(), args: args.iter().map(step).collect(), order: order.clone() }
        }
    }
}

/// Splits a (already-simplified) term into a rational coefficient and the
/// remaining "shape", e.g. `Mul([Rational(3), x])` -> `(3, x)`, a bare
/// `Rational(3)` -> `(3, one())`, and anything else -> `(1, term)`.
fn split_coeff(term: Expr) -> (BigScalar, Expr) {
    match term {
        Expr::Rational(s) => (s, Expr::one()),
        Expr::Mul(factors) => {
            let mut coeff = BigScalar::one();
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
        other => (BigScalar::one(), other),
    }
}

/// Rebuilds `coeff * rest`, merging into an existing `Mul` rather than
/// nesting one, and collapsing away a coefficient of 1 / a `rest` of 1.
fn scale(coeff: BigScalar, rest: Expr) -> Expr {
    if coeff.is_zero() {
        return Expr::zero();
    }
    if rest == Expr::one() {
        return Expr::Rational(coeff);
    }
    if coeff == BigScalar::one() {
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
        Some(first) if !split_coeff(first.clone()).0.is_negative() => (terms.to_vec(), 1),
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

    let mut grouped: BTreeMap<Expr, BigScalar> = BTreeMap::new();
    for t in flat {
        let (coeff, rest) = split_coeff(t);
        let entry = grouped.entry(rest).or_insert(BigScalar::zero());
        *entry = entry.clone() + coeff;
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
        // exact multiple of some power of `base`, once that power is
        // itself expanded the same way, is exactly what a
        // fully-collapsing rational function (Kretschmann, or a metric
        // pullback through a chart transition) produces, term for term --
        // so check for that directly instead of hoping some other
        // rewrite stumbles onto it. Two exponents matter: `-min_k`
        // (exactly enough to clear the denominator) and, since the
        // numerator can carry a *higher* power of `base` than that
        // (nothing bounds it above), `numerator's own term count - 1` --
        // the only other exponent an expanded 2-term `base^p` could
        // possibly match a given monomial count at.
        let mut candidates = vec![-min_k];
        let overshoot = as_term_list(&combined_numerator).len() as i32 - 1;
        if overshoot > 0 && overshoot != -min_k {
            candidates.push(overshoot);
        }
        let found = candidates
            .into_iter()
            .find_map(|p| divide_by_expanded_power(&combined_numerator, &base, p).map(|q| (p, q)));
        if let Some((p, q)) = found {
            let final_exp = p + min_k;
            plain.push(if final_exp == 0 {
                q
            } else {
                let mut factors = match q {
                    Expr::Mul(fs) => fs,
                    other => vec![other],
                };
                factors.push(Expr::Pow(Box::new(base), final_exp));
                factors.sort();
                Expr::Mul(factors)
            });
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
pub(super) fn divide_by_expanded_power(numerator: &Expr, base: &Expr, n: i32) -> Option<Expr> {
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

    let mut coeff = BigScalar::one();
    let mut bases: BTreeMap<Expr, i32> = BTreeMap::new();
    for f in flat {
        let (base, exp) = match f {
            Expr::Pow(b, n) => (*b, n),
            other => (other, 1),
        };
        if let Expr::Rational(s) = &base {
            match scalar_pow(s.clone(), exp) {
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
                if let Some(folded) = scalar_pow(BigScalar::new(-1, 1), exp) {
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
    //
    // Tried restricting this to fire only when `sum_base` is the *lone*
    // sum-typed base in the product (blocking e.g. `R^2` from expanding
    // against an unrelated `(1+R)^-2`, which glues the resulting
    // monomials to `(1+R)^-2` with no cancellation to show for it). That
    // broke the Kretschmann computation, which genuinely needs
    // distribution while multiple unrelated sums are in play elsewhere
    // in the same product. The two needs are in real tension; resolving
    // it needs an explicit numerator/denominator representation
    // (`rationalize`) rather than another local rule here -- see that
    // function's docs.
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
        if coeff != BigScalar::one() {
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

    if coeff != BigScalar::one() {
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
        Expr::Rational(s) => match scalar_pow(s.clone(), n) {
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

fn scalar_pow(s: BigScalar, n: i32) -> Option<BigScalar> {
    if n == 0 {
        return Some(BigScalar::one());
    }
    let (base, n) = if n < 0 { (s.recip()?, -n) } else { (s, n) };
    let mut acc = BigScalar::one();
    for _ in 0..n {
        acc = acc * base.clone();
    }
    Some(acc)
}
} // mod legacy_v1

#[cfg(test)]
mod tests {
    use super::legacy_v1::normalize_v1;
    use super::*;
    use crate::Expr;

    /// Documents a real property found while building the exact-rational
    /// differential oracle (see `TrigMemo`'s doc comment): `normalize()`
    /// does not fold every value-equal placement of a constant scale
    /// factor around `Pow(-1)` into one canonical shape. Not a
    /// correctness bug -- both sides are individually valid, reduced
    /// `RationalFunction`s for the same value -- but worth a permanent
    /// regression marker so this is never "rediscovered" as a surprise;
    /// `assert_ne!` (not `assert_eq!`) because the different shape is the
    /// documented, current behavior, not a bug to fix here.
    #[test]
    fn normalize_does_not_canonicalize_scale_factor_placement_around_pow_neg1() {
        let x = Expr::var("x");
        let half_times_inverse = Expr::rational(1, 2) * x.clone().pow(-1);
        let inverse_of_double = Expr::Pow(Box::new(Expr::int(2) * x), -1);
        assert_ne!(normalize(&half_times_inverse), normalize(&inverse_of_double));
    }

    /// Answers the question the non-canonical-shape finding above raises
    /// (session request: "measure with a constructed case, don't
    /// estimate"): does the shape ambiguity ever survive as a
    /// *non-zero-looking* result for a quantity that is genuinely zero,
    /// which would make `Expr::is_zero()` under-report -- exactly what
    /// `render_classes` (`oderom-components/src/render.rs`) uses to
    /// count "N components identically zero"? Constructed to mirror the
    /// real pipeline's actual pattern (`christoffel`/`riemann_mixed`:
    /// normalize a component, store it, combine several *already-
    /// normalized* stored components into a fresh sum, normalize that
    /// sum again) rather than testing the two shapes in isolation.
    ///
    /// Result: benign. `half_times_inverse` and `inverse_of_double` are
    /// each normalized and stored first (as `christoffel`'s `Grid::set`
    /// would), then combined with opposite sign into one new `Add` and
    /// normalized again (as `riemann_mixed`'s own combination step
    /// would) -- and that second `normalize()` call *does* produce
    /// exactly `Rational(0)`. Mechanism, traced in `rational_function.rs`:
    /// whichever surface shape a sub-expression has, `expr_to_rational`
    /// re-derives its actual `RationalFunction` value from scratch, not
    /// from its printed form; `expr_to_rational`'s `Add` case
    /// accumulates a whole sum into one `(num, den)` pair via exact
    /// `Poly` cross-multiplication, and `reduce_inner_candidate`'s very
    /// first check is `if num.is_zero() { return .. 0 .. }`, unconditional
    /// on whatever shape `den` ended up in. The shape ambiguity (root
    /// cause: `BigScalar::gcd` returns a unit for any non-integer input,
    /// so `poly_gcd` never rescales a fractional leading coefficient) is
    /// real, but it only ever affects which of several equally-reduced
    /// *non-zero* forms gets rendered -- it cannot produce a "not
    /// obviously zero" result for something that truly is zero, because
    /// exact cancellation happens one level below any such rendering
    /// choice, in the `Poly` coefficient arithmetic itself.
    #[test]
    fn shape_ambiguity_around_pow_neg1_does_not_survive_as_a_false_nonzero() {
        let x = Expr::var("x");
        let stored_a = normalize(&(Expr::rational(1, 2) * x.clone().pow(-1)));
        let stored_b = normalize(&Expr::Pow(Box::new(Expr::int(2) * x), -1));
        assert_ne!(stored_a, stored_b, "test assumption: the two stored forms must actually differ in shape");

        let combined_later = stored_a + Expr::int(-1) * stored_b;
        let result = normalize(&combined_later);
        assert!(result.is_zero(), "shape ambiguity DID survive as a false non-zero: {result:?} -- not benign, needs a README limitation, not this comment");
    }

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

        let quotient = legacy_v1::divide_by_expanded_power(&numerator, &f, 4);
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

    // ---------------------------------------------------------------
    // Differential test (DESIGN-RATIONAL-FORM.md section 4 step 5): the
    // primary safety net for replacing normalize()'s internals, and (per
    // an explicit, permanent requirement) kept running for as long as
    // both engines exist -- `ODEROM_ENGINE=legacy` is the escape hatch
    // `normalize_v1` backs.
    //
    // Compares by VALUE (evaluate both sides at several sample points),
    // not structural equality: the two engines have different, each
    // internally valid, canonical forms for rational expressions (`v2`
    // always fully reduces to one num/den pair via polynomial GCD; `v1`
    // keeps an unreduced sum -- or a different sign/exponent convention
    // -- when it doesn't find a cancellation the way `v2`'s GCD does).
    // Confirmed real, not a loophole: several genuine structural
    // mismatches were found this way that were NOT value bugs (e.g.
    // `Pow(x,-2)` vs `Pow(Pow(x,2),-1)`, or `1-x` vs `-1*(x-1)` --
    // algebraically identical, different sign convention chosen). What
    // this must still catch, and does: any case where the two engines
    // disagree on the actual VALUE, which is the only property that
    // still has to hold as a differential oracle now that structural
    // equality is no longer expected.
    // ---------------------------------------------------------------

    // Replaces an earlier f64-based oracle (kept in git history, not
    // here): it evaluated `Sin(arg)`/`Cos(arg)` via real `.sin()`/
    // `.cos()` on `arg`'s own evaluated float, which does satisfy
    // sin^2+cos^2=1 for a genuine angle -- but the rest of the tree
    // (Add/Mul/Pow on possibly near-singular quantities) still ran in
    // f64, and near sin(M) close to -1 (found: `(1+sin(M))^-3` around
    // `M=-1.6`) catastrophic cancellation made two *algebraically equal*
    // results differ in the 6th decimal digit, past the tolerance, on a
    // sporadic proptest case that has to keep passing every run -- a
    // flaky property test is a property test someone disables. The oracle
    // itself was the bug, not the engines.
    //
    // No float anywhere below: `BigScalar` is exact arbitrary-precision
    // rational (`crate::BigScalar`, already what `Expr::Rational` holds),
    // and `Sin(arg)`/`Cos(arg)` are given exact rational values via the
    // standard rational parametrization of the unit circle -- for a
    // rational `t`, `cos = (1-t^2)/(1+t^2)`, `sin = 2t/(1+t^2)` satisfies
    // `sin^2+cos^2=1` identically (verify: `(1-t^2)^2+(2t)^2 =
    // 1-2t^2+t^4+4t^2 = 1+2t^2+t^4 = (1+t^2)^2`), and `1+t^2 >= 1` for
    // every rational `t`, so this parametrization's own denominator is
    // never zero -- the only way to hit an exact pole is the *outer*
    // expression genuinely dividing by zero at this point, which is
    // exactly the case this oracle already needs to detect (and skip,
    // never guess at) as a shared removable singularity.
    //
    // One `t` per *syntactically distinct* Sin/Cos argument, not per
    // node: memoized by `normalize(arg)` (never the raw `arg`), matching
    // exactly the key `AtomTable` interns Sin/Cos atoms by (D-RF.6) --
    // two arguments that are the same after normalizing get the same
    // `t` and therefore the same value, two that are not get independent
    // `t`s, precisely mirroring what the engine itself considers "the
    // same trig atom" instead of a coarser or finer notion the oracle
    // invented on its own. Assignment order is deterministic (a fixed
    // pool, cycled), not RNG -- proptest's shrinking reruns a failing
    // case many times and needs the same case to evaluate the same way
    // every time.
    use crate::BigScalar;
    use std::collections::HashMap;

    fn pow_exact(base: BigScalar, n: i32) -> Option<BigScalar> {
        if n == 0 {
            return Some(BigScalar::one());
        }
        let mut acc = BigScalar::one();
        for _ in 0..n.unsigned_abs() {
            acc = acc * base.clone();
        }
        if n < 0 { acc.recip() } else { Some(acc) }
    }

    /// `(cos, sin)` for one rational `t`, via the parametrization above.
    fn cos_sin_of_t(t: &BigScalar) -> (BigScalar, BigScalar) {
        let t2 = t.clone() * t.clone();
        let denom = BigScalar::one() + t2.clone();
        let inv = denom.clone().recip().expect("1+t^2 is never zero for a rational t");
        let cos = (BigScalar::one() - t2) * inv.clone();
        let sin = (BigScalar::new(2, 1) * t.clone()) * inv;
        (cos, sin)
    }

    /// A handful of small distinct rationals to assign as `t`, cycled --
    /// enough variety that a coincidental algebraic relation at one `t`
    /// is very unlikely to also hold at the next one tried.
    const T_POOL: &[(i64, i64)] = &[(2, 1), (-3, 1), (1, 3), (5, 2), (-1, 4), (7, 1), (-4, 3), (3, 5)];

    /// Keyed by `arg`'s own *evaluated numeric value* at this sample
    /// point, not by `arg`'s symbolic form (an earlier version of this
    /// oracle keyed by `normalize(arg)`, mirroring how `AtomTable`
    /// interns Sin/Cos atoms -- wrong here: found, by that version
    /// promptly failing, that `normalize()` does not put every
    /// value-equal argument into one canonical shape, e.g. `(1/2)*x^-1`
    /// stays `Mul([1/2, Pow(x,-1)])` while `(2*x)^-1` stays
    /// `Pow(Mul([2,x]),-1)` -- same rational function, two different
    /// trees, so keying by the normalized tree assigned them two
    /// unrelated `t`s and manufactured a disagreement between two
    /// engines that were both right. Keying by the evaluated *value*
    /// sidesteps the question of how canonical `normalize()`'s output is
    /// altogether: whatever symbolic form `arg` takes, if it evaluates to
    /// the same rational number here, it gets the same `t` -- which is
    /// also exactly what this oracle's very first (float) version did
    /// (`eval(arg,vars).sin()`), just exact now instead of approximate.
    struct TrigMemo {
        assigned: HashMap<BigScalar, (BigScalar, BigScalar)>,
        next: usize,
    }

    impl TrigMemo {
        fn new(seed_offset: usize) -> Self {
            TrigMemo { assigned: HashMap::new(), next: seed_offset }
        }

        fn cos_sin(&mut self, arg_value: BigScalar) -> (BigScalar, BigScalar) {
            if let Some(v) = self.assigned.get(&arg_value) {
                return v.clone();
            }
            let (num, den) = T_POOL[self.next % T_POOL.len()];
            self.next += 1;
            let t = BigScalar::new(num, den);
            let v = cos_sin_of_t(&t);
            self.assigned.insert(arg_value, v.clone());
            v
        }
    }

    fn eval_exact(e: &Expr, vars: &[(&str, BigScalar)], trig: &mut TrigMemo) -> Option<BigScalar> {
        match e {
            Expr::Rational(s) => Some(s.clone()),
            Expr::Var(name) => vars.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone()),
            Expr::Add(terms) => {
                let mut acc = BigScalar::zero();
                for t in terms {
                    acc = acc + eval_exact(t, vars, trig)?;
                }
                Some(acc)
            }
            Expr::Mul(factors) => {
                let mut acc = BigScalar::one();
                for f in factors {
                    acc = acc * eval_exact(f, vars, trig)?;
                }
                Some(acc)
            }
            Expr::Pow(base, n) => pow_exact(eval_exact(base, vars, trig)?, *n),
            Expr::Sin(arg) => {
                let v = eval_exact(arg, vars, trig)?;
                Some(trig.cos_sin(v).1)
            }
            Expr::Cos(arg) => {
                let v = eval_exact(arg, vars, trig)?;
                Some(trig.cos_sin(v).0)
            }
            // No exact-rational oracle for exp/sinh/cosh: sin/cos get
            // one specifically because the tangent-half-angle
            // parametrization (`cos_sin_of_t`) makes `sin`/`cos` of a
            // RATIONAL parameter exactly rational, satisfying the
            // identity by construction. There is no analogous trick for
            // a genuinely transcendental function of a rational argument
            // (`exp`/`sinh`/`cosh` of a nonzero rational is essentially
            // never rational) -- so `arb_expr` below deliberately never
            // generates these three, and this arm is never actually
            // exercised by the fuzzer. `None` (not a float
            // approximation, and not `unreachable!()`) is still the
            // right answer if that ever changes without this arm being
            // revisited: the same "can't evaluate here" meaning already
            // used for a pole on either side (see `values_agree`'s own
            // comment), never a silent wrong answer.
            // Same reasoning as Exp/Sinh/Cosh above, one step further: an
            // indeterminate function has no value at all, exact or
            // otherwise -- `arb_expr` below never generates one, so this
            // arm is likewise never actually exercised.
            Expr::Exp(_) | Expr::Sinh(_) | Expr::Cosh(_) | Expr::Func { .. } => None,
        }
    }

    /// Exact rational sample points -- none of them small integers, same
    /// reasoning as before (this generator's leaves are `-5..=5`, so an
    /// integer point is the most likely to accidentally land on a
    /// removable singularity both sides happen to share), plus a
    /// distinct `T_POOL` starting offset per point so the *trig* values
    /// vary across points too, not just the plain variables.
    fn sample_points() -> Vec<(Vec<(&'static str, BigScalar)>, usize)> {
        vec![
            (vec![("x", BigScalar::new(13, 10)), ("y", BigScalar::new(-27, 10)), ("M", BigScalar::new(6, 10)), ("r", BigScalar::new(31, 10))], 0),
            (vec![("x", BigScalar::new(-42, 10)), ("y", BigScalar::new(11, 10)), ("M", BigScalar::new(29, 10)), ("r", BigScalar::new(-8, 10))], 3),
            (vec![("x", BigScalar::new(4, 10)), ("y", BigScalar::new(55, 10)), ("M", BigScalar::new(-16, 10)), ("r", BigScalar::new(22, 10))], 6),
        ]
    }

    /// Both sides agree, exactly, at every sample point where both are
    /// defined (a point landing on a pole either side's *own*
    /// denominator has isn't a disagreement -- both engines are still
    /// correct there, `Expr` just doesn't evaluate a pole). No tolerance
    /// anywhere: exact rational equality, or an exact pole on one or
    /// both sides.
    fn values_agree(a: &Expr, b: &Expr) -> Result<(), String> {
        for (point, seed_offset) in sample_points() {
            let mut trig = TrigMemo::new(seed_offset);
            let va = eval_exact(a, &point, &mut trig);
            let vb = eval_exact(b, &point, &mut trig);
            if let (Some(va), Some(vb)) = (va, vb) {
                if va != vb {
                    return Err(format!("at {point:?}: {va:?} != {vb:?}"));
                }
            }
        }
        Ok(())
    }

    use proptest::prelude::*;

    /// Deliberately never generates `Exp`/`Sinh`/`Cosh`: this whole
    /// differential oracle (`v1_and_v2_agree`/`v2_is_idempotent` below)
    /// depends on `eval_exact` being able to evaluate every generated
    /// tree to an EXACT rational at each sample point, which only works
    /// for `sin`/`cos` because of the tangent-half-angle parametrization
    /// trick (`TrigMemo`) -- there is no equivalent for a genuinely
    /// transcendental function, so adding them here would need a new
    /// evaluation scheme this crate does not have, not a one-line
    /// addition. Correctness for the three new functions instead comes
    /// from the targeted, real-curvature tests next to their own
    /// implementation (`oderom-components` fixtures), per the scope
    /// decided for that round.
    fn arb_expr() -> impl Strategy<Value = Expr> {
        let leaf = prop_oneof![
            (-5i64..=5).prop_map(Expr::int),
            prop_oneof![Just("x"), Just("y"), Just("M"), Just("r")].prop_map(Expr::var),
        ];
        leaf.prop_recursive(4, 32, 3, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 2..=3).prop_map(Expr::Add),
                proptest::collection::vec(inner.clone(), 2..=3).prop_map(Expr::Mul),
                // Clamped to +-1 when `b` is itself a `cos(...)`: `poly_gcd`
                // no longer mishandles cos^2-of-the-same-argument (fixed,
                // `TrigRewriteSuppressor` above) -- this clamp is
                // deliberate defense in depth, not a workaround for a
                // known-broken case, so that routine fuzzing runs spend
                // their budget on the far larger space of *other* shapes
                // instead of repeatedly re-exercising this one already-hard
                // (and now separately regression-tested, see
                // `cos_squared_of_the_same_argument_inside_a_gcd_used_to_break_subresultant_prs`
                // below) code path on every run.
                (inner.clone(), -3i32..=3).prop_map(|(b, n)| {
                    let n = if matches!(b, Expr::Cos(_)) { n.clamp(-1, 1) } else { n };
                    if n == 0 { Expr::one() } else { Expr::Pow(Box::new(b), n) }
                }),
                inner.clone().prop_map(|e| e.sin()),
                inner.prop_map(|e| e.cos()),
            ]
        })
    }

    /// Regression test for the exact minimal case property-based fuzzing
    /// found (`proptest-regressions/normalize.txt`, seed
    /// `672e22ba5716...`): squaring a leading coefficient that contains
    /// `cos(0)` mid-`poly_gcd` used to trigger D-RF.7's `cos^2 ->
    /// 1-sin^2` rewrite *inside* subresultant PRS, changing the
    /// coefficient ring out from under the algorithm's own degree
    /// bookkeeping and leaving a genuinely non-exact `beta_i` division
    /// (verified independently outside Rust: exact in the free ring
    /// Q[cos(t)][x], not exact once cos^2 gets reduced mid-computation).
    /// Fixed by `TrigRewriteSuppressor` (DESIGN-RATIONAL-FORM.md section
    /// 6): the rewrite is suppressed for `poly_gcd`'s entire recursive
    /// descent and reapplied exactly once, via `Poly::normalize_trig`, on
    /// its final result.
    #[test]
    fn cos_squared_of_the_same_argument_inside_a_gcd_used_to_break_subresultant_prs() {
        let e = Expr::Add(vec![
            Expr::int(0).cos(),
            Expr::Pow(Box::new(Expr::Add(vec![Expr::var("r"), Expr::var("x")])), -3),
            Expr::Add(vec![Expr::var("r"), Expr::int(-1)]),
        ]);
        let _ = normalize(&e);
    }

    proptest! {
        // 256 (proptest's default) comprovadamente deixa bug passar: a
        // regressão `(0^-1)^-1` (reduce_inner's constant-denominator fold
        // missing a `den.is_zero()` guard, silently turning 0^-1 into 1)
        // só apareceu rodando manualmente com PROPTEST_CASES=4000 --
        // nunca teria sido pega no default. Estes dois testes são os
        // únicos invariantes do motor novo (concordância por valor com
        // o legado, idempotência), não dois entre muitos, e são baratos
        // (a suíte inteira do workspace continua rodando em segundos com
        // isso ligado) -- sem motivo para economizar aqui. Mesma
        // convenção de `oderom-canon/tests/prop_canon.rs`.
        // `fork` + `timeout`, added after `v2_is_idempotent` drew an
        // input on which `normalize` does not terminate and spun 5h37m
        // at 100% CPU inside a full-workspace run, producing no output
        // and losing the input: proptest persists *failures*, and a hang
        // is not one. Each case now runs in a child process that is
        // killed from outside when it overruns, which turns the hang
        // into a reported failure -- so shrinking runs, the expression
        // is printed, and the seed lands in `proptest-regressions/`.
        //
        // Killing from outside is the point. `check_cancelled` exists
        // only in `rational_function.rs`; `normalize`'s own rewrite loop
        // has no checkpoint, so cancellation-based interruption is not
        // known to reach the loop that spins.
        //
        // Caveat worth knowing when one of these fires: proptest reports
        // a timeout as `failed in other process`, the same wording it
        // uses for a child that crashed. The two are not distinguishable
        // from the message alone.
        // The ceiling is per profile, and that is a declared choice.
        //
        // A single fixed number cannot mean the same thing in both:
        // `(r^-3 + M + x + 1)^3` -- the input the generator drew that
        // started this -- costs 3.18s in release and ~29s in debug, a
        // measured 9x. Five seconds in debug is about half a second of
        // real work, which misclassifies merely-slow inputs as broken;
        // five seconds in release is the bound we actually want. So the
        // debug ceiling is scaled rather than shared.
        //
        // What a failure here means: the case did not finish in time.
        // It does NOT mean non-termination. That distinction cost 5h37m
        // of wall clock and two rounds of wrong diagnosis -- the first
        // report of this called it a hang, and it was slowness.
        //
        // No wall-clock assertion is made about any specific input.
        // Timing assertions have produced four false alarms in this
        // project (see DESIGN-RATIONAL-FORM.md); the ceiling exists to
        // bound suite runtime, not to police performance.
        #![proptest_config(ProptestConfig {
            cases: 10_000,
            fork: true,
            timeout: if cfg!(debug_assertions) { 45_000 } else { 5_000 },
            ..ProptestConfig::default()
        })]

        #[test]
        fn v1_and_v2_agree(e in arb_expr()) {
            let v1 = normalize_v1(&e);
            let v2 = normalize(&e);
            if let Err(msg) = values_agree(&v1, &v2) {
                prop_assert!(false, "value disagreement normalizing {:?}: {msg} (v1={:?} v2={:?})", e, v1, v2);
            }
        }

        /// Both engines must be idempotent by VALUE -- normalizing an
        /// already normalized expression must not change what it
        /// evaluates to (structural idempotence is not guaranteed to the
        /// same degree: e.g. a sign-convention choice between `1-x` and
        /// `-1*(x-1)`-shaped results can differ round to round without
        /// either being wrong).
        #[test]
        fn v2_is_idempotent(e in arb_expr()) {
            // M2: which of the two calls spins? The distinction decides
            // the diagnosis. If the first hangs, the defect has nothing
            // to do with idempotence -- this test is merely where it
            // surfaced, being a property over `arb_expr()`, and any such
            // property can hit it. If the second hangs, there is a real
            // cycle between two normal forms.
            //
            // Written to stderr *before* each call, so the last line the
            // killed child emitted names the call that did not return.
            eprintln!("v2_is_idempotent: entering FIRST normalize on {e:?}");
            let once = normalize(&e);
            eprintln!("v2_is_idempotent: FIRST returned; entering SECOND normalize on {once:?}");
            let twice = normalize(&once);
            eprintln!("v2_is_idempotent: SECOND returned");
            if let Err(msg) = values_agree(&once, &twice) {
                prop_assert!(false, "value changed on re-normalizing {:?}: {msg} (once={:?} twice={:?})", e, once, twice);
            }
        }
    }
}
