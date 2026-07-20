//! Putting an expression over a single, explicit numerator/denominator
//! pair -- what a real CAS calls `together()`.
//!
//! [`normalize`] reduces by local rewriting (fold constants, collect like
//! terms/bases, distribute a positive power of a sum) applied repeatedly
//! to a fixed point. That is enough for expressions built around a single
//! recurring "denominator sum" (Marco 2's Kretschmann scalar has only
//! `1 - 2M/r` playing that role throughout), but it cannot reliably
//! reduce an expression built from *several* independent sums at once:
//! Marco 3's metric-pullback check squares a metric's conformal factor
//! after a chart transition, producing `(1+R)^-2 * R^2` for two unrelated
//! sums `R` and `1+R`. `normalize` has no way to know, before trying,
//! whether expanding `R^2` will ever pay off -- and here it doesn't
//! (there's no negative power of `R` for it to cancel against, only the
//! unrelated `(1+R)^-2`), so expanding it just glues raw monomials to
//! `(1+R)^-2` as a shared factor, permanently hiding the very structure a
//! later pass would have needed to simplify further. A version of
//! `normalize` that refused to distribute unless its sum was the only one
//! in the product fixed exactly this case and broke Kretschmann, which
//! *needs* to distribute one sum while an unrelated one is also present
//! elsewhere in the same product. The two needs are in genuine tension
//! for a rewrite system with no memory of *why* a base is opaque.
//!
//! [`rationalize`] sidesteps the tension by not trying to discover the
//! numerator/denominator split through pattern-matching at all: it
//! *carries* the split explicitly through a single top-down recursion
//! (`a/b + c/d = (ad+bc)/(bd)`, `(a/b)*(c/d) = ac/bd`, `(a/b)^n =
//! a^n/b^n`), so which sums are "the denominator" is always known by
//! construction, never re-inferred from an already-mixed expression.

use crate::normalize::normalize;
use crate::Expr;

/// `expr == numerator/denominator`, with both fully expanded (via
/// [`normalize`], which internally cancels a shared power between a sum
/// and its own reciprocal whenever it recognizes one -- something a
/// numerator/denominator built up via straightforward fraction arithmetic
/// hits far more reliably than an expression assembled by unrelated
/// multiplications and additions).
pub fn rationalize(expr: &Expr) -> (Expr, Expr) {
    to_fraction(&normalize(expr))
}

fn to_fraction(expr: &Expr) -> (Expr, Expr) {
    match expr {
        Expr::Rational(_) | Expr::Var(_) | Expr::Sin(_) | Expr::Cos(_) => (expr.clone(), Expr::one()),
        Expr::Add(terms) => {
            let mut num = Expr::zero();
            let mut den = Expr::one();
            for t in terms {
                let (tn, td) = to_fraction(t);
                let new_num = normalize(&(num * td.clone() + tn * den.clone()));
                let new_den = normalize(&(den * td));
                num = new_num;
                den = new_den;
            }
            (num, den)
        }
        Expr::Mul(factors) => {
            let mut num = Expr::one();
            let mut den = Expr::one();
            for f in factors {
                let (fnum, fden) = to_fraction(f);
                num = normalize(&(num * fnum));
                den = normalize(&(den * fden));
            }
            (num, den)
        }
        Expr::Pow(base, n) => {
            let (bn, bd) = to_fraction(base);
            if *n >= 0 {
                (normalize(&pow_by_repeated_mul(bn, *n)), normalize(&pow_by_repeated_mul(bd, *n)))
            } else {
                (normalize(&pow_by_repeated_mul(bd, -n)), normalize(&pow_by_repeated_mul(bn, -n)))
            }
        }
    }
}

fn pow_by_repeated_mul(base: Expr, n: i32) -> Expr {
    let mut acc = Expr::one();
    for _ in 0..n {
        acc = acc * base.clone();
    }
    acc
}

/// The polynomial degree of `e`'s denominator once `e` is put over one
/// fraction by [`rationalize`].
///
/// Used as a second guardrail (`oderom-cli`'s `--max-denominator-degree`)
/// alongside the wall-clock timeout, precisely because plain node count
/// provably does not catch the Reissner-Nordstrom blowup (see
/// `oderom-components/tests/diagnostic_rn.rs`): the failing computation's
/// raw *tree* never gets large, but `rationalize`'s own `den = den * td`
/// never cancels a shared factor against a matching negative power (no
/// GCD step exists yet -- that is what DESIGN-RATIONAL-FORM.md proposes
/// adding), so the denominator's *degree* grows every time another
/// unreduced term is folded in, well before the tree itself looks
/// alarming by size alone.
pub fn denominator_degree(e: &Expr) -> i32 {
    degree(&rationalize(e).1)
}

