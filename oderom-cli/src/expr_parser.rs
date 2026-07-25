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
//! ATOM    := INT | IDENT | KNOWN_FN '(' SUM ')' | '(' SUM ')'
//!          | IDENT "'"* '(' SUM (',' SUM)* ')'          -- indeterminate function
//!          | IDENT DERIV_SUBSCRIPT? '(' SUM (',' SUM)* ')'
//!          | '\frac' '{' SUM '}' '{' SUM '}'
//!          | '\left' '(' SUM '\right' ')'
//!          | '\'KNOWN_FN ('^' SIGNED_EXP)? ('(' SUM ')' | '{' SUM '}')
//!          | '\' GREEK_NAME                      -- Var(lowercase name)
//! KNOWN_FN := 'sin' | 'cos' | 'exp' | 'sinh' | 'cosh'
//! DERIV_SUBSCRIPT := '_' '{' IDENT (',' IDENT)* '}'
//! SIGNED_EXP := ('-')? INT | '{' ('-')? INT '}'
//! ```
//!
//! `KNOWN_FN` is a closed, fixed list (`known_fn_of` below) -- `NAME(...)`
//! is always reserved syntax for a function call (never read as
//! juxtaposition-multiplication, see `parse_atom`'s `Tok::Ident` arm).
//! Any name NOT in that list, called with `(...)`, is an indeterminate
//! function instead (Marco 6 step 4, DESIGN-M6-PREP.md section 1): an
//! opaque symbol the engine never evaluates or expands, distinct from
//! both an alias (`NOME := EXPR`, which the bare, paren-less identifier
//! resolves to if declared) and a known function (which has a known
//! value and a known derivative) -- the same name can be all three at
//! once in different positions (`f` alone is the alias if declared,
//! `f(r)` is always the indeterminate function, `sin(x)` is always the
//! known one) with no collision, by construction. Its derivative is
//! written `f'(r)`/`f''(r)` (prime, single-argument only -- ambiguous
//! for more) or `h_{t,r}(t,r)` (braced, comma-separated subscript
//! naming which argument(s) were differentiated, any argument count,
//! each repeated once per order) -- never both on the same call, and
//! never on a known function (`sin'(x)` is a clean error: write
//! `diff(sin(x), x)` instead). This only REPRESENTS and DERIVES: no
//! query in this grammar solves an equation, integrates, or resolves an
//! indeterminate function for a closed form.
//!
//! `1/2` is not special-cased as a rational literal: it falls out of
//! `PRODUCT`'s ordinary division (`Mul([1, Pow(2, -1)])`), and
//! `oderom_expr::normalize` collapses that to the same `Rational(1, 2)`
//! a literal would have produced. One fewer production, same language.

use crate::error::CliError;
use crate::parser::{Tok, TokStream};
use oderom_expr::{Expr, GREEK_LETTERS};
use std::collections::{HashMap, HashSet};

/// What `parse_atom` needs to resolve a bare identifier that might be an
/// alias reference (`NOME := EXPR`, `oderom-cli/src/parser.rs`'s own
/// `parse_model`) rather than an ordinary free variable -- threaded
/// through every function in this file purely to reach that one call
/// site; nothing else in this module ever inspects it. Two pieces, not
/// one map, because "not found in `declared`" has two different honest
/// meanings: genuinely never declared as an alias anywhere in the whole
/// document (silently an ordinary free variable, exactly the behavior
/// this feature must never disturb) vs. declared as an alias LATER in
/// the document -- a forward reference, which `all_names` is what tells
/// apart from the first case, and which is a clean error rather than a
/// silent wrong answer.
pub(crate) struct AliasScope<'a> {
    pub(crate) declared: &'a HashMap<String, Expr>,
    pub(crate) all_names: &'a HashSet<String>,
}

/// What a bare identifier means, once `AliasScope::resolve` has checked
/// it against both the aliases declared so far and every alias name the
/// whole document will ever declare.
enum AliasLookup {
    /// A declared alias -- substitute its (already fully expanded, alias
    /// free) expression directly. This is the entire feature: by the
    /// time this `Expr` is returned, nothing downstream (`normalize`,
    /// the cache, ...) can tell an alias was ever involved.
    Expand(Expr),
    /// Declared as an alias somewhere in this same document, but not yet
    /// -- a forward reference.
    ForwardReference,
    /// Not a known alias at all -- an ordinary free variable/coordinate/
    /// parameter, exactly as before this feature existed.
    Unknown,
}

impl<'a> AliasScope<'a> {
    fn resolve(&self, name: &str) -> AliasLookup {
        if let Some(expr) = self.declared.get(name) {
            AliasLookup::Expand(expr.clone())
        } else if self.all_names.contains(name) {
            AliasLookup::ForwardReference
        } else {
            AliasLookup::Unknown
        }
    }
}

