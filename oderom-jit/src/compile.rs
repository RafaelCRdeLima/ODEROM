//! Lowering [`Expr`] to [`Program`], with common-subexpression
//! elimination via hash-consing (see the crate docs).

use crate::error::JitError;
use crate::program::{Op, Program};
use oderom_expr::Expr;
use rustc_hash::FxHashMap;

struct Builder {
    ops: Vec<Op>,
    hashcons: FxHashMap<Op, usize>,
}

impl Builder {
    fn push(&mut self, op: Op) -> usize {
        if let Some(&idx) = self.hashcons.get(&op) {
            return idx;
        }
        let idx = self.ops.len();
        self.hashcons.insert(op, idx);
        self.ops.push(op);
        idx
    }
}

/// Compiles `expr` into a [`Program`]. `vars[i]` names the coordinate
/// that fills `Op::Var(i)`'s slot when the resulting program is later
/// evaluated. Errors if `expr` mentions a variable with no matching
/// entry in `vars`.
pub fn compile(expr: &Expr, vars: &[String]) -> Result<Program, JitError> {
    let mut b = Builder { ops: Vec::new(), hashcons: FxHashMap::default() };
    let output = lower(expr, vars, &mut b)?;
    Ok(Program { ops: b.ops, output })
}

fn lower(expr: &Expr, vars: &[String], b: &mut Builder) -> Result<usize, JitError> {
    Ok(match expr {
        Expr::Rational(s) => b.push(Op::constant(s.to_f64_lossy())),
        Expr::Var(name) => {
            let idx = vars
                .iter()
                .position(|v| v == name)
                .ok_or_else(|| JitError::UnknownVariable(name.clone()))?;
            b.push(Op::Var(idx))
        }
        Expr::Add(terms) => {
            let mut acc: Option<usize> = None;
            for t in terms {
                let id = lower(t, vars, b)?;
                acc = Some(match acc {
                    None => id,
                    Some(a) => b.push(Op::Add(a, id)),
                });
            }
            acc.unwrap_or_else(|| b.push(Op::constant(0.0)))
        }
        Expr::Mul(factors) => {
            let mut acc: Option<usize> = None;
            for f in factors {
                let id = lower(f, vars, b)?;
                acc = Some(match acc {
                    None => id,
                    Some(a) => b.push(Op::Mul(a, id)),
                });
            }
            acc.unwrap_or_else(|| b.push(Op::constant(1.0)))
        }
        Expr::Pow(base, n) => {
            let id = lower(base, vars, b)?;
            b.push(Op::Pow(id, *n))
        }
        Expr::Sin(inner) => {
            let id = lower(inner, vars, b)?;
            b.push(Op::Sin(id))
        }
        Expr::Cos(inner) => {
            let id = lower(inner, vars, b)?;
            b.push(Op::Cos(id))
        }
        Expr::Exp(inner) => {
            let id = lower(inner, vars, b)?;
            b.push(Op::Exp(id))
        }
        Expr::Sinh(inner) => {
            let id = lower(inner, vars, b)?;
            b.push(Op::Sinh(id))
        }
        Expr::Cosh(inner) => {
            let id = lower(inner, vars, b)?;
            b.push(Op::Cosh(id))
        }
        Expr::Func { name, .. } => return Err(JitError::IndeterminateFunction(name.clone())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oderom_expr::normalize;

    fn vars(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn evaluates_a_constant() {
        let p = compile(&Expr::int(5), &[]).unwrap();
        assert_eq!(p.eval(&[]), 5.0);
    }

    #[test]
    fn evaluates_a_variable() {
        let p = compile(&Expr::var("x"), &vars(&["x"])).unwrap();
        assert_eq!(p.eval(&[3.5]), 3.5);
    }

    #[test]
    fn evaluates_arithmetic() {
        // 2*x + 3
        let e = normalize(&(Expr::int(2) * Expr::var("x") + Expr::int(3)));
        let p = compile(&e, &vars(&["x"])).unwrap();
        assert_eq!(p.eval(&[4.0]), 11.0);
    }

    #[test]
    fn evaluates_power_and_trig() {
        // x^2 + sin(x)
        let e = Expr::var("x").pow(2) + Expr::var("x").sin();
        let p = compile(&e, &vars(&["x"])).unwrap();
        let expected = 2.0f64.powi(2) + 2.0f64.sin();
        assert!((p.eval(&[2.0]) - expected).abs() < 1e-12);
    }

    #[test]
    fn common_subexpressions_share_one_instruction() {
        // (x+1) * (x+1): the two (x+1) sub-expressions must compile to
        // the same instruction, not two copies.
        let x = Expr::var("x");
        let e = (x.clone() + Expr::one()) * (x + Expr::one());
        let p = compile(&e, &vars(&["x"])).unwrap();
        // Var(x), Const(1), Add(x,1), Mul(add,add) -- 4 instructions, not 5.
        assert_eq!(p.ops.len(), 4);
        assert_eq!(p.eval(&[3.0]), 16.0);
    }

    #[test]
    fn unlisted_variable_is_an_error() {
        let err = compile(&Expr::var("y"), &vars(&["x"])).unwrap_err();
        assert_eq!(err, JitError::UnknownVariable("y".to_string()));
    }

    /// Marco 6 step 4 (indeterminate functions): a `Func` node has no
    /// known numeric value, so compiling it must fail cleanly, never
    /// panic and never silently produce a wrong number.
    #[test]
    fn an_indeterminate_function_is_a_clean_compile_error() {
        let f_of_r = Expr::func("f", vec![Expr::var("r")]);
        let err = compile(&f_of_r, &vars(&["r"])).unwrap_err();
        assert_eq!(err, JitError::IndeterminateFunction("f".to_string()));
    }
}
