//! Isolating a target subexpression's linear coefficient out of a
//! larger expression -- `oderom-components::curvature::accel_equations`
//! (Marco 6 step 4, round C) uses this to divide the acceleration term
//! `x_a''(param)` out of `geodesic`'s own closed-form equation, turning
//! `x_a'' + Gamma...  = 0` into `x_a'' = f(x, x')`. Purely structural:
//! no numeric substitution, no assumption that any particular normal
//! form holds -- a direct recursive classification of how `target`
//! participates in `e`, checked, never guessed.
//!
//! `target` is compared to every node of `e` by structural equality
//! (`Expr`'s own `PartialEq`) -- exactly the same "is this literally the
//! node I mean" check `oderom-components::geodesic_render`'s own
//! `substitute_dotted` already uses for the analogous "is this literally
//! the parameter's own derivative" question, never a shape-only
//! heuristic.

use crate::{normalize, Expr};

/// `e = coeff*target + remainder`, both already `normalize()`-d, or
/// `None` if `e` is not affine-linear in `target` -- `target` appears
/// with degree other than exactly 1 somewhere (squared, inverted,
/// nested inside a transcendental function's argument or another
/// function's own argument list), or more than one factor of the same
/// product each depend on `target` (which would make that product
/// degree >= 2 in `target`). Never panics, never assumes a shape holds
/// -- an expression this can't classify is reported as `None`, which
/// the caller turns into a named, diagnostic error rather than ever
/// dividing by something that was silently wrong.
///
/// Every branch below normalizes its own `(coeff, remainder)` before
/// returning -- not just once at the very end -- so that every zero-ness
/// check made along the way (deciding whether a `Mul` factor or `Pow`
/// base actually depends on `target`) is checking a real, reduced zero,
/// never an unreduced shape like `Add([Rational(0), Rational(0)])` that
/// merely *looks* nonzero without being normalized first.
pub fn isolate_linear(e: &Expr, target: &Expr) -> Option<(Expr, Expr)> {
    analyze(e, target)
}