pub(crate) fn parse_scalar_expr(toks: &mut TokStream, aliases: &AliasScope) -> Result<Expr, CliError> {
    parse_sum(toks, aliases)
}

fn parse_sum(toks: &mut TokStream, aliases: &AliasScope) -> Result<Expr, CliError> {
    let mut terms = vec![parse_product(toks, aliases)?];
    loop {
        match toks.peek() {
            Tok::Sym('+') => {
                toks.advance();
                terms.push(parse_product(toks, aliases)?);
            }
            Tok::Sym('-') => {
                toks.advance();
                terms.push(Expr::int(-1) * parse_product(toks, aliases)?);
            }
            _ => break,
        }
    }
    Ok(one_or_add(terms))
}

fn parse_product(toks: &mut TokStream, aliases: &AliasScope) -> Result<Expr, CliError> {
    let mut factors = vec![parse_unary(toks, aliases)?];
    loop {
        match toks.peek().clone() {
            Tok::Sym('*') => {
                toks.advance();
                factors.push(parse_unary(toks, aliases)?);
            }
            Tok::Sym('/') => {
                toks.advance();
                factors.push(Expr::Pow(Box::new(parse_unary(toks, aliases)?), -1));
            }
            // Juxtaposition ("2M", "r^2\sin^2(\theta)"): no explicit `*`
            // between two atoms is still multiplication, as long as the
            // next token unambiguously starts one (never `+`/`-`, which
            // always belong to `SUM` -- "2 - M" stays subtraction) AND
            // isn't actually the start of a brand new top-level
            // statement -- the one case an identifier can fail that even
            // though `starts_atom` alone would accept it (see
            // `crate::parser::starts_new_top_level_statement`'s own doc
            // comment for the alias-declaration boundary problem this
            // guards against).
            t if starts_atom(&t) && !matches!(&t, Tok::Ident(name) if crate::parser::starts_new_top_level_statement(toks, name)) => {
                factors.push(parse_unary(toks, aliases)?);
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

fn parse_unary(toks: &mut TokStream, aliases: &AliasScope) -> Result<Expr, CliError> {
    if *toks.peek() == Tok::Sym('-') {
        toks.advance();
        Ok(Expr::int(-1) * parse_unary(toks, aliases)?)
    } else {
        parse_power(toks, aliases)
    }
}

fn parse_power(toks: &mut TokStream, aliases: &AliasScope) -> Result<Expr, CliError> {
    let base = parse_atom(toks, aliases)?;
    if *toks.peek() == Tok::Sym('^') {
        toks.advance();
        let exp = parse_signed_exponent(toks)?;
        Ok(Expr::Pow(Box::new(base), exp))
    } else {
        Ok(base)
    }
}

fn parse_atom(toks: &mut TokStream, aliases: &AliasScope) -> Result<Expr, CliError> {
    match toks.peek().clone() {
        Tok::Int(n) => {
            toks.advance();
            Ok(Expr::int(n as i64))
        }
        Tok::Ident(name) => {
            toks.advance();
            // A derivative mark right after the name, before the `(` --
            // `f'`/`f''` (prime, single-argument only) or `h_{t,r}`
            // (braced subscript, any argument count). Consumed
            // unconditionally here (not gated on whether `name` turns
            // out to be a known function or an indeterminate one) so
            // both get the same clean rejection below rather than one
            // silently swallowing the mark and the other choking on a
            // stray token.
            let prime_order = consume_primes(toks);
            let subscript =
                if *toks.peek() == Tok::Sym('_') { Some(parse_derivative_subscript(toks)?) } else { None };
            let has_deriv_mark = prime_order > 0 || subscript.is_some();

            if *toks.peek() == Tok::Sym('(') {
                if let Some(f) = known_fn_of(&name) {
                    if has_deriv_mark {
                        return Err(toks.error(format!(
                            "`{name}` is a known function with a known derivative -- write `diff({name}(...), var)`, not `{name}'(...)` or `{name}_{{...}}(...)`"
                        )));
                    }
                    toks.expect_sym('(')?;
                    let arg = parse_sum(toks, aliases)?;
                    toks.expect_sym(')')?;
                    return Ok(apply_known_fn(f, arg));
                }
                // A name with an expected standard meaning (`tan`,
                // `sqrt`, `log`, an inverse trig function, ...) that
                // just isn't wired up as callable yet: refused, never
                // silently accepted as an indeterminate function. This
                // is the whole reason `RESERVED_FN_NAMES` exists --
                // without it, `tan(theta)` would parse as an opaque
                // symbol named "tan" and differentiate as an ordinary
                // indeterminate function (`tan'`, not the real `sec^2`),
                // a plausible-looking but silently wrong answer for
                // anyone who meant the real tangent. See this module's
                // header doc and `RESERVED_FN_NAMES`'s own doc comment.
                if RESERVED_FN_NAMES.contains(&name.as_str()) {
                    return Err(toks.error(format!(
                        "`{name}` is a known elementary function, not yet implemented as callable -- reserved names cannot be used as an indeterminate function"
                    )));
                }
                // `NOME(...)`, `NOME` not a known function: an
                // indeterminate function call (DESIGN-M6-PREP.md
                // section 1) -- never alias lookup, even if `name`
                // happens to also be a declared alias. This is exactly
                // what the alias feature's own reservation of
                // `NOME(...)` syntax was for: an alias can never occupy
                // this position, by construction, not by convention, so
                // the same name can be both a bare-identifier alias and
                // a parenthesized indeterminate function with no
                // collision.
                toks.expect_sym('(')?;
                let mut args = vec![parse_sum(toks, aliases)?];
                while *toks.peek() == Tok::Sym(',') {
                    toks.advance();
                    args.push(parse_sum(toks, aliases)?);
                }
                toks.expect_sym(')')?;
                build_func_call(toks, name, args, prime_order, subscript)
            } else if has_deriv_mark {
                Err(toks.error(format!("`{name}` with a derivative mark must be followed by an argument list, e.g. `{name}'(r)` or `{name}_{{r}}(r)`")))
            } else {
                match aliases.resolve(&name) {
                    AliasLookup::Expand(expr) => Ok(expr),
                    AliasLookup::ForwardReference => {
                        Err(toks.error(format!("alias `{name}` is used before its own `{name} := ...` declaration")))
                    }
                    AliasLookup::Unknown => Ok(Expr::var(name)),
                }
            }
        }
        Tok::Sym('(') => {
            toks.advance();
            let e = parse_sum(toks, aliases)?;
            toks.expect_sym(')')?;
            Ok(e)
        }
        Tok::Command(name) => parse_command_atom(toks, &name, aliases),
        other => Err(toks.error(format!("expected a scalar expression, found {other:?}"))),
    }
}

/// Consumes zero or more consecutive `'` right after a function name --
/// `f'`, `f''` -- returning the count (the derivative order they
/// denote). Unambiguous only for a single-argument function
/// (`build_func_call` rejects it otherwise); never touches anything but
/// this one position, so `'` anywhere else in the grammar is unaffected.
fn consume_primes(toks: &mut TokStream) -> u32 {
    let mut n = 0u32;
    while *toks.peek() == Tok::Sym('\'') {
        toks.advance();
        n += 1;
    }
    n
}

/// `_{name (',' name)*}` -- the unambiguous multivariable partial-
/// derivative subscript (DESIGN-M6-PREP.md section 1: prime notation is
/// ambiguous for more than one argument, so this is the "algo
/// inequívoco" it asks for). Always braced, always comma-separated,
/// deliberately never the glued/backtracking-decomposed form
/// `index_resolve.rs`'s own `NAME_{...}` metric/connection-index syntax
/// also accepts: that decomposition needs a `Chart` to backtrack
/// against, and `expr_parser.rs` is deliberately chart-agnostic (no
/// `Chart` in scope anywhere in this file) -- concatenating
/// multi-character coordinate names (`theta`, `phi`) with no separator
/// would also be ambiguous to split back apart. Comma is the one
/// separator this grammar never needs a glued alternative for.
fn parse_derivative_subscript(toks: &mut TokStream) -> Result<Vec<String>, CliError> {
    toks.expect_sym('_')?;
    toks.expect_sym('{')?;
    let mut names = vec![toks.ident()?];
    while *toks.peek() == Tok::Sym(',') {
        toks.advance();
        names.push(toks.ident()?);
    }
    toks.expect_sym('}')?;
    Ok(names)
}

/// Combines a parsed indeterminate-function call's argument list with
/// its derivative mark(s) -- at most one of `prime_order`/`subscript`
/// is ever meaningfully nonempty for a given call (the parser lets both
/// be attempted, but prime notation only ever survives past this point
/// for a single-argument function; see `parse_atom`'s own comment on
/// why both are consumed unconditionally regardless of which the
/// callee turns out to need) -- into the one `order` multi-index
/// `Expr::Func` stores (`order[i]` = how many times differentiated wrt
/// `args[i]`).
fn build_func_call(toks: &TokStream, name: String, args: Vec<Expr>, prime_order: u32, subscript: Option<Vec<String>>) -> Result<Expr, CliError> {
    let mut order = vec![0u32; args.len()];
    if prime_order > 0 {
        if args.len() != 1 {
            return Err(toks.error(format!(
                "`{name}'` (prime notation) is ambiguous for a function of {} arguments -- use `{name}_{{var,...}}(...)` instead",
                args.len()
            )));
        }
        order[0] = prime_order;
    }
    if let Some(names) = subscript {
        for var_name in names {
            let Some(pos) = args.iter().position(|a| matches!(a, Expr::Var(v) if *v == var_name)) else {
                return Err(toks.error(format!("`{name}`'s derivative subscript names `{var_name}`, which is not one of its own arguments")));
            };
            order[pos] += 1;
        }
    }
    Ok(Expr::Func { name, args, order })
}

fn parse_command_atom(toks: &mut TokStream, name: &str, aliases: &AliasScope) -> Result<Expr, CliError> {
    toks.advance();
    match name {
        "frac" => {
            toks.expect_sym('{')?;
            let num = parse_sum(toks, aliases)?;
            toks.expect_sym('}')?;
            toks.expect_sym('{')?;
            let den = parse_sum(toks, aliases)?;
            toks.expect_sym('}')?;
            Ok(num * Expr::Pow(Box::new(den), -1))
        }
        "left" => {
            toks.expect_sym('(')?;
            let e = parse_sum(toks, aliases)?;
            toks.expect_command("right")?;
            toks.expect_sym(')')?;
            Ok(e)
        }
        "sin" | "cos" | "exp" | "sinh" | "cosh" => {
            let f = known_fn_of(name).expect("matched one of the known function names above");
            let power = if *toks.peek() == Tok::Sym('^') {
                toks.advance();
                Some(parse_signed_exponent(toks)?)
            } else {
                None
            };
            let arg = parse_group(toks, aliases)?;
            let applied = apply_known_fn(f, arg);
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
                Err(toks.error(format!("unknown LaTeX command `\\{name}`")))
            }
        }
    }
}

/// `'(' SUM ')'` or `'{' SUM '}'` -- the argument of `\sin`/`\cos`, which
/// LaTeX writers spell either way.
fn parse_group(toks: &mut TokStream, aliases: &AliasScope) -> Result<Expr, CliError> {
    match toks.peek().clone() {
        Tok::Sym('(') => {
            toks.advance();
            let e = parse_sum(toks, aliases)?;
            toks.expect_sym(')')?;
            Ok(e)
        }
        Tok::Sym('{') => {
            toks.advance();
            let e = parse_sum(toks, aliases)?;
            toks.expect_sym('}')?;
            Ok(e)
        }
        other => Err(toks.error(format!("expected `(` or `{{`, found {other:?}"))),
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

/// The closed set of transcendental functions this grammar accepts --
/// see this module's own header doc for why `NAME(...)` is reserved
/// syntax regardless of whether `NAME` is in this list.
#[derive(Clone, Copy)]
enum KnownFn {
    Sin,
    Cos,
    Exp,
    Sinh,
    Cosh,
}

fn known_fn_of(name: &str) -> Option<KnownFn> {
    match name {
        "sin" => Some(KnownFn::Sin),
        "cos" => Some(KnownFn::Cos),
        "exp" => Some(KnownFn::Exp),
        "sinh" => Some(KnownFn::Sinh),
        "cosh" => Some(KnownFn::Cosh),
        _ => None,
    }
}

fn apply_known_fn(f: KnownFn, arg: Expr) -> Expr {
    match f {
        KnownFn::Sin => arg.sin(),
        KnownFn::Cos => arg.cos(),
        KnownFn::Exp => arg.exp(),
        KnownFn::Sinh => arg.sinh(),
        KnownFn::Cosh => arg.cosh(),
    }
}

/// Elementary functions a physicist would write expecting the standard
/// textbook meaning, not yet wired up as a `KnownFn` -- reserved so
/// `tan(theta)` is a clean, honest error ("not yet implemented") rather
/// than silently succeeding as an indeterminate function named "tan",
/// which would differentiate as an ordinary opaque `tan'` instead of the
/// real `sec^2` and hand back a plausible-looking, silently wrong
/// answer to anyone who meant the real tangent -- exactly the failure
/// class this project has been burned by before, and a real regression
/// against the honest `unknown function` rejection this same call used
/// to get before indeterminate functions existed (Marco 6 step 4).
/// `f`, `g`, `h`, and every other name with no standard mathematical
/// meaning are unaffected -- still ordinary, valid indeterminate
/// functions, exactly as today.
///
/// This is a "not yet" list, not a "never" one: every name here is a
/// real function this project could plausibly implement as a `KnownFn`
/// in a later round (it would move out of this list into
/// `known_fn_of`, the same way `exp`/`sinh`/`cosh` already did in an
/// earlier one) -- reserving the name now is what keeps that door open
/// without letting the name mean something else (an opaque
/// indeterminate function) in the meantime.
///
/// The set: the reciprocal trig functions (`tan`/`cot`/`sec`/`csc`) and
/// their hyperbolic analogues (`tanh`/`coth`/`sech`/`csch`); both common
/// spellings of the inverse trig and inverse hyperbolic functions
/// (`asin`/`arcsin`, `acos`/`arccos`, `atan`/`arctan`, `asinh`/
/// `arcsinh`, `acosh`/`arccosh`, `atanh`/`arctanh`); `log`/`ln`
/// (logarithm, either spelling -- which base `log` alone would mean is
/// exactly the kind of decision this project should make deliberately
/// when it actually implements it, not by accident of what got left
/// out of this list); `sqrt` (square root); `atan2` (two-argument
/// arctangent, common in physics for recovering an angle); `abs`
/// (absolute value). Deliberately excludes non-differentiable/discrete
/// functions with no natural derivative (`floor`, `ceil`, `sign`) --
/// out of scope for a project centered on differentiable curvature
/// computations, and not the failure mode this list exists to close.
const RESERVED_FN_NAMES: &[&str] = &[
    "tan", "cot", "sec", "csc", "tanh", "coth", "sech", "csch", "asin", "arcsin", "acos", "arccos", "atan", "arctan",
    "asinh", "arcsinh", "acosh", "arccosh", "atanh", "arctanh", "log", "ln", "sqrt", "atan2", "abs",
];

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

    fn no_aliases() -> AliasScope<'static> {
        // `Box::leak`: test-only, and only ever called a handful of
        // times per test run -- the simplest way to hand out `&'static`
        // empty maps without a shared global, which would be the wrong
        // kind of "simple" (state leaking between tests that don't
        // otherwise share anything).
        AliasScope {
            declared: Box::leak(Box::new(HashMap::new())),
            all_names: Box::leak(Box::new(HashSet::new())),
        }
    }

    fn parse(src: &str) -> Expr {
        let mut toks = TokStream::new(src).unwrap();
        parse_scalar_expr(&mut toks, &no_aliases()).unwrap()
    }

    fn parse_with_aliases(src: &str, declared: &HashMap<String, Expr>, all_names: &HashSet<String>) -> Result<Expr, CliError> {
        let mut toks = TokStream::new(src).unwrap();
        parse_scalar_expr(&mut toks, &AliasScope { declared, all_names })
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
    fn exp_sinh_cosh_parse_in_ascii_and_latex_and_agree() {
        for (ascii_name, latex_name) in [("exp", "exp"), ("sinh", "sinh"), ("cosh", "cosh")] {
            let ascii = normalize(&parse(&format!("{ascii_name}(H*t)")));
            let latex = normalize(&parse(&format!(r"\{latex_name}(H*t)")));
            assert_eq!(ascii, latex, "{ascii_name}");
        }
    }

    #[test]
    fn cosh_squared_latex_power_form_matches_ascii() {
        // The same `\sin^2(...)`-style power-attached-to-function LaTeX
        // form sin/cos already get (parse_command_atom's "sin"|"cos"
        // arm) generalizes for free to exp/sinh/cosh -- same code path,
        // not a new syntax form.
        let ascii = normalize(&parse("cosh(chi)^2"));
        let latex = normalize(&parse(r"\cosh^2(\chi)"));
        assert_eq!(ascii, latex);
    }

    #[test]
    fn name_call_syntax_is_reserved_even_for_unknown_names() {
        // `f(x)` is read as a function call attempt, never juxtaposition
        // (`f * x`) -- and, as of Marco 6 step 4 (DESIGN-M6-PREP.md
        // section 1), a name not in the closed known-function list is no
        // longer a parse error here at all: it is accepted as an
        // indeterminate function, order zero (not yet differentiated) in
        // its one argument.
        let result = parse("f(x)");
        assert_eq!(result, Expr::Func { name: "f".to_string(), args: vec![Expr::var("x")], order: vec![0] });
    }

    #[test]
    fn braced_and_bare_exponents_agree() {
        assert_eq!(normalize(&parse("r^-6")), normalize(&parse("r^{-6}")));
    }

    #[test]
    fn a_name_with_no_standard_mathematical_meaning_is_an_indeterminate_function_not_an_error() {
        // `tan` was this module's own probe for "definitely not
        // sin/cos/exp/sinh/cosh" before this round -- it no longer
        // works as that probe (see the RESERVED_FN_NAMES tests below:
        // `tan` carries a real, expected meaning, so it is reserved,
        // not indeterminate). A name with no standard mathematical
        // meaning at all -- `blorp`, `f`, `g`, `h` -- is still, and
        // must remain, a perfectly ordinary indeterminate function.
        let result = parse("blorp(x)");
        assert_eq!(result, Expr::Func { name: "blorp".to_string(), args: vec![Expr::var("x")], order: vec![0] });
    }

    #[test]
    fn an_indeterminate_function_call_with_a_bad_argument_is_still_a_parse_error() {
        // The call syntax itself is still real grammar, not a blanket
        // "anything goes": an unterminated/malformed argument list is
        // still rejected, same as it always was for a known function.
        let mut toks = TokStream::new("f(x +)").unwrap();
        assert!(parse_scalar_expr(&mut toks, &no_aliases()).is_err());
    }

    #[test]
    fn unknown_latex_command_is_a_parse_error() {
        let mut toks = TokStream::new(r"\nabla x").unwrap();
        assert!(parse_scalar_expr(&mut toks, &no_aliases()).is_err());
    }

    #[test]
    fn a_declared_alias_expands_inline() {
        let mut declared = HashMap::new();
        declared.insert("f".to_string(), normalize(&(Expr::one() - Expr::int(2) * Expr::var("M") / Expr::var("r"))));
        let all_names: HashSet<String> = ["f".to_string()].into_iter().collect();

        let expanded = parse_with_aliases("-f", &declared, &all_names).unwrap();
        let direct = parse("-(1 - 2*M/r)");
        assert_eq!(normalize(&expanded), normalize(&direct));
    }

    #[test]
    fn an_alias_can_appear_inside_a_function_argument_and_a_larger_expression() {
        let mut declared = HashMap::new();
        declared.insert("f".to_string(), normalize(&Expr::var("r").pow(2)));
        let all_names: HashSet<String> = ["f".to_string()].into_iter().collect();

        let expanded = parse_with_aliases("sin(f) + 1/f", &declared, &all_names).unwrap();
        let direct = parse("sin(r^2) + 1/r^2");
        assert_eq!(normalize(&expanded), normalize(&direct));
    }

    #[test]
    fn an_alias_can_reference_an_earlier_alias_since_declared_ones_are_already_expanded() {
        // Mirrors exactly how `parser.rs::parse_alias_decl` builds
        // `declared` incrementally: by the time `g` is parsed, `f`'s OWN
        // stored value is already fully expanded (no alias reference
        // left inside it) -- so looking `f` up while parsing `g`'s body
        // needs no recursive expansion at lookup time at all.
        let mut declared = HashMap::new();
        declared.insert("f".to_string(), normalize(&Expr::var("r").pow(2)));
        let all_names: HashSet<String> = ["f".to_string(), "g".to_string()].into_iter().collect();

        let g_body = parse_with_aliases("f + 1", &declared, &all_names).unwrap();
        assert_eq!(normalize(&g_body), normalize(&(Expr::var("r").pow(2) + Expr::one())));
    }

    #[test]
    fn a_name_never_declared_as_an_alias_anywhere_stays_an_ordinary_free_variable() {
        // No behavior change for the overwhelming common case (M, Q, H,
        // ...): a name that is not, anywhere in the document, the target
        // of a `:=` is treated exactly as before this feature existed.
        let declared = HashMap::new();
        let all_names = HashSet::new();
        let result = parse_with_aliases("M + 1", &declared, &all_names).unwrap();
        assert_eq!(result, Expr::var("M") + Expr::one());
    }

    #[test]
    fn a_name_declared_as_an_alias_later_in_the_document_is_a_clean_error_not_a_free_variable() {
        // The forward-reference case: `f` WILL be declared as an alias
        // somewhere in this document (`all_names` says so), but is not
        // in `declared` yet at this point -- must error, never silently
        // fall back to `Expr::var("f")`.
        let declared = HashMap::new();
        let all_names: HashSet<String> = ["f".to_string()].into_iter().collect();
        let err = parse_with_aliases("-f", &declared, &all_names).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("used before its own `f := ...` declaration"), "{message}");
    }

    #[test]
    fn parenthesized_use_of_a_name_that_is_also_a_declared_alias_is_the_indeterminate_function_never_the_alias() {
        // `f(x)` must never be read as "call the alias like a function"
        // -- this was already true before Marco 6 step 4 (the
        // parenthesized position was reserved, not yet occupied); now
        // that the reservation is filled, the invariant is the same,
        // just with a positive assertion instead of a rejection: `f(x)`
        // parses as the indeterminate function `f` applied to `x`,
        // completely ignoring `f`'s stored alias expression (`r`) --
        // the alias and the indeterminate function are two different
        // things that happen to share a name, distinguished purely by
        // whether parens follow, never colliding (the "não colapse os
        // três" invariant: alias, known function, indeterminate
        // function never fold into each other).
        let mut declared = HashMap::new();
        declared.insert("f".to_string(), Expr::var("r"));
        let all_names: HashSet<String> = ["f".to_string()].into_iter().collect();
        let result = parse_with_aliases("f(x)", &declared, &all_names).unwrap();
        assert_eq!(result, Expr::Func { name: "f".to_string(), args: vec![Expr::var("x")], order: vec![0] });
    }

    // -----------------------------------------------------------------
    // Marco 6 step 4: indeterminate functions and their derivative
    // notation (DESIGN-M6-PREP.md section 1)
    // -----------------------------------------------------------------

    #[test]
    fn single_prime_parses_as_derivative_order_one() {
        let result = parse("f'(r)");
        assert_eq!(result, Expr::Func { name: "f".to_string(), args: vec![Expr::var("r")], order: vec![1] });
    }

    #[test]
    fn double_prime_parses_as_derivative_order_two() {
        let result = parse("f''(r)");
        assert_eq!(result, Expr::Func { name: "f".to_string(), args: vec![Expr::var("r")], order: vec![2] });
    }

    #[test]
    fn multi_argument_function_parses_with_one_order_zero_slot_per_argument() {
        let result = parse("h(t, r)");
        assert_eq!(result, Expr::Func { name: "h".to_string(), args: vec![Expr::var("t"), Expr::var("r")], order: vec![0, 0] });
    }

    #[test]
    fn braced_subscript_marks_the_named_argument_as_differentiated_once() {
        let result = parse("h_{t}(t, r)");
        assert_eq!(result, Expr::Func { name: "h".to_string(), args: vec![Expr::var("t"), Expr::var("r")], order: vec![1, 0] });
        let result = parse("h_{r}(t, r)");
        assert_eq!(result, Expr::Func { name: "h".to_string(), args: vec![Expr::var("t"), Expr::var("r")], order: vec![0, 1] });
    }

    #[test]
    fn braced_subscript_can_repeat_a_variable_for_a_higher_order_partial() {
        // h_{t,t}(t,r) = d^2h/dt^2 -- the subscript form generalizes to
        // higher order the same way prime marks do for one argument.
        let result = parse("h_{t,t}(t, r)");
        assert_eq!(result, Expr::Func { name: "h".to_string(), args: vec![Expr::var("t"), Expr::var("r")], order: vec![2, 0] });
    }

    #[test]
    fn braced_subscript_handles_a_genuine_mixed_partial() {
        let result = parse("h_{t,r}(t, r)");
        assert_eq!(result, Expr::Func { name: "h".to_string(), args: vec![Expr::var("t"), Expr::var("r")], order: vec![1, 1] });
    }

    #[test]
    fn prime_notation_on_a_multi_argument_function_is_a_clean_ambiguity_error() {
        let mut toks = TokStream::new("h'(t, r)").unwrap();
        let err = parse_scalar_expr(&mut toks, &no_aliases()).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("ambiguous"), "{message}");
        assert!(message.contains("h_{var,...}"), "{message}");
    }

    #[test]
    fn a_subscript_variable_not_among_the_arguments_is_a_clean_error() {
        let mut toks = TokStream::new("h_{x}(t, r)").unwrap();
        let err = parse_scalar_expr(&mut toks, &no_aliases()).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("`x`"), "{message}");
        assert!(message.contains("not one of its own arguments"), "{message}");
    }

    #[test]
    fn a_known_function_with_a_prime_is_a_clean_error_not_a_silent_derivative() {
        // sin/cos/exp/sinh/cosh already have a known derivative rule
        // (`oderom_expr::diff`) -- prime notation on one of them would
        // either silently mean something different than it does for an
        // indeterminate function, or require guessing; rejected instead,
        // naming the real way to get a known function's derivative.
        let mut toks = TokStream::new("sin'(x)").unwrap();
        let err = parse_scalar_expr(&mut toks, &no_aliases()).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("known derivative"), "{message}");
        assert!(message.contains("diff(sin(...), var)"), "{message}");
    }

    #[test]
    fn a_known_function_with_a_derivative_subscript_is_also_a_clean_error() {
        let mut toks = TokStream::new("sin_{x}(x)").unwrap();
        let err = parse_scalar_expr(&mut toks, &no_aliases()).unwrap_err();
        assert!(err.to_string().contains("known derivative"));
    }

    #[test]
    fn a_derivative_mark_with_no_following_argument_list_is_a_clean_error() {
        // `f'` alone (no call at all) must not be silently read as some
        // other token sequence -- it names exactly what is missing.
        let mut toks = TokStream::new("f' + 1").unwrap();
        let err = parse_scalar_expr(&mut toks, &no_aliases()).unwrap_err();
        assert!(err.to_string().contains("must be followed by an argument list"));
    }

    #[test]
    fn an_indeterminate_function_can_appear_inside_a_larger_expression_and_derives_correctly_downstream() {
        // f(r)^2 parses as Pow(Func, 2) -- the same juxtaposition/power
        // grammar any other atom already gets, no special case needed
        // for this atom to compose with the rest of SCALAR_EXPR.
        let result = normalize(&parse("f(r)^2 + 1"));
        let expected = normalize(&(Expr::Func { name: "f".to_string(), args: vec![Expr::var("r")], order: vec![0] }.pow(2) + Expr::one()));
        assert_eq!(result, expected);
    }

    #[test]
    fn distinct_indeterminate_functions_stay_distinct_after_normalize() {
        // f(r) and g(r) must never collapse into each other or cancel --
        // an indeterminate function has no identity relating it to any
        // other (DESIGN-M6-PREP.md section 1).
        let result = normalize(&(parse("f(r)") - parse("g(r)")));
        assert_ne!(result, Expr::zero());
    }

    // -----------------------------------------------------------------
    // Reserved elementary-function names: a name with an expected
    // standard meaning (`tan`, `sqrt`, an inverse trig function, ...)
    // must never silently become an opaque indeterminate function --
    // that produced a plausible-looking but wrong answer for `tan`
    // specifically (differentiating as an ordinary indeterminate `tan'`
    // instead of the real `sec^2`), caught before it shipped.
    // -----------------------------------------------------------------

    #[test]
    fn a_reserved_elementary_function_name_is_a_clean_error_not_an_opaque_function() {
        let mut toks = TokStream::new("tan(theta)").unwrap();
        let err = parse_scalar_expr(&mut toks, &no_aliases()).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("not yet implemented as callable"), "{message}");
        assert!(message.contains("reserved names cannot be used as an indeterminate function"), "{message}");
    }

    #[test]
    fn every_listed_reserved_name_is_rejected_not_silently_accepted() {
        // Every name in RESERVED_FN_NAMES, called with one argument,
        // must be rejected -- not spot-checked on just `tan`, since the
        // whole point of a reserved list is that nothing on it is
        // missed.
        for name in RESERVED_FN_NAMES {
            let src = format!("{name}(x)");
            let mut toks = TokStream::new(&src).unwrap();
            let err = parse_scalar_expr(&mut toks, &no_aliases());
            assert!(err.is_err(), "expected `{src}` to be rejected as reserved, got {err:?}");
        }
    }

    #[test]
    fn a_reserved_name_with_a_derivative_mark_is_still_the_reserved_error_not_the_known_derivative_one() {
        // `tan` is not in `known_fn_of` (it isn't callable at all yet),
        // so a derivative mark on it must report the reservation, never
        // the "known function with a known derivative" message that
        // only makes sense for a function the engine actually knows how
        // to differentiate.
        let mut toks = TokStream::new("tan'(theta)").unwrap();
        let err = parse_scalar_expr(&mut toks, &no_aliases()).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("not yet implemented as callable"), "{message}");
        assert!(!message.contains("known derivative"), "{message}");
    }

    #[test]
    fn a_reserved_name_and_a_genuinely_unknown_name_behave_per_their_own_class_side_by_side() {
        // The acceptance test for this whole feature: in the SAME
        // expression, `tan(theta)` (reserved -- a real function not yet
        // implemented) must error, while `f(r)` (no standard meaning --
        // still a valid indeterminate function) must not, and the two
        // must not affect each other's outcome.
        let f_ok = parse("f(r)");
        assert_eq!(f_ok, Expr::Func { name: "f".to_string(), args: vec![Expr::var("r")], order: vec![0] });

        let mut toks = TokStream::new("f(r) + tan(theta)").unwrap();
        let err = parse_scalar_expr(&mut toks, &no_aliases()).unwrap_err();
        assert!(err.to_string().contains("not yet implemented as callable"));

        let mut toks2 = TokStream::new("tan(theta) + f(r)").unwrap();
        let err2 = parse_scalar_expr(&mut toks2, &no_aliases()).unwrap_err();
        assert!(err2.to_string().contains("not yet implemented as callable"), "order must not matter: {err2}");
    }

    #[test]
    fn reserved_names_include_both_spellings_of_the_inverse_functions_and_the_hyperbolic_reciprocals() {
        // Spot checks beyond `tan` alone: both common spellings of an
        // inverse function (a physicist could write either), a
        // hyperbolic reciprocal, log/ln, sqrt, and the two-argument
        // atan2 -- confirms the list is genuinely the breadth described
        // in its own doc comment, not just the one name every other
        // test happens to probe.
        for name in ["asin", "arcsin", "atanh", "coth", "log", "ln", "sqrt", "abs"] {
            assert!(RESERVED_FN_NAMES.contains(&name), "expected `{name}` to be reserved");
        }
        let mut toks = TokStream::new("atan2(y, x)").unwrap();
        assert!(parse_scalar_expr(&mut toks, &no_aliases()).is_err(), "atan2 (two arguments) must also be reserved");
    }
}
