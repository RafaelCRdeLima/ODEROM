//! Symbolic differentiation: the textbook rules (sum, generalized product,
//! integer power, chain rule for `sin`/`cos`/.../an indeterminate
//! function). Unlike simplification, there is no ambiguity in what
//! "correct" means here, so this is a direct structural recursion, not a
//! search.

use crate::Expr;

/// `d(expr)/d(var)`. Checked for cancellation on every call, including
/// the recursive ones (same pattern as
/// `canonical::normalize_via_rational_form`'s own first line, the
/// project's one other unconditional-on-every-call checkpoint): unlike
/// `normalize()`, `diff` never itself calls into the ring/GCD machinery,
/// so without its own check here, a `diff` call over a large expression
/// (the generalized product rule over a long `Mul`, or repeated
/// differentiation building up nested `Func` order) would run
/// uncancellable until it happened to call `normalize()` next -- a real
/// gap `run_cancellable`'s thread-local token would otherwise sit armed
/// but unpolled through. `check_cancelled` is `pub(crate)`, reachable
/// from here since `diff.rs` lives inside `oderom-expr` itself
/// (DESIGN-M6-PREP.md section 4, point 3).
pub fn diff(expr: &Expr, var: &str) -> Expr {
    crate::cancel::check_cancelled();
    match expr {
        Expr::Rational(_) => Expr::zero(),
        Expr::Var(v) => {
            if v == var {
                Expr::one()
            } else {
                Expr::zero()
            }
        }
        Expr::Add(terms) => Expr::Add(terms.iter().map(|t| diff(t, var)).collect()),
        Expr::Mul(factors) => {
            // Generalized product rule: d(f1*..*fn) = sum_i (d(fi) * prod_{j!=i} fj).
            let terms: Vec<Expr> = (0..factors.len())
                .map(|i| {
                    let mut parts: Vec<Expr> = Vec::with_capacity(factors.len());
                    parts.push(diff(&factors[i], var));
                    for (j, f) in factors.iter().enumerate() {
                        if j != i {
                            parts.push(f.clone());
                        }
                    }
                    Expr::Mul(parts)
                })
                .collect();
            Expr::Add(terms)
        }
        Expr::Pow(base, n) => {
            // d(base^n) = n * base^(n-1) * d(base)
            if *n == 0 {
                return Expr::zero();
            }
            Expr::Mul(vec![
                Expr::int(*n as i64),
                Expr::Pow(base.clone(), n - 1),
                diff(base, var),
            ])
        }
        Expr::Sin(inner) => Expr::Mul(vec![Expr::Cos(inner.clone()), diff(inner, var)]),
        Expr::Cos(inner) => Expr::Mul(vec![
            Expr::int(-1),
            Expr::Sin(inner.clone()),
            diff(inner, var),
        ]),
        // d(exp(u))/dx = exp(u) * u' -- exp is its own derivative.
        Expr::Exp(inner) => Expr::Mul(vec![Expr::Exp(inner.clone()), diff(inner, var)]),
        // sinh/cosh derive into each other, same shape as sin/cos but
        // with no sign flip: d(sinh(u)) = cosh(u)*u', d(cosh(u)) =
        // sinh(u)*u' (unlike d(cos(u)) = -sin(u)*u').
        Expr::Sinh(inner) => Expr::Mul(vec![Expr::Cosh(inner.clone()), diff(inner, var)]),
        Expr::Cosh(inner) => Expr::Mul(vec![Expr::Sinh(inner.clone()), diff(inner, var)]),
        // The multivariate chain rule, generalized over `order` instead
        // of a fixed sibling function: d(F(g1,...,gn))/dvar =
        // sum_i (dF/d(arg_i))(g1,...,gn) * d(g_i)/dvar, where dF/d(arg_i)
        // is the SAME function with `order[i]` incremented by one (not a
        // different function -- an indeterminate function's derivative
        // is itself, one order higher, exactly the "f of order k derives
        // to f of order k+1" DESIGN-M6-PREP.md section 1 describes).
        //
        // This one rule, with no special case, already produces every
        // required behavior: a single-argument `f(r)` differentiated
        // wrt `r` gives `f'(r) * 1 = f'(r)`; wrt any other variable `t`
        // gives `f'(r) * d(r)/dt = f'(r) * 0`, which `normalize()`
        // collapses to zero -- "wrong variable is zero" is not a special
        // case here, it falls out of the chain rule the same way it
        // already does for `Var`.
        Expr::Func { name, args, order } => {
            let terms: Vec<Expr> = args
                .iter()
                .enumerate()
                .map(|(i, arg)| {
                    let mut partial_order = order.clone();
                    partial_order[i] += 1;
                    let partial = Expr::Func { name: name.clone(), args: args.clone(), order: partial_order };
                    Expr::Mul(vec![partial, diff(arg, var)])
                })
                .collect();
            Expr::Add(terms)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize;

    #[test]
    fn derivative_of_constant_is_zero() {
        assert_eq!(normalize(&diff(&Expr::int(5), "r")), Expr::zero());
    }

    #[test]
    fn derivative_of_var_wrt_itself_is_one() {
        assert_eq!(normalize(&diff(&Expr::var("r"), "r")), Expr::one());
    }

    #[test]
    fn derivative_of_unrelated_var_is_zero() {
        assert_eq!(normalize(&diff(&Expr::var("theta"), "r")), Expr::zero());
    }

    #[test]
    fn power_rule() {
        // d(r^3)/dr = 3 r^2
        let e = Expr::var("r").pow(3);
        let expected = Expr::int(3) * Expr::var("r").pow(2);
        assert_eq!(normalize(&diff(&e, "r")), normalize(&expected));
    }

    #[test]
    fn negative_power_rule() {
        // d(r^-1)/dr = -1 * r^-2
        let e = Expr::var("r").pow(-1);
        let expected = Expr::int(-1) * Expr::var("r").pow(-2);
        assert_eq!(normalize(&diff(&e, "r")), normalize(&expected));
    }

    #[test]
    fn product_rule() {
        // d(r * M)/dr = M   (M constant wrt r)
        let e = Expr::var("r") * Expr::var("M");
        assert_eq!(normalize(&diff(&e, "r")), Expr::var("M"));
    }

    #[test]
    fn chain_rule_through_sin() {
        // d(sin(2r))/dr = 2 cos(2r)
        let e = (Expr::int(2) * Expr::var("r")).sin();
        let expected = Expr::int(2) * (Expr::int(2) * Expr::var("r")).cos();
        assert_eq!(normalize(&diff(&e, "r")), normalize(&expected));
    }

    #[test]
    fn derivative_of_1_minus_2m_over_r() {
        // d/dr (1 - 2M/r) = 2M/r^2
        let e = Expr::one() - Expr::int(2) * Expr::var("M") / Expr::var("r");
        let expected = Expr::int(2) * Expr::var("M") * Expr::var("r").pow(-2);
        assert_eq!(normalize(&diff(&e, "r")), normalize(&expected));
    }

    #[test]
    fn exp_is_its_own_derivative() {
        // d(exp(r))/dr = exp(r)
        let e = Expr::var("r").exp();
        assert_eq!(normalize(&diff(&e, "r")), normalize(&Expr::var("r").exp()));
    }

    #[test]
    fn chain_rule_through_exp() {
        // d(exp(2r))/dr = 2*exp(2r)
        let e = (Expr::int(2) * Expr::var("r")).exp();
        let expected = Expr::int(2) * (Expr::int(2) * Expr::var("r")).exp();
        assert_eq!(normalize(&diff(&e, "r")), normalize(&expected));
    }

    #[test]
    fn sinh_derives_to_cosh() {
        // d(sinh(2r))/dr = 2*cosh(2r) -- no sign flip, unlike cos.
        let e = (Expr::int(2) * Expr::var("r")).sinh();
        let expected = Expr::int(2) * (Expr::int(2) * Expr::var("r")).cosh();
        assert_eq!(normalize(&diff(&e, "r")), normalize(&expected));
    }

    #[test]
    fn cosh_derives_to_sinh() {
        // d(cosh(2r))/dr = 2*sinh(2r) -- also no sign flip.
        let e = (Expr::int(2) * Expr::var("r")).cosh();
        let expected = Expr::int(2) * (Expr::int(2) * Expr::var("r")).sinh();
        assert_eq!(normalize(&diff(&e, "r")), normalize(&expected));
    }

    // -----------------------------------------------------------------
    // Marco 6 step 4: indeterminate functions (DESIGN-M6-PREP.md
    // section 1). `f`'s derivative is itself, one order higher -- never
    // a fixed sibling function the way sin's derivative is always
    // literally cos -- so these are the chain rule generalized over
    // `order`, not a set of special cases.
    // -----------------------------------------------------------------

    fn f_of(r: Expr) -> Expr {
        Expr::func("f", vec![r])
    }

    fn g_of(r: Expr) -> Expr {
        Expr::func("g", vec![r])
    }

    fn f_prime_of(r: Expr) -> Expr {
        Expr::Func { name: "f".to_string(), args: vec![r], order: vec![1] }
    }

    fn g_prime_of(r: Expr) -> Expr {
        Expr::Func { name: "g".to_string(), args: vec![r], order: vec![1] }
    }

    fn f_double_prime_of(r: Expr) -> Expr {
        Expr::Func { name: "f".to_string(), args: vec![r], order: vec![2] }
    }

    #[test]
    fn derivative_of_an_indeterminate_function_wrt_its_own_argument_is_the_bare_prime() {
        // diff(f(r), r) = f'(r).
        let result = normalize(&diff(&f_of(Expr::var("r")), "r"));
        let expected = normalize(&f_prime_of(Expr::var("r")));
        assert_eq!(result, expected);
    }

    #[test]
    fn power_and_chain_rule_compose_through_an_indeterminate_function() {
        // diff(f(r)^2, r) = 2*f(r)*f'(r).
        let e = f_of(Expr::var("r")).pow(2);
        let result = normalize(&diff(&e, "r"));
        let expected = normalize(&(Expr::int(2) * f_of(Expr::var("r")) * f_prime_of(Expr::var("r"))));
        assert_eq!(result, expected);
    }

    #[test]
    fn product_rule_composes_through_two_distinct_indeterminate_functions() {
        // diff(f(r)*g(r), r) = f'(r)*g(r) + f(r)*g'(r).
        let e = f_of(Expr::var("r")) * g_of(Expr::var("r"));
        let result = normalize(&diff(&e, "r"));
        let expected =
            normalize(&(f_prime_of(Expr::var("r")) * g_of(Expr::var("r")) + f_of(Expr::var("r")) * g_prime_of(Expr::var("r"))));
        assert_eq!(result, expected);
    }

    #[test]
    fn derivative_of_an_indeterminate_function_wrt_an_unrelated_coordinate_is_zero() {
        // diff(f(r), t) = 0 -- falls out of the chain rule itself
        // (diff(r, t) = 0), not a special case: see `diff`'s own `Func`
        // arm doc comment.
        let result = normalize(&diff(&f_of(Expr::var("r")), "t"));
        assert_eq!(result, Expr::zero());
    }

    #[test]
    fn second_derivative_of_an_indeterminate_function_is_order_two() {
        // diff(diff(f(r), r), r) = f''(r).
        let once = diff(&f_of(Expr::var("r")), "r");
        let result = normalize(&diff(&once, "r"));
        let expected = normalize(&f_double_prime_of(Expr::var("r")));
        assert_eq!(result, expected);
    }

    #[test]
    fn multivariable_partial_derivative_picks_out_only_the_matching_argument_slot() {
        // h(t, r): dh/dt and dh/dr are two DISTINCT order-one partials
        // (order [1,0] vs [0,1]) -- neither collapses into the other,
        // and differentiating wrt a genuinely unrelated third variable
        // is zero, the same "falls out of the chain rule" property the
        // single-argument case has.
        let t = Expr::var("t");
        let r = Expr::var("r");
        let h = Expr::func("h", vec![t.clone(), r.clone()]);

        let dh_dt = normalize(&diff(&h, "t"));
        let expected_dt = normalize(&Expr::Func { name: "h".to_string(), args: vec![t.clone(), r.clone()], order: vec![1, 0] });
        assert_eq!(dh_dt, expected_dt);

        let dh_dr = normalize(&diff(&h, "r"));
        let expected_dr = normalize(&Expr::Func { name: "h".to_string(), args: vec![t.clone(), r.clone()], order: vec![0, 1] });
        assert_eq!(dh_dr, expected_dr);

        assert_ne!(dh_dt, dh_dr, "the two distinct partials must not collapse into each other");

        let dh_ds = normalize(&diff(&h, "s"));
        assert_eq!(dh_ds, Expr::zero(), "differentiating wrt a variable that appears in none of h's arguments must be zero");
    }

    #[test]
    fn second_mixed_partial_of_a_multivariable_indeterminate_function() {
        // d^2h/dt dr: differentiate h(t,r) wrt t (giving order [1,0]),
        // then that result wrt r (giving order [1,1]) -- confirms
        // `order` accumulates correctly across two separate `diff`
        // calls on different variables, not just repeated calls on the
        // same one (the single-argument second-derivative test above).
        let t = Expr::var("t");
        let r = Expr::var("r");
        let h = Expr::func("h", vec![t.clone(), r.clone()]);
        let dh_dt = diff(&h, "t");
        let mixed = normalize(&diff(&dh_dt, "r"));
        let expected = normalize(&Expr::Func { name: "h".to_string(), args: vec![t, r], order: vec![1, 1] });
        assert_eq!(mixed, expected);
    }

    #[test]
    fn distinct_indeterminate_functions_never_cancel_or_collapse() {
        // f(r) - f(r) = 0 (same atom, cancels normally), but f(r) -
        // g(r) must NOT collapse to zero or otherwise simplify -- an
        // indeterminate function has no identity relating it to a
        // different one (DESIGN-M6-PREP.md section 1: unlike sin/cos,
        // no cross-atom rewrite exists or should ever be invented for
        // this atom).
        let r = Expr::var("r");
        assert_eq!(normalize(&(f_of(r.clone()) - f_of(r.clone()))), Expr::zero());
        assert_ne!(normalize(&(f_of(r.clone()) - g_of(r))), Expr::zero());
    }
}