/// The standard recursive definition of polynomial degree (a sum's
/// degree is the max of its terms', a product's is the sum of its
/// factors', `base^n` is `|n|` times `base`'s), extended to treat
/// `sin`/`cos` as degree-1 atoms like any variable -- consistent with
/// [`crate::normalize`] never expanding or relating them by identity.
fn degree(e: &Expr) -> i32 {
    match e {
        Expr::Rational(_) => 0,
        Expr::Var(_) | Expr::Sin(_) | Expr::Cos(_) => 1,
        Expr::Add(terms) => terms.iter().map(degree).max().unwrap_or(0),
        Expr::Mul(factors) => factors.iter().map(degree).sum(),
        Expr::Pow(base, n) => n.unsigned_abs() as i32 * degree(base),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rationalizes_a_single_fraction() {
        let x = Expr::var("x");
        let e = Expr::int(3) * Expr::Pow(Box::new(x), -1);
        let (num, den) = rationalize(&e);
        assert_eq!(num, Expr::int(3));
        assert_eq!(den, Expr::var("x"));
    }

    #[test]
    fn combines_two_fractions_with_different_denominators() {
        // 1/x + 1/y = (x+y)/(xy)
        let x = Expr::var("x");
        let y = Expr::var("y");
        let e = Expr::Pow(Box::new(x.clone()), -1) + Expr::Pow(Box::new(y.clone()), -1);
        let (num, den) = rationalize(&e);
        assert_eq!(num, normalize(&(x.clone() + y.clone())));
        assert_eq!(den, normalize(&(x * y)));
    }

    #[test]
    fn handles_two_independent_sums_multiplied_together() {
        // (1+x)^-2 * x^2 -- the exact shape that broke local rewriting.
        // The property that matters: neither the numerator nor the
        // denominator has any negative exponent left in it (a positive
        // exponent, e.g. `(1+x)^2` in the denominator, is fine -- it's
        // just not a *fraction* anymore).
        let x = Expr::var("x");
        let f = Expr::one() + x.clone();
        let e = Expr::Pow(Box::new(f), -2) * x.pow(2);
        let (num, den) = rationalize(&e);
        assert!(!has_negative_exponent(&num), "{num:?}");
        assert!(!has_negative_exponent(&den), "{den:?}");
    }

    #[test]
    fn denominator_degree_of_a_simple_power() {
        let r = Expr::var("r");
        assert_eq!(denominator_degree(&Expr::Pow(Box::new(r.clone()), -1)), 1);
        assert_eq!(denominator_degree(&Expr::Pow(Box::new(r), -4)), 4);
    }

    #[test]
    fn denominator_degree_grows_when_denominators_do_not_unify() {
        // Distinct-but-related sums (not the literal same Add node)
        // don't hit normalize's single-common-denominator fast path --
        // this is the actual shape that blows up
        // (combine_over_common_denominators's own comment: "two or more
        // distinct denominator sums... left uncombined"), unlike adding
        // several copies of the *same* fraction, which that fast path
        // already handles today.
        let r = Expr::var("r");
        let f = |k: i64| Expr::one() - Expr::int(k) * Expr::var("M") / r.clone();
        let one_term = Expr::Pow(Box::new(f(2)), -1);
        let four_terms = Expr::Pow(Box::new(f(2)), -1)
            + Expr::Pow(Box::new(f(3)), -1)
            + Expr::Pow(Box::new(f(4)), -1)
            + Expr::Pow(Box::new(f(5)), -1);

        let d1 = denominator_degree(&one_term);
        let d4 = denominator_degree(&four_terms);
        assert!(d4 > d1, "expected degree to grow when denominators don't unify: d1={d1}, d4={d4}");
    }

    fn has_negative_exponent(e: &Expr) -> bool {
        match e {
            Expr::Rational(_) | Expr::Var(_) => false,
            Expr::Add(terms) | Expr::Mul(terms) => terms.iter().any(has_negative_exponent),
            Expr::Pow(base, n) => *n < 0 || has_negative_exponent(base),
            Expr::Sin(inner) | Expr::Cos(inner) => has_negative_exponent(inner),
        }
    }
}
