//! The new `normalize()` engine (DESIGN-RATIONAL-FORM.md): `Expr` goes
//! in, gets converted to a [`RationalFunction`] over one per-call
//! [`AtomTable`] (D-RF.5 -- never a global), all the algebra (including
//! GCD reduction) happens there, and the result converts back to `Expr`.
//! `crate::normalize::normalize`'s public signature is unchanged; this
//! is purely what it does internally now.

use crate::poly::{AtomTable, Poly, Term};
use crate::rational_function::RationalFunction;
use crate::Expr;
use crate::BigScalar;

thread_local! {
    static NORMALIZE_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Decrements `NORMALIZE_DEPTH` on drop, not just on the function's
/// normal return path -- `check_cancelled()` below can unwind out of
/// this function via panic, and without this guard that unwind would
/// skip the decrement and leave the depth counter stuck incremented for
/// whatever runs next on this thread.
struct DepthGuard;
impl Drop for DepthGuard {
    fn drop(&mut self) {
        NORMALIZE_DEPTH.with(|d| d.set(d.get() - 1));
    }
}

pub(crate) fn normalize_via_rational_form(e: &Expr) -> Expr {
    // Checked on every call, including the recursive ones for sin/cos
    // argument canonicalization below -- this is the single entry point
    // every simplification in the crate funnels through, so this alone
    // gives far finer granularity than the per-component checkpoint
    // above it (`oderom-components::curvature`'s `_checkpointed`
    // variants): a component whose *own* `normalize()` call runs long
    // now yields here long before that call would otherwise return.
    crate::cancel::check_cancelled();
    // `normalize_via_rational_form` recurses for sin/cos argument
    // canonicalization (see `expr_to_rational` below) -- only the true
    // outermost call resets/prints the `ODEROM_REDUCE_STATS` counters
    // (rational_function.rs, big_scalar.rs), matching "per component"
    // (one top-level `normalize()` call), not "per sin/cos argument
    // nested inside it".
    let depth = NORMALIZE_DEPTH.with(|d| {
        let n = d.get() + 1;
        d.set(n);
        n
    });
    let _depth_guard = DepthGuard;
    if depth == 1 && std::env::var("ODEROM_REDUCE_STATS").is_ok() {
        crate::rational_function::reset_reduce_stats();
        crate::big_scalar::reset_arith_stats();
    }
    let mut table = AtomTable::new();
    let rf = expr_to_rational(e, &mut table);
    let result = rational_to_expr(&rf, &table);
    if depth == 1 && std::env::var("ODEROM_REDUCE_STATS").is_ok() {
        let (total, gcd_branch, literal_one) = crate::rational_function::reduce_stats();
        let (arith_count, arith_nanos) = crate::big_scalar::arith_stats();
        eprintln!(
            "ODEROM_REDUCE_STATS: reduce() calls={total} gcd_branch={gcd_branch} literal_one={literal_one} ({:.1}% of gcd_branch found nothing to cancel) | BigScalar add+mul ops={arith_count} time={:?} ({:.1}ns/op avg)",
            if gcd_branch > 0 { 100.0 * literal_one as f64 / gcd_branch as f64 } else { 0.0 },
            std::time::Duration::from_nanos(arith_nanos),
            if arith_count > 0 { arith_nanos as f64 / arith_count as f64 } else { 0.0 }
        );
    }
    result
}

pub(crate) fn expr_to_rational(e: &Expr, table: &mut AtomTable) -> RationalFunction {
    match e {
        Expr::Rational(s) => RationalFunction::from_poly(Poly::constant(s.clone())),
        Expr::Var(name) => RationalFunction::from_poly(Poly::generator(table.var(name))),
        Expr::Add(terms) => {
            // Accumulate the whole sum over one common denominator via
            // raw Poly cross-multiplication, reducing exactly once at the
            // end -- not once per term (DESIGN-RATIONAL-FORM.md probe:
            // does avoiding a chain of N `reduce()` calls, each handing
            // its own leftover content to the next, avoid the cross-call
            // content-carryover growth directly, no GCD-in-the-
            // coefficient-ring needed at all?).
            let mut num_acc = Poly::zero();
            let mut den_acc = Poly::constant(BigScalar::one());
            for t in terms {
                let rf = expr_to_rational(t, table);
                let new_num = num_acc.mul(&rf.den, table).add(&rf.num.mul(&den_acc, table));
                let new_den = den_acc.mul(&rf.den, table);
                num_acc = new_num;
                den_acc = new_den;
            }
            RationalFunction::from_raw(num_acc, den_acc, table)
        }
        Expr::Mul(factors) => factors
            .iter()
            .fold(RationalFunction::from_poly(Poly::constant(BigScalar::one())), |acc, f| {
                acc.mul(&expr_to_rational(f, table), table)
            }),
        Expr::Pow(base, n) => expr_to_rational(base, table).pow(*n, table),
        // D-RF.6: the argument is canonicalized by recursing into this
        // same engine (a strictly smaller subtree, so this terminates)
        // *before* interning, so two structurally different but equal
        // arguments (e.g. built via different paths upstream) always
        // land on the same AtomId -- required for sin/cos collection to
        // work at all, not an incidental nicety.
        Expr::Sin(arg) => {
            let canonical_arg = normalize_via_rational_form(arg);
            let id = table.sin(canonical_arg);
            RationalFunction::from_poly(Poly::generator(id))
        }
        Expr::Cos(arg) => {
            let canonical_arg = normalize_via_rational_form(arg);
            let id = table.cos(canonical_arg);
            RationalFunction::from_poly(Poly::generator(id))
        }
        // Same D-RF.6 treatment as Sin/Cos above: argument canonicalized
        // by recursion before interning. Unlike Sin/Cos, `exp` gets no
        // cross-atom identity (`exp(a)*exp(b) = exp(a+b)` is deliberately
        // NOT canonicalized -- see AtomTable::exp's own doc comment for
        // why).
        Expr::Exp(arg) => {
            let canonical_arg = normalize_via_rational_form(arg);
            let id = table.exp(canonical_arg);
            RationalFunction::from_poly(Poly::generator(id))
        }
        // `sinh`/`cosh` get the same cross-atom treatment Sin/Cos's own
        // D-RF.7 pythagorean identity does, generalized: `cosh(arg)^2 ->
        // 1 + sinh(arg)^2` inside `poly.rs`'s `mul_term`/`normalize_trig`
        // (never the other way around -- `cosh` is preferred, matching
        // how `cos` -- not `sin` -- is the one reduced today).
        Expr::Sinh(arg) => {
            let canonical_arg = normalize_via_rational_form(arg);
            let id = table.sinh(canonical_arg);
            RationalFunction::from_poly(Poly::generator(id))
        }
        Expr::Cosh(arg) => {
            let canonical_arg = normalize_via_rational_form(arg);
            let id = table.cosh(canonical_arg);
            RationalFunction::from_poly(Poly::generator(id))
        }
        // Same D-RF.6 treatment as Sin/Cos/Exp/Sinh/Cosh above: every
        // argument canonicalized by recursion before interning, so two
        // structurally different but equal argument lists land on the
        // same AtomId. Unlike those, no cross-atom identity exists for
        // an indeterminate function at all (DESIGN-M6-PREP.md section 1)
        // -- this atom is, algebraically, even simpler than `exp`.
        Expr::Func { name, args, order } => {
            let canonical_args: Vec<Expr> = args.iter().map(normalize_via_rational_form).collect();
            let id = table.func(name.clone(), canonical_args, order.clone());
            RationalFunction::from_poly(Poly::generator(id))
        }
    }
}

fn rational_to_expr(rf: &RationalFunction, table: &AtomTable) -> Expr {
    let num = poly_to_expr(&rf.num, table);
    // Division by literal zero is left opaque, same as the old engine's
    // scalar_pow ("0^negative: leave opaque rather than guess") --
    // `reduce` no longer tries to invert a zero denominator (see its own
    // docs), so this reconstructs `num * 0^-1` unevaluated rather than
    // asserting it can never happen.
    if rf.den.is_zero() {
        let zero_pow = Expr::Pow(Box::new(Expr::zero()), -1);
        return if num == Expr::one() { zero_pow } else { num * zero_pow };
    }
    let is_one_den = {
        let terms = rf.den.sorted_terms(table);
        terms.len() == 1 && terms[0].coeff == BigScalar::one() && terms[0].generators.is_empty()
    };
    if is_one_den {
        num
    } else {
        let den = poly_to_expr(&rf.den, table);
        let den_pow = Expr::Pow(Box::new(den), -1);
        // Same "omit the redundant `* 1`" rule as the zero-denominator
        // branch above -- this general branch needs it too (found via
        // `v1_and_v2_agree`: `Cos(0 + x^-1)` normalized to
        // `Cos(Mul([1, x^-1]))` instead of bare `Cos(Pow(x,-1))`, a
        // structural mismatch against the old engine's exact output
        // shape for this case).
        if num == Expr::one() {
            den_pow
        } else {
            num * den_pow
        }
    }
}

fn poly_to_expr(p: &Poly, table: &AtomTable) -> Expr {
    let terms = p.sorted_terms(table);
    if terms.is_empty() {
        return Expr::zero();
    }
    let add_terms: Vec<Expr> = terms.iter().map(|t| term_to_expr(t, table)).collect();
    if add_terms.len() == 1 {
        add_terms.into_iter().next().expect("checked len==1")
    } else {
        Expr::Add(add_terms)
    }
}

fn term_to_expr(term: &Term, table: &AtomTable) -> Expr {
    let mut generators = term.generators.clone();
    generators.sort_by_key(|a| table.to_expr(a.0));

    let mut factors: Vec<Expr> = Vec::new();
    if term.coeff != BigScalar::one() || generators.is_empty() {
        factors.push(Expr::Rational(term.coeff.clone()));
    }
    for (id, exp) in generators {
        let base = table.to_expr(id);
        factors.push(if exp == 1 { base } else { Expr::Pow(Box::new(base), exp as i32) });
    }
    if factors.len() == 1 {
        factors.into_iter().next().expect("checked len==1")
    } else {
        Expr::Mul(factors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_pole_variable_with_two_denominator_degrees_is_fine() {
        // Positive control for the regression test below: with only ONE
        // free variable (r) ever acting as a coefficient-field element,
        // adding two monomials of different denominator degree in r
        // reduces cleanly and quickly -- establishing that the bug next
        // door needs a second variable (M) to manifest.
        let r = Expr::var("r");
        let t1 = Expr::int(-1536) * r.clone().pow(-9);
        let t2 = Expr::int(-384) * r.pow(-7);
        let _ = normalize_via_rational_form(&(t1 + t2));
    }

    /// Substitutes plain `f64` values for every `Var` in a fully-reduced
    /// `Expr` and evaluates it -- used below to check a reduction's
    /// *value* is right without depending on exactly how its terms are
    /// ordered.
    fn eval(e: &Expr, vars: &[(&str, f64)]) -> f64 {
        match e {
            Expr::Rational(s) => s.to_f64_lossy(),
            Expr::Var(name) => vars.iter().find(|(n, _)| *n == name).map(|(_, v)| *v).expect("unbound var"),
            Expr::Add(terms) => terms.iter().map(|t| eval(t, vars)).sum(),
            Expr::Mul(factors) => factors.iter().map(|f| eval(f, vars)).product(),
            Expr::Pow(base, n) => eval(base, vars).powi(*n),
            Expr::Sin(arg) => eval(arg, vars).sin(),
            Expr::Cos(arg) => eval(arg, vars).cos(),
            Expr::Exp(arg) => eval(arg, vars).exp(),
            Expr::Sinh(arg) => eval(arg, vars).sinh(),
            Expr::Cosh(arg) => eval(arg, vars).cosh(),
            Expr::Func { name, .. } => panic!("eval() (test-only numeric oracle) has no value for indeterminate function `{name}` -- no test in this module constructs one"),
        }
    }

    #[test]
    fn reduce_terminates_summing_two_monomials_in_two_variables() {
        // The true minimal reproducer for a since-fixed non-termination
        // bug (DESIGN-RATIONAL-FORM.md): no hidden algebraic cancellation
        // needed (contrast the Riemann-derived case below) -- just TWO
        // monomials, in TWO variables, with different powers of both,
        // added together. This is literally the first two terms of the
        // Kretschmann-scalar numerator that motivated this whole engine.
        // Root cause (fixed): coefficients (the M-ring) were themselves
        // `RationalFunction`s that could recurse back into `reduce()` and
        // pick a *different* pole variable, with no measure that ever
        // strictly decreased -- traced directly as pole choice
        // ping-ponging `r -> M -> r -> M -> ...` across recursion depth,
        // unbounded. Fixed by making the coefficient ring a plain `Poly`
        // (no denominator field, no `reduce()`, no pole selection at all
        // -- see rational_function.rs's module doc comment) rather than
        // patching the old recursive shape.
        let m = Expr::var("M");
        let r = Expr::var("r");
        let t1 = Expr::int(-1536) * m.clone().pow(5) * r.clone().pow(-9);
        let t2 = Expr::int(-384) * m.pow(3) * r.pow(-7);
        let result = normalize_via_rational_form(&(t1 + t2));
        let expected = -1536.0 * 2.0f64.powi(5) / 3.0f64.powi(9) + -384.0 * 2.0f64.powi(3) / 3.0f64.powi(7);
        assert!((eval(&result, &[("M", 2.0), ("r", 3.0)]) - expected).abs() < 1e-9, "{result:?}");
    }

    #[test]
    fn debug_pow4() {
        let m = Expr::var("M");
        let r = Expr::var("r");
        let f = Expr::one() - Expr::int(2) * m / r;
        let e = Expr::Pow(Box::new(f), 4);
        let result = normalize_via_rational_form(&e);
        eprintln!("result = {:?}", result);
        eprintln!("node_count = {}", result.node_count());
    }

    #[test]
    #[ignore] // TEMPORARY: doesn't terminate yet (subresultant PRS work in progress); run explicitly.
    fn debug_four_term_probe_pow4() {
        // The four-term probe from the test plan: 1 - 2M/r + Q^2/r^2 -
        // L^2/r^3, raised to the 4th power (same shape as debug_pow4
        // above, three parameters instead of one).
        let m = Expr::var("M");
        let q = Expr::var("Q");
        let l = Expr::var("L");
        let r = Expr::var("r");
        let f = Expr::one() - Expr::int(2) * m * r.clone().pow(-1) + q.pow(2) * r.clone().pow(-2)
            - l.pow(2) * r.pow(-3);
        let e = Expr::Pow(Box::new(f), 4);
        let t0 = std::time::Instant::now();
        let result = normalize_via_rational_form(&e);
        eprintln!("elapsed = {:?}", t0.elapsed());
        eprintln!("node_count = {}", result.node_count());
    }

    #[test]
    fn reduce_finds_a_hidden_denominator_cancellation() {
        // Regression test for a since-fixed non-termination bug
        // (DESIGN-RATIONAL-FORM.md): distilled from the Schwarzschild
        // riemann[1,0,0,1] component, whose full expression the old
        // (legacy_v1) engine reduces to a plain polynomial, -4M^2/r^4 +
        // 2M/r^3, with NO surviving (1-2M/r) denominator at all -- i.e.
        // `a * b` below algebraically collapses to M^2/r^4, since
        // `b == M/r^2 * (1 - 2M/r)` is exactly `a`'s `(1-2M/r)^-1` factor's
        // reciprocal, times M/r^2. Before the two-type redesign, finding
        // this required the coefficient ring to recurse back into
        // `reduce()`, which never converged (recursion depth grew with no
        // plateau, past 900 levels before the real stack overflowed).
        let m = Expr::var("M");
        let r = Expr::var("r");
        let f_inv = (Expr::one() - Expr::int(2) * m.clone() * r.clone().pow(-1)).pow(-1);
        let a = m.clone() * r.clone().pow(-2) * f_inv;
        let b = Expr::int(-2) * m.clone().pow(2) * r.clone().pow(-3) + m * r.pow(-2);
        let val = a * b;
        let result = normalize_via_rational_form(&val);
        let expected = 4.0f64.powi(2) / 3.0f64.powi(4); // M^2/r^4 at M=4, r=3
        assert!((eval(&result, &[("M", 4.0), ("r", 3.0)]) - expected).abs() < 1e-9, "{result:?}");
    }

    #[test]
    fn roundtrips_a_simple_expression() {
        let e = Expr::int(2) * Expr::var("x") + Expr::int(3);
        let result = normalize_via_rational_form(&e);
        // 2x + 3, some canonical order -- just check it evaluates the
        // same shape (two Add terms) rather than assume an exact order.
        match &result {
            Expr::Add(terms) => assert_eq!(terms.len(), 2, "{result:?}"),
            other => panic!("expected an Add, got {other:?}"),
        }
    }

    #[test]
    fn cancels_a_simple_reciprocal() {
        let x = Expr::var("x");
        let e = x.clone() * Expr::Pow(Box::new(x), -1);
        assert_eq!(normalize_via_rational_form(&e), Expr::one());
    }

    #[test]
    fn collects_sin_squared_across_separately_built_terms() {
        // sin(theta)^2 * A + sin(theta)^2 * B, A and B distinct
        // rationals -- must collect into one term (D-RF.7's concern
        // about sin/cos being real generators, not opaque subtrees).
        let theta = Expr::var("theta");
        let sin_sq = theta.sin().pow(2);
        let e = Expr::int(2) * sin_sq.clone() + Expr::int(3) * sin_sq;
        let result = normalize_via_rational_form(&e);
        // Should be a single Mul term (5 * sin(theta)^2), not an Add of two.
        assert!(!matches!(result, Expr::Add(_)), "{result:?}");
    }

    #[test]
    fn pythagorean_identity_makes_an_expression_collapse_to_zero() {
        // sin(theta)^2 + cos(theta)^2 - 1 = 0 -- only true via the
        // identity; a free-generator ring would leave this as a
        // nonzero three-term sum (D-RF.7's whole point).
        let theta = Expr::var("theta");
        let e = theta.clone().sin().pow(2) + theta.cos().pow(2) - Expr::one();
        assert_eq!(normalize_via_rational_form(&e), Expr::zero());
    }

    #[test]
    fn hyperbolic_identity_makes_an_expression_collapse_to_zero() {
        // cosh(chi)^2 - sinh(chi)^2 - 1 = 0 -- the hyperbolic analogue of
        // the pythagorean identity above (D-RF.7, generalized in
        // `poly.rs::reduce_trig_powers`), sign flipped: `cosh^2 =
        // 1+sinh^2`, not `1-sin^2`. A free-generator ring would leave
        // this as a nonzero three-term sum, same reasoning as sin/cos.
        let chi = Expr::var("chi");
        let e = chi.clone().cosh().pow(2) - chi.sinh().pow(2) - Expr::one();
        assert_eq!(normalize_via_rational_form(&e), Expr::zero());
    }

    #[test]
    fn collects_sinh_squared_across_separately_built_terms() {
        // sinh(chi)^2 * A + sinh(chi)^2 * B -- must collect into one
        // term, the same D-RF.7 concern `collects_sin_squared_across_
        // separately_built_terms` above already checks for sin. `sinh`,
        // not `cosh`, on purpose: `cosh` is the one D-RF.7 always
        // reduces (`cosh^2 -> 1+sinh^2`, mirroring `cos` being the one
        // always reduced, never `sin`) -- `cosh(chi)^2*A + cosh(chi)^2*B`
        // does NOT stay one `Mul` term, it expands to `5 +
        // 5*sinh(chi)^2` (two `Add` terms), correctly, and asserting
        // otherwise would be testing the wrong thing (found by this
        // exact test failing first with `cosh`, confirming the
        // asymmetry holds through the real engine rather than silently
        // picking the wrong example).
        let chi = Expr::var("chi");
        let sinh_sq = chi.sinh().pow(2);
        let e = Expr::int(2) * sinh_sq.clone() + Expr::int(3) * sinh_sq;
        let result = normalize_via_rational_form(&e);
        assert!(!matches!(result, Expr::Add(_)), "{result:?}");
    }

    #[test]
    fn cosh_squared_always_expands_via_the_hyperbolic_identity() {
        // The other half of the asymmetry the test above documents:
        // `cosh(chi)^2` (unlike `sinh(chi)^2`) never survives as its own
        // atom raised to a power -- D-RF.7 always rewrites it to
        // `1+sinh(chi)^2`, so `5*cosh(chi)^2` normalizes to a two-term
        // sum, not one `Mul` term.
        let chi = Expr::var("chi");
        let e = Expr::int(5) * chi.clone().cosh().pow(2);
        let result = normalize_via_rational_form(&e);
        let expected = normalize_via_rational_form(&(Expr::int(5) + Expr::int(5) * chi.sinh().pow(2)));
        assert_eq!(result, expected, "{result:?}");
    }

    #[test]
    fn exp_of_two_different_arguments_does_not_collapse() {
        // exp(a)*exp(b) is deliberately NOT rewritten to exp(a+b)
        // (`AtomTable::exp`'s own doc comment has the full reasoning) --
        // confirms that choice holds through the real engine, not just
        // in the doc comment: two distinct Exp atoms multiplied together
        // stay a product of two atoms, not one.
        let a = Expr::var("a");
        let b = Expr::var("b");
        let e = a.exp() * b.exp();
        let result = normalize_via_rational_form(&e);
        assert!(matches!(&result, Expr::Mul(factors) if factors.iter().filter(|f| matches!(f, Expr::Exp(_))).count() == 2), "{result:?}");
    }

    #[test]
    fn exp_of_the_same_argument_still_combines_via_ordinary_exponent_arithmetic() {
        // exp(a)*exp(a) DOES combine -- not via the a+b identity, but
        // because it is the same atom multiplied by itself, handled by
        // the same ordinary same-generator exponent bookkeeping every
        // other atom already gets (`mul_term`'s `combined` map).
        let a = Expr::var("a");
        let e = a.clone().exp() * a.exp();
        let result = normalize_via_rational_form(&e);
        let expected = normalize_via_rational_form(&Expr::Pow(Box::new(Expr::var("a").exp()), 2));
        assert_eq!(result, expected, "{result:?}");
    }
}
