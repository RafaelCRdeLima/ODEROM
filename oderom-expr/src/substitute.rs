//! Variable substitution: replacing every occurrence of a coordinate
//! variable with another expression. Needed starting with Marco 3, to
//! express one chart's metric in another chart's coordinates via a
//! transition map. Direct structural recursion, no ambiguity, same as
//! [`crate::diff`].

use crate::Expr;

/// Replaces every occurrence of `Var(var)` in `expr` with `replacement`.
pub fn substitute(expr: &Expr, var: &str, replacement: &Expr) -> Expr {
    match expr {
        Expr::Rational(_) => expr.clone(),
        Expr::Var(v) => {
            if v == var {
                replacement.clone()
            } else {
                expr.clone()
            }
        }
        Expr::Add(terms) => Expr::Add(terms.iter().map(|t| substitute(t, var, replacement)).collect()),
        Expr::Mul(factors) => {
            Expr::Mul(factors.iter().map(|f| substitute(f, var, replacement)).collect())
        }
        Expr::Pow(base, n) => Expr::Pow(Box::new(substitute(base, var, replacement)), *n),
        Expr::Sin(inner) => Expr::Sin(Box::new(substitute(inner, var, replacement))),
        Expr::Cos(inner) => Expr::Cos(Box::new(substitute(inner, var, replacement))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize;

    #[test]
    fn substitutes_a_bare_variable() {
        let e = Expr::var("x");
        let result = substitute(&e, "x", &Expr::int(5));
        assert_eq!(result, Expr::int(5));
    }

    #[test]
    fn leaves_unrelated_variables_alone() {
        let e = Expr::var("y");
        let result = substitute(&e, "x", &Expr::int(5));
        assert_eq!(result, Expr::var("y"));
    }

    #[test]
    fn substitutes_inside_a_compound_expression() {
        // (x + 1) * x^2, substitute x -> (u + v)
        let x = Expr::var("x");
        let e = (x.clone() + Expr::one()) * x.pow(2);
        let uv = Expr::var("u") + Expr::var("v");
        let result = substitute(&e, "x", &uv);
        let expected = (Expr::var("u") + Expr::var("v") + Expr::one())
            * (Expr::var("u") + Expr::var("v")).pow(2);
        assert_eq!(normalize(&result), normalize(&expected));
    }

    #[test]
    fn substitutes_inside_sin_and_cos() {
        let e = Expr::var("theta").sin() + Expr::var("theta").cos();
        let result = substitute(&e, "theta", &Expr::int(0));
        assert_eq!(normalize(&result), normalize(&(Expr::int(0).sin() + Expr::int(0).cos())));
    }

    #[test]
    fn stereographic_style_substitution() {
        // u -> u / (u^2 + v^2), a fragment of the stereographic transition.
        let u = Expr::var("u");
        let v = Expr::var("v");
        let denom = u.clone().pow(2) + v.pow(2);
        let replacement = u / denom;
        let e = Expr::var("u").pow(2);
        let result = normalize(&substitute(&e, "u", &replacement));
        // Just check it doesn't panic and produces *something* containing u, v.
        assert_ne!(result, Expr::zero());
    }
}
