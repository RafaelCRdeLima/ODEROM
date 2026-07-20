//! `SCALAR_EXPR`: one grammar for scalar expressions (metric/connection
//! component values), with two token spellings at every production that
//! has a natural LaTeX counterpart -- never two parsers. See
//! DESIGN-UI.md section 6.1 for the EBNF this implements.
//!
//! ```text
//! SUM     := PRODUCT (('+' | '-') PRODUCT)*
//! PRODUCT := UNARY (('*' | '/') UNARY)*
//! UNARY   := '-' UNARY | POWER
//! POWER   := ATOM ('^' SIGNED_EXP)?
//! ATOM    := INT | IDENT | ('sin'|'cos') '(' SUM ')' | '(' SUM ')'
//!          | '\frac' '{' SUM '}' '{' SUM '}'
//!          | '\left' '(' SUM '\right' ')'
//!          | ('\sin'|'\cos') ('^' SIGNED_EXP)? ('(' SUM ')' | '{' SUM '}')
//!          | '\' GREEK_NAME                      -- Var(lowercase name)
//! SIGNED_EXP := ('-')? INT | '{' ('-')? INT '}'
//! ```
//!
//! `1/2` is not special-cased as a rational literal: it falls out of
//! `PRODUCT`'s ordinary division (`Mul([1, Pow(2, -1)])`), and
//! `oderom_expr::normalize` collapses that to the same `Rational(1, 2)`
//! a literal would have produced. One fewer production, same language.

use crate::error::CliError;
use crate::parser::{Tok, TokStream};
use oderom_expr::{Expr, GREEK_LETTERS};

pub(crate) fn parse_scalar_expr(toks: &mut TokStream) -> Result<Expr, CliError> {
    parse_sum(toks)
}

fn parse_sum(toks: &mut TokStream) -> Result<Expr, CliError> {
    let mut terms = vec![parse_product(toks)?];
    loop {
        match toks.peek() {
            Tok::Sym('+') => {
                toks.advance();
                terms.push(parse_product(toks)?);
            }
            Tok::Sym('-') => {
                toks.advance();
                terms.push(Expr::int(-1) * parse_product(toks)?);
            }
            _ => break,
        }
    }
    Ok(one_or_add(terms))
}

fn parse_product(toks: &mut TokStream) -> Result<Expr, CliError> {
    let mut factors = vec![parse_unary(toks)?];
    loop {
        match toks.peek().clone() {
            Tok::Sym('*') => {
                toks.advance();
                factors.push(parse_unary(toks)?);
            }
            Tok::Sym('/') => {
                toks.advance();
                factors.push(Expr::Pow(Box::new(parse_unary(toks)?), -1));
            }
            // Juxtaposition ("2M", "r^2\sin^2(\theta)"): no explicit `*`
            // between two atoms is still multiplication, as long as the
            // next token unambiguously starts one (never `+`/`-`, which
            // always belong to `SUM` -- "2 - M" stays subtraction).
            t if starts_atom(&t) => {
                factors.push(parse_unary(toks)?);
            }
            _ => break,
        }
    }
    Ok(one_or_mul(factors))
}

fn starts_atom(t: &Tok) -> bool {
    match t {
        Tok::Int(_) | Tok::Ident(_) | Tok::Sym('(') => true,
        // `\right` is `\left`'s closing delimiter, never a value in its
        // own right -- excluded so `\left(...\right)` doesn't get read
        // as "...juxtaposed with \right".
        Tok::Command(name) => name != "right",
        _ => false,
    }
}

fn parse_unary(toks: &mut TokStream) -> Result<Expr, CliError> {
    if *toks.peek() == Tok::Sym('-') {
        toks.advance();
        Ok(Expr::int(-1) * parse_unary(toks)?)
    } else {
        parse_power(toks)
    }
}

fn parse_power(toks: &mut TokStream) -> Result<Expr, CliError> {
    let base = parse_atom(toks)?;
    if *toks.peek() == Tok::Sym('^') {
        toks.advance();
        let exp = parse_signed_exponent(toks)?;
        Ok(Expr::Pow(Box::new(base), exp))
    } else {
        Ok(base)
    }
}

fn parse_atom(toks: &mut TokStream) -> Result<Expr, CliError> {
    match toks.peek().clone() {
        Tok::Int(n) => {
            toks.advance();
            Ok(Expr::int(n as i64))
        }
        Tok::Ident(name) => {
            toks.advance();
            if *toks.peek() == Tok::Sym('(') {
                parse_ascii_function(toks, &name)
            } else {
                Ok(Expr::var(name))
            }
        }
        Tok::Sym('(') => {
            toks.advance();
            let e = parse_sum(toks)?;
            toks.expect_sym(')')?;
            Ok(e)
        }
        Tok::Command(name) => parse_command_atom(toks, &name),
        other => Err(CliError::Parse(format!("expected a scalar expression, found {other:?}"))),
    }
}

fn parse_ascii_function(toks: &mut TokStream, name: &str) -> Result<Expr, CliError> {
    let Some(trig) = trig_of(name) else {
        return Err(CliError::Parse(format!("unknown function `{name}` (only sin/cos are supported)")));
    };
    toks.expect_sym('(')?;
    let arg = parse_sum(toks)?;
    toks.expect_sym(')')?;
    Ok(apply_trig(trig, arg))
}