fn analyze(e: &Expr, target: &Expr) -> Option<(Expr, Expr)> {
    if e == target {
        return Some((Expr::one(), Expr::zero()));
    }
    let (coeff, remainder) = match e {
        Expr::Rational(_) | Expr::Var(_) => (Expr::zero(), e.clone()),
        Expr::Add(terms) => {
            let mut coeffs = Vec::with_capacity(terms.len());
            let mut rests = Vec::with_capacity(terms.len());
            for t in terms {
                let (c, r) = analyze(t, target)?;
                coeffs.push(c);
                rests.push(r);
            }
            (Expr::Add(coeffs), Expr::Add(rests))
        }
        Expr::Mul(factors) => {
            // At most one factor may depend on `target` for the whole
            // product to stay linear in it -- two independent factors
            // each depending on `target` would make the product at
            // least quadratic in `target`.
            let mut free_factors = Vec::with_capacity(factors.len());
            let mut linear: Option<(Expr, Expr)> = None;
            for f in factors {
                let (c, r) = analyze(f, target)?;
                if c.is_zero() {
                    free_factors.push(r);
                } else if linear.is_some() {
                    return None;
                } else {
                    linear = Some((c, r));
                }
            }
            let free_product = match free_factors.len() {
                0 => Expr::one(),
                1 => free_factors.into_iter().next().expect("checked len==1"),
                _ => Expr::Mul(free_factors),
            };
            match linear {
                None => (Expr::zero(), free_product),
                Some((c, r)) => (c * free_product.clone(), r * free_product),
            }
        }
        Expr::Pow(base, n) => {
            let (c, r) = analyze(base, target)?;
            if c.is_zero() {
                (Expr::zero(), Expr::Pow(Box::new(r), *n))
            } else if *n == 1 {
                // `normalize()` already collapses `Pow(_, 1)` on its own,
                // but this does not assume that invariant holds here --
                // if it ever showed up, exponent 1 is still linear.
                (c, r)
            } else {
                return None; // (c*target + r)^n, n != 1, is not affine in target.
            }
        }
        Expr::Sin(inner) | Expr::Cos(inner) | Expr::Exp(inner) | Expr::Sinh(inner) | Expr::Cosh(inner) => {
            let (c, _) = analyze(inner, target)?;
            if c.is_zero() {
                (Expr::zero(), e.clone())
            } else {
                return None; // target inside a transcendental function's argument: not affine.
            }
        }
        Expr::Func { args, .. } => {
            for a in args {
                let (c, _) = analyze(a, target)?;
                if !c.is_zero() {
                    return None; // target nested inside another function's argument list.
                }
            }
            (Expr::zero(), e.clone())
        }
    };
    Some((normalize(&coeff), normalize(&remainder)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Expr {
        Expr::Func { name: "r".to_string(), args: vec![Expr::var("tau")], order: vec![2] }
    }

    #[test]
    fn isolates_the_bare_target_with_coefficient_one() {
        let (c, r) = isolate_linear(&t(), &t()).unwrap();
        assert_eq!(c, Expr::one());
        assert_eq!(r, Expr::zero());
    }

    #[test]
    fn isolates_a_target_with_a_nontrivial_coefficient_and_remainder() {
        // 3*target + M*r  (a plain sum, no shared denominator)
        let e = Expr::int(3) * t() + Expr::var("M") * Expr::var("r");
        let (c, r) = isolate_linear(&e, &t()).unwrap();
        assert_eq!(normalize(&c), Expr::int(3));
        assert_eq!(normalize(&r), normalize(&(Expr::var("M") * Expr::var("r"))));
    }

    #[test]
    fn isolates_correctly_when_the_whole_equation_shares_a_common_denominator() {
        // target - M*x^2/(M - r) -- the exact shape geodesic_equations
        // produces (`x_a'' + Gamma^a_bc * x_b' * x_c'`, one Christoffel
        // coefficient with a nontrivial denominator): the acceleration
        // term is added at coefficient 1 BEFORE `normalize()` combines
        // everything over a common denominator, so the target's true
        // coefficient in the fully expanded single fraction is 1 again
        // (the `(M-r)` that distribution multiplies target's own term
        // by cancels exactly against the shared `(M-r)^-1` factor) --
        // never left as some leftover, uncancelled `1/(M-r)`.
        let m = Expr::var("M");
        let r = Expr::var("r");
        let x = Expr::var("x");
        let e = normalize(&(t() - m.clone() * x.clone().pow(2) / (m.clone() - r.clone())));
        let (c, rem) = isolate_linear(&e, &t()).unwrap();
        assert_eq!(c, Expr::one());
        assert_eq!(rem, normalize(&(-(m.clone() * x.pow(2)) / (m - r))));
    }

    #[test]
    fn a_squared_target_is_reported_as_not_linear() {
        let e = t().pow(2) + Expr::var("M");
        assert!(isolate_linear(&e, &t()).is_none());
    }

    #[test]
    fn two_independent_factors_each_depending_on_target_is_not_linear() {
        let e = t() * t();
        assert!(isolate_linear(&e, &t()).is_none());
    }

    #[test]
    fn target_nested_inside_a_transcendental_function_is_not_linear() {
        let e = t().sin();
        assert!(isolate_linear(&e, &t()).is_none());
    }

    #[test]
    fn target_nested_inside_another_functions_argument_is_not_linear() {
        let e = Expr::func("f", vec![t()]);
        assert!(isolate_linear(&e, &t()).is_none());
    }

    #[test]
    fn an_expression_never_containing_target_has_coefficient_zero() {
        let e = Expr::var("M") * Expr::var("r");
        let (c, r) = isolate_linear(&e, &t()).unwrap();
        assert_eq!(c, Expr::zero());
        assert_eq!(normalize(&r), normalize(&(Expr::var("M") * Expr::var("r"))));
    }
}
