//! Rewriting to a normal form: flatten associative `Add`/`Mul`, fold
//! rational constants, and collect like terms (`Add`) / like bases
//! (`Mul`, by summing exponents of matching bases) by structural
//! equality once children are canonically ordered (see [`crate::Expr`]'s
//! `Ord` impl).
//!
//! Deliberately **not included**: distributing a product over a sum
//! (`a*(b+c) -> a*b + a*c`) or expanding a positive integer power of a
//! sum (`(a+b)^2 -> a^2+2ab+b^2`). A first version did both, and it broke
//! exactly the cancellation this normalizer exists for: `Mul` groups
//! factors by base and sums their exponents treating *any* expression as
//! a base -- including a bare sum -- so `f * f^-1` (`f` an `Add`)
//! collapses to `1` for free, without ever expanding `f`. Distributing
//! `f` eagerly replaces it with its terms *before* that cancellation can
//! see it, permanently hiding the `f^-1` it should have met. Since the
//! rational functions Christoffel/Riemann/Ricci computations produce
//! don't need polynomial expansion to reach a single normal form (their
//! sums combine additively, not by multiplying out binomials), the
//! smaller, non-expanding normalizer is both simpler and correct where
//! the expanding one wasn't. If a future computation genuinely needs
//! expansion, it belongs as an explicit, opt-in step at the call site,
//! not folded back into this general-purpose pass.
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

fn simplify_add(terms: Vec<Expr>) -> Expr {
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

    let mut out: Vec<Expr> = grouped
        .into_iter()
        .filter(|(_, c)| !c.is_zero())
        .map(|(rest, coeff)| scale(coeff, rest))
        .collect();
    out.sort();

    match out.len() {
        0 => Expr::zero(),
        1 => out.into_iter().next().expect("checked len==1"),
        _ => Expr::Add(out),
    }
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
        *bases.entry(base).or_insert(0) += exp;
    }

    if coeff.is_zero() {
        return Expr::zero();
    }

    let mut out: Vec<Expr> = bases
        .into_iter()
        .filter(|(_, e)| *e != 0)
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
    fn does_not_distribute_multiplication_over_addition() {
        // 2*(x+3) is left alone, not expanded to 2x+6 -- see the module
        // docs for why forced distribution was removed.
        let x = Expr::var("x");
        let e = Expr::int(2) * (x.clone() + Expr::int(3));
        let result = normalize(&e);
        // Still a product containing a sum, not expanded to 2x + 6.
        assert!(matches!(&result, Expr::Mul(factors) if factors.iter().any(|f| matches!(f, Expr::Add(_)))));
        let expanded = Expr::int(2) * x + Expr::int(6);
        assert_ne!(result, normalize(&expanded));
    }

    #[test]
    fn a_bare_sum_still_cancels_its_own_reciprocal_power() {
        // f * f^-1 = 1 even though f is never expanded (see
        // cancels_reciprocal_of_a_sum for the same mechanism with the
        // factors reversed).
        let x = Expr::var("x");
        let f = x + Expr::one();
        let e = Expr::Pow(Box::new(f.clone()), -1) * f;
        assert_eq!(normalize(&e), Expr::one());
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
