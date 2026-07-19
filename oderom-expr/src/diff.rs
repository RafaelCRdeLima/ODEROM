//! Symbolic differentiation: the textbook rules (sum, generalized product,
//! integer power, chain rule for `sin`/`cos`). Unlike simplification,
//! there is no ambiguity in what "correct" means here, so this is a
//! direct structural recursion, not a search.

use crate::Expr;

/// `d(expr)/d(var)`.
pub fn diff(expr: &Expr, var: &str) -> Expr {
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
}
