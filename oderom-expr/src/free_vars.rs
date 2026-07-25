//! Every free variable name (`Expr::Var`) appearing anywhere in an
//! expression -- used by `geodesic` (Marco 6 step 4, round B) to check
//! its user-named affine parameter doesn't collide with a variable
//! already used in the metric/connection in scope (`M`, `Q`, ...).
//! Direct structural recursion, no ambiguity, same shape as `diff`/
//! `substitute`.
//!
//! A `Func`'s own `name` (e.g. `f` in `f(r)`) is deliberately never
//! collected here: it names a distinct function symbol, not a
//! reference to a variable -- only its *arguments* can contain `Var`
//! nodes worth collecting.

use crate::Expr;
use std::collections::HashSet;

pub fn free_vars(e: &Expr) -> HashSet<String> {
    let mut out = HashSet::new();
    collect(e, &mut out);
    out
}

fn collect(e: &Expr, out: &mut HashSet<String>) {
    match e {
        Expr::Rational(_) => {}
        Expr::Var(name) => {
            out.insert(name.clone());
        }
        Expr::Add(terms) | Expr::Mul(terms) => terms.iter().for_each(|t| collect(t, out)),
        Expr::Pow(base, _) => collect(base, out),
        Expr::Sin(x) | Expr::Cos(x) | Expr::Exp(x) | Expr::Sinh(x) | Expr::Cosh(x) => collect(x, out),
        Expr::Func { args, .. } => args.iter().for_each(|a| collect(a, out)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_a_bare_variable() {
        let vars = free_vars(&Expr::var("M"));
        assert_eq!(vars, HashSet::from(["M".to_string()]));
    }

    #[test]
    fn collects_every_distinct_variable_in_a_compound_expression() {
        let e = Expr::var("M") * Expr::var("r").pow(-1) + Expr::var("Q").pow(2) * Expr::var("r").pow(-2);
        let vars = free_vars(&e);
        assert_eq!(vars, HashSet::from(["M".to_string(), "Q".to_string(), "r".to_string()]));
    }

    #[test]
    fn collects_variables_inside_known_function_arguments() {
        let vars = free_vars(&Expr::var("theta").sin());
        assert_eq!(vars, HashSet::from(["theta".to_string()]));
    }

    #[test]
    fn collects_variables_inside_an_indeterminate_functions_arguments_never_its_own_name() {
        let vars = free_vars(&Expr::func("f", vec![Expr::var("r")]));
        assert_eq!(vars, HashSet::from(["r".to_string()]), "the function name `f` itself must never be collected as a free variable");
    }

    #[test]
    fn a_constant_has_no_free_variables() {
        assert!(free_vars(&Expr::int(5)).is_empty());
    }
}