fn parse_command_atom(toks: &mut TokStream, name: &str) -> Result<Expr, CliError> {
    toks.advance();
    match name {
        "frac" => {
            toks.expect_sym('{')?;
            let num = parse_sum(toks)?;
            toks.expect_sym('}')?;
            toks.expect_sym('{')?;
            let den = parse_sum(toks)?;
            toks.expect_sym('}')?;
            Ok(num * Expr::Pow(Box::new(den), -1))
        }
        "left" => {
            toks.expect_sym('(')?;
            let e = parse_sum(toks)?;
            toks.expect_command("right")?;
            toks.expect_sym(')')?;
            Ok(e)
        }
        "sin" | "cos" => {
            let trig = trig_of(name).expect("matched \"sin\"/\"cos\" above");
            let power = if *toks.peek() == Tok::Sym('^') {
                toks.advance();
                Some(parse_signed_exponent(toks)?)
            } else {
                None
            };
            let arg = parse_group(toks)?;
            let applied = apply_trig(trig, arg);
            Ok(match power {
                Some(p) => Expr::Pow(Box::new(applied), p),
                None => applied,
            })
        }
        _ => {
            let lower = name.to_ascii_lowercase();
            if GREEK_LETTERS.contains(&lower.as_str()) {
                Ok(Expr::var(lower))
            } else {
                Err(CliError::Parse(format!("unknown LaTeX command `\\{name}`")))
            }
        }
    }
}

/// `'(' SUM ')'` or `'{' SUM '}'` -- the argument of `\sin`/`\cos`, which
/// LaTeX writers spell either way.
fn parse_group(toks: &mut TokStream) -> Result<Expr, CliError> {
    match toks.peek().clone() {
        Tok::Sym('(') => {
            toks.advance();
            let e = parse_sum(toks)?;
            toks.expect_sym(')')?;
            Ok(e)
        }
        Tok::Sym('{') => {
            toks.advance();
            let e = parse_sum(toks)?;
            toks.expect_sym('}')?;
            Ok(e)
        }
        other => Err(CliError::Parse(format!("expected `(` or `{{`, found {other:?}"))),
    }
}

/// `SIGNED_EXP := ('-')? INT | '{' ('-')? INT '}'` -- an exponent is
/// always a literal integer (positive or negative), never a general
/// subexpression: that is all `Expr::Pow(Box<Expr>, i32)` can represent.
fn parse_signed_exponent(toks: &mut TokStream) -> Result<i32, CliError> {
    if *toks.peek() == Tok::Sym('{') {
        toks.advance();
        let n = parse_bare_signed_int(toks)?;
        toks.expect_sym('}')?;
        Ok(n)
    } else {
        parse_bare_signed_int(toks)
    }
}

fn parse_bare_signed_int(toks: &mut TokStream) -> Result<i32, CliError> {
    let negative = if *toks.peek() == Tok::Sym('-') {
        toks.advance();
        true
    } else {
        false
    };
    let n = toks.int()? as i32;
    Ok(if negative { -n } else { n })
}

#[derive(Clone, Copy)]
enum Trig {
    Sin,
    Cos,
}

fn trig_of(name: &str) -> Option<Trig> {
    match name {
        "sin" => Some(Trig::Sin),
        "cos" => Some(Trig::Cos),
        _ => None,
    }
}

fn apply_trig(t: Trig, arg: Expr) -> Expr {
    match t {
        Trig::Sin => arg.sin(),
        Trig::Cos => arg.cos(),
    }
}

fn one_or_add(mut terms: Vec<Expr>) -> Expr {
    if terms.len() == 1 {
        terms.pop().expect("length checked above")
    } else {
        Expr::Add(terms)
    }
}

fn one_or_mul(mut factors: Vec<Expr>) -> Expr {
    if factors.len() == 1 {
        factors.pop().expect("length checked above")
    } else {
        Expr::Mul(factors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oderom_expr::normalize;

    fn parse(src: &str) -> Expr {
        let mut toks = TokStream::new(src).unwrap();
        parse_scalar_expr(&mut toks).unwrap()
    }

    #[test]
    fn ascii_and_latex_lower_to_the_same_ast() {
        let ascii = normalize(&parse("-(1 - 2*M/r)"));
        let latex = normalize(&parse(r"-\left(1 - \frac{2M}{r}\right)"));
        assert_eq!(ascii, latex);

        let ascii2 = normalize(&parse("r^2 * sin(theta)^2"));
        let latex2 = normalize(&parse(r"r^2 \sin^2(\theta)"));
        assert_eq!(ascii2, latex2);
    }

    #[test]
    fn frac_is_division() {
        let a = normalize(&parse("1/(1 - 2*M/r)"));
        let b = normalize(&parse(r"\frac{1}{1 - \frac{2M}{r}}"));
        assert_eq!(a, b);
    }

    #[test]
    fn braced_and_bare_exponents_agree() {
        assert_eq!(normalize(&parse("r^-6")), normalize(&parse("r^{-6}")));
    }

    #[test]
    fn unknown_function_is_a_parse_error() {
        let mut toks = TokStream::new("tan(x)").unwrap();
        assert!(parse_scalar_expr(&mut toks).is_err());
    }

    #[test]
    fn unknown_latex_command_is_a_parse_error() {
        let mut toks = TokStream::new(r"\nabla x").unwrap();
        assert!(parse_scalar_expr(&mut toks).is_err());
    }
}
