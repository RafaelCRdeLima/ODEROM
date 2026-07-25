//! [`Render`] for [`Expr`]: infix Unicode text, LaTeX math source, and a
//! hand-written JSON encoding (see `oderom_core::render` for why these
//! three targets and why the trait lives in `oderom-core`). `Display`
//! is a thin wrapper over `render(Target::Unicode)`.
//!
//! Unicode and LaTeX share the same idea: an `Expr` is rendered
//! bottom-up into `(precedence level, text)` pairs, and a child is
//! parenthesized exactly when its own level is lower than the minimum
//! its parent requires (`Add` < `Mul` < `Pow`'s base < atoms). `Mul`
//! additionally splits its factors into a numerator and a denominator
//! (any `Pow(_, negative exponent)` factor moves to the denominator
//! with its exponent negated) so that e.g. `Kretschmann` renders as
//! `48*M^2/r^6`, not `48*M^2*r^-6`.

use crate::{BigScalar, Expr};
use oderom_core::{Render, Target};
use std::fmt;

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.render(Target::Unicode))
    }
}

impl Render for Expr {
    fn render(&self, target: Target) -> String {
        match target {
            Target::Unicode => unicode(self, SUM),
            Target::Latex => latex(self, SUM),
            Target::Json => json(self),
            Target::Mathematica => mathematica(self, SUM),
            Target::Sympy => sympy(self, SUM),
        }
    }
}

const SUM: u8 = 0;
const PRODUCT: u8 = 1;
const POWER: u8 = 2;
const ATOM: u8 = 3;

fn parenthesize_if(body: String, level: u8, min: u8) -> String {
    if level < min {
        format!("({body})")
    } else {
        body
    }
}

/// Splits `s` into `(is_negative, magnitude)`, e.g. `"-3/4"` ->
/// `(true, "3/4")`. Used to turn `Add`'s `+`-joined terms into proper
/// `a - b` text without rebuilding any `Expr` trees -- a rendered
/// factor/term already starts with `-` exactly when its value is
/// negative, by construction of [`unicode_mul`]/[`latex_mul`].
fn split_sign(s: String) -> (bool, String) {
    match s.strip_prefix('-') {
        Some(rest) => (true, rest.to_string()),
        None => (false, s),
    }
}

fn join_signed(parts: Vec<String>, on_empty: &str) -> String {
    if parts.is_empty() {
        return on_empty.to_string();
    }
    let mut out = String::new();
    for (i, part) in parts.into_iter().enumerate() {
        let (negative, magnitude) = split_sign(part);
        if i == 0 {
            if negative {
                out.push('-');
            }
        } else {
            out.push_str(if negative { " - " } else { " + " });
        }
        out.push_str(&magnitude);
    }
    out
}

// ---------------------------------------------------------------------
// Unicode
// ---------------------------------------------------------------------

fn unicode(e: &Expr, min: u8) -> String {
    let (level, body) = match e {
        Expr::Rational(s) => (ATOM, s.render(Target::Unicode)),
        Expr::Var(name) => (ATOM, name.clone()),
        Expr::Pow(base, exp) => (POWER, format!("{}^{exp}", unicode(base, ATOM))),
        Expr::Sin(x) => (ATOM, format!("sin({})", unicode(x, SUM))),
        Expr::Cos(x) => (ATOM, format!("cos({})", unicode(x, SUM))),
        Expr::Exp(x) => (ATOM, format!("exp({})", unicode(x, SUM))),
        Expr::Sinh(x) => (ATOM, format!("sinh({})", unicode(x, SUM))),
        Expr::Cosh(x) => (ATOM, format!("cosh({})", unicode(x, SUM))),
        Expr::Func { name, args, order } => (ATOM, unicode_func(name, args, order)),
        Expr::Mul(factors) => (PRODUCT, unicode_mul(factors)),
        Expr::Add(terms) => {
            let mut flat = Vec::new();
            flatten_add(terms, &mut flat);
            (SUM, join_signed(flat.iter().map(|t| unicode(t, PRODUCT)).collect(), "0"))
        }
    };
    parenthesize_if(body, level, min)
}

fn unicode_mul(factors: &[Expr]) -> String {
    let mut flat = Vec::new();
    flatten_mul(factors, &mut flat);
    let (sign_negative, coeff, num, den) = split_mul(&flat, unicode, |b, e| format!("{b}^{e}"));
    assemble_mul(sign_negative, coeff.map(|c| c.render(Target::Unicode)), num, den, "*")
}

/// `f(r)`, `f'(r)`, `f''(r)` (a single argument: derivative order shown
/// as trailing prime marks -- unambiguous only with exactly one
/// argument, which is why the parser only ever accepts this notation
/// for a single-argument function, see `oderom-cli/src/expr_parser.rs`).
/// `h(t,r)`, `h_t(t,r)`, `h_tr(t,r)` (more than one argument: no prime,
/// which variable(s) were differentiated is spelled out as a
/// concatenated subscript instead, order matching `args`' own order,
/// each name repeated `order[i]` times -- e.g. `order=[2,1]` on `h(t,r)`
/// renders `h_ttr`). Plain text, so no control-word concatenation risk
/// the LaTeX form below has to guard against.
fn unicode_func(name: &str, args: &[Expr], order: &[u32]) -> String {
    let args_str = args.iter().map(|a| unicode(a, SUM)).collect::<Vec<_>>().join(",");
    let marks = if args.len() == 1 {
        "'".repeat(order[0] as usize)
    } else {
        func_subscript(args, order)
    };
    format!("{name}{marks}({args_str})")
}

/// The name to show in a derivative subscript for argument `i` -- the
/// argument's own bare variable name when it is one (the realistic
/// case: an indeterminate function's arguments are chart coordinates,
/// e.g. `h(t, r)`), or a positional fallback (`arg1`, `arg2`, ...) for
/// anything else, since there is no other stable name to point at (this
/// grammar has no separate "function signature" declaration to read a
/// parameter name from -- see `oderom-cli/src/expr_parser.rs`'s own doc
/// comment on this same constraint).
fn func_arg_subscript_name(arg: &Expr, position: usize) -> String {
    match arg {
        Expr::Var(name) => name.clone(),
        _ => format!("arg{}", position + 1),
    }
}

/// Shared by `unicode_func`/`latex_func`: the list of subscript pieces
/// (one per differentiation, in argument order, each argument's own
/// name repeated `order[i]` times), before either target decides how to
/// join them (bare concatenation for Unicode; space-separated inside
/// `_{...}` for LaTeX, see `latex_func`'s own doc comment for why a
/// space is required there and never optional).
fn func_subscript_pieces(args: &[Expr], order: &[u32]) -> Vec<String> {
    let mut pieces = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        let n = func_arg_subscript_name(arg, i);
        for _ in 0..order[i] {
            pieces.push(n.clone());
        }
    }
    pieces
}

fn func_subscript(args: &[Expr], order: &[u32]) -> String {
    let pieces = func_subscript_pieces(args, order);
    if pieces.is_empty() {
        String::new()
    } else {
        format!("_{}", pieces.join(""))
    }
}

// ---------------------------------------------------------------------
// LaTeX
// ---------------------------------------------------------------------

fn latex(e: &Expr, min: u8) -> String {
    let (level, body) = match e {
        Expr::Rational(s) => (ATOM, s.render(Target::Latex)),
        Expr::Var(name) => (ATOM, latex_var(name)),
        Expr::Pow(base, exp) => (POWER, format!("{}^{{{exp}}}", latex(base, ATOM))),
        Expr::Sin(x) => (ATOM, format!("\\sin\\left({}\\right)", latex(x, SUM))),
        Expr::Cos(x) => (ATOM, format!("\\cos\\left({}\\right)", latex(x, SUM))),
        Expr::Exp(x) => (ATOM, format!("\\exp\\left({}\\right)", latex(x, SUM))),
        Expr::Sinh(x) => (ATOM, format!("\\sinh\\left({}\\right)", latex(x, SUM))),
        Expr::Cosh(x) => (ATOM, format!("\\cosh\\left({}\\right)", latex(x, SUM))),
        Expr::Func { name, args, order } => (ATOM, latex_func(name, args, order)),
        Expr::Mul(factors) => (PRODUCT, latex_mul(factors)),
        Expr::Add(terms) => {
            let mut flat = Vec::new();
            flatten_add(terms, &mut flat);
            (SUM, join_signed(flat.iter().map(|t| latex(t, PRODUCT)).collect(), "0"))
        }
    };
    parenthesize_if(body, level, min)
}

fn latex_mul(factors: &[Expr]) -> String {
    let mut flat = Vec::new();
    flatten_mul(factors, &mut flat);
    let (sign_negative, coeff, num, den) =
        split_mul(&flat, latex, |b, e| format!("{b}^{{{e}}}"));
    if den.is_empty() {
        assemble_mul(sign_negative, coeff.map(|c| c.render(Target::Latex)), num, den, " ")
    } else {
        let num_str = {
            let mut parts = Vec::new();
            if let Some(c) = &coeff {
                if *c != BigScalar::one() || num.is_empty() {
                    parts.push(c.render(Target::Latex));
                }
            }
            parts.extend(num);
            if parts.is_empty() {
                "1".to_string()
            } else {
                parts.join(" ")
            }
        };
        let den_str = den.join(" ");
        let body = format!("\\frac{{{num_str}}}{{{den_str}}}");
        if sign_negative {
            format!("-{body}")
        } else {
            body
        }
    }
}

/// LaTeX form of an indeterminate function call: `f(r)`, `f'(r)`,
/// `f''(r)` for one argument (a literal `'` renders as an actual prime
/// mark in math mode, no macro needed); `h(t,r)`, `h_{t}(t,r)`,
/// `h_{t r}(t,r)` for more than one -- ALWAYS braced, and each
/// subscript piece joined with a plain space, never bare concatenation.
/// Both are load-bearing, not stylistic: bare concatenation of two
/// single-character subscript pieces (`h_tr`) only subscripts the
/// first, leaving the rest as ordinary (non-subscripted) text; and a
/// Greek piece directly followed by another piece with no separator
/// parses as one longer, undefined control word -- the exact `\thetar`
/// bug a user caught by eye in this project's tensor-index rendering
/// (`oderom_components::render::format_indices`), fixed there the same
/// way: a space, which math mode ignores for layout but which still
/// ends a control word in the right place.
fn latex_func(name: &str, args: &[Expr], order: &[u32]) -> String {
    let args_str = args.iter().map(|a| latex(a, SUM)).collect::<Vec<_>>().join(", ");
    let shown_name = latex_var(name);
    let marks = if args.len() == 1 {
        "'".repeat(order[0] as usize)
    } else {
        let pieces = func_subscript_pieces(args, order);
        if pieces.is_empty() {
            String::new()
        } else {
            let latex_pieces: Vec<String> = pieces.iter().map(|p| latex_var(p)).collect();
            format!("_{{{}}}", latex_pieces.join(" "))
        }
    };
    format!("{shown_name}{marks}({args_str})")
}

/// The Greek letters that show up constantly as coordinate/index names
/// in differential geometry (`theta`, `phi`, ...), lowercase. Shared with
/// `oderom-cli`'s LaTeX-flavored parser (`\theta` -> `Var("theta")`) so
/// the two directions -- render a name as a macro, read a macro back as
/// a name -- can never drift apart by listing the letters twice.
pub const GREEK_LETTERS: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
    "lambda", "mu", "nu", "xi", "omicron", "pi", "rho", "sigma", "tau", "upsilon", "phi", "chi",
    "psi", "omega",
];

/// LaTeX macros for the Greek letters (see [`GREEK_LETTERS`]); anything
/// else passes through unchanged. `pub` (not just crate-internal) so
/// `oderom-components::render` can turn a chart coordinate name (e.g.
/// `"theta"`) into the same `\theta` macro this module already uses for
/// expression variables, rather than a second copy of this mapping.
pub fn latex_var(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if GREEK_LETTERS.contains(&lower.as_str()) {
        let macro_name = if name.chars().next().is_some_and(char::is_uppercase) {
            let mut c = lower.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                None => lower,
            }
        } else {
            lower
        };
        format!("\\{macro_name}")
    } else {
        name.to_string()
    }
}

// ---------------------------------------------------------------------
// Mathematica (Rodada Exportação)
// ---------------------------------------------------------------------

/// `^` for power (always parenthesized when the exponent is negative --
/// `x^(-6)`, never a bare `x^-6`: valid either way in real Mathematica,
/// but parenthesized is unambiguous on inspection, and "funcione", not
/// "bonito", is the standard this whole target is held to), `Sin[x]`-
/// style capitalized-and-bracketed calls for the five built-in
/// functions, `a/b` for a rational literal (exact in Mathematica for any
/// two integers, see `BigScalar`/`Scalar`'s own Mathematica `Render`
/// arms), bare identifiers for variables (Greek names are NOT escaped to
/// any macro here -- `theta`, never `\theta`; Mathematica wants a plain
/// identifier, and the internal representation already stores exactly
/// that string, see `latex_var`'s own doc comment for why no unescaping
/// step is needed).
fn mathematica(e: &Expr, min: u8) -> String {
    let (level, body) = match e {
        Expr::Rational(s) => (ATOM, s.render(Target::Mathematica)),
        Expr::Var(name) => (ATOM, name.clone()),
        Expr::Pow(base, exp) => {
            let exp_str = if *exp < 0 { format!("({exp})") } else { format!("{exp}") };
            (POWER, format!("{}^{exp_str}", mathematica(base, ATOM)))
        }
        Expr::Sin(x) => (ATOM, format!("Sin[{}]", mathematica(x, SUM))),
        Expr::Cos(x) => (ATOM, format!("Cos[{}]", mathematica(x, SUM))),
        Expr::Exp(x) => (ATOM, format!("Exp[{}]", mathematica(x, SUM))),
        Expr::Sinh(x) => (ATOM, format!("Sinh[{}]", mathematica(x, SUM))),
        Expr::Cosh(x) => (ATOM, format!("Cosh[{}]", mathematica(x, SUM))),
        Expr::Func { name, args, order } => (ATOM, mathematica_func(name, args, order)),
        Expr::Mul(factors) => (PRODUCT, mathematica_mul(factors)),
        Expr::Add(terms) => {
            let mut flat = Vec::new();
            flatten_add(terms, &mut flat);
            (SUM, join_signed(flat.iter().map(|t| mathematica(t, PRODUCT)).collect(), "0"))
        }
    };
    parenthesize_if(body, level, min)
}

fn mathematica_mul(factors: &[Expr]) -> String {
    let mut flat = Vec::new();
    flatten_mul(factors, &mut flat);
    // `render_den_pow`'s own `e` is always positive by construction
    // (`split_mul` negates a negative exponent before calling it) --
    // parenthesizing only matters for a genuinely negative exponent
    // typed inline (the bare `Expr::Pow` match arm above), never here.
    let (sign_negative, coeff, num, den) = split_mul(&flat, mathematica, |b, e| format!("{b}^{e}"));
    assemble_mul(sign_negative, coeff.map(|c| c.render(Target::Mathematica)), num, den, "*")
}

/// `f[r]`, `f'[r]`, `f''[r]` for a single argument -- Mathematica's own
/// prime notation for `Derivative[n][f]`, real and valid for any `n`
/// (not just 1/2), and exactly the notation the export round asks for by
/// name. `h[t,r]`, `D[h[t,r],{t,2},{r,1}]` for more than one argument
/// (or a single argument differentiated to order 0, i.e. not
/// differentiated at all -- no `D[...]` wrapper, just a bare call): each
/// differentiated argument becomes one `{var,order}` pair, zero-order
/// arguments omitted entirely, in argument order -- the flat form
/// Mathematica's own `D` accepts directly.
fn mathematica_func(name: &str, args: &[Expr], order: &[u32]) -> String {
    let args_str = args.iter().map(|a| mathematica(a, SUM)).collect::<Vec<_>>().join(", ");
    if args.len() == 1 {
        let marks = "'".repeat(order[0] as usize);
        return format!("{name}{marks}[{args_str}]");
    }
    let call = format!("{name}[{args_str}]");
    let pairs: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|(i, _)| order[*i] > 0)
        .map(|(i, a)| format!("{{{}, {}}}", mathematica(a, SUM), order[i]))
        .collect();
    if pairs.is_empty() {
        call
    } else {
        format!("D[{call}, {}]", pairs.join(", "))
    }
}

// ---------------------------------------------------------------------
// SymPy (Rodada Exportação)
// ---------------------------------------------------------------------

/// `**` for power (SymPy/Python accept a bare negative-integer exponent,
/// `r**-6`, but this project parenthesizes it anyway -- `r**(-6)` --
/// same "unambiguous over pretty" reasoning as the Mathematica renderer
/// above, and consistent with it rather than an arbitrary difference
/// between the two), lowercase `sin(x)`-style calls for the five
/// built-ins (already exactly what Unicode produces -- `sin`/`cos`/
/// `exp`/`sinh`/`cosh` are the same spelling in both), always-explicit
/// `*` (SymPy never accepts bare juxtaposition -- `2*M`, never `2 M`),
/// bare identifiers for variables (no LaTeX escaping, same reasoning as
/// `mathematica`'s own doc comment), and rational literals via
/// `BigScalar`/`Scalar`'s own Sympy arm (`Rational(n,d)` for a genuine
/// fraction -- see that doc comment for why a bare `n/d` would silently
/// evaluate as an inexact Python float instead of an exact SymPy
/// rational).
fn sympy(e: &Expr, min: u8) -> String {
    let (level, body) = match e {
        Expr::Rational(s) => (ATOM, s.render(Target::Sympy)),
        Expr::Var(name) => (ATOM, name.clone()),
        Expr::Pow(base, exp) => {
            let exp_str = if *exp < 0 { format!("({exp})") } else { format!("{exp}") };
            (POWER, format!("{}**{exp_str}", sympy(base, ATOM)))
        }
        Expr::Sin(x) => (ATOM, format!("sin({})", sympy(x, SUM))),
        Expr::Cos(x) => (ATOM, format!("cos({})", sympy(x, SUM))),
        Expr::Exp(x) => (ATOM, format!("exp({})", sympy(x, SUM))),
        Expr::Sinh(x) => (ATOM, format!("sinh({})", sympy(x, SUM))),
        Expr::Cosh(x) => (ATOM, format!("cosh({})", sympy(x, SUM))),
        Expr::Func { name, args, order } => (ATOM, sympy_func(name, args, order)),
        Expr::Mul(factors) => (PRODUCT, sympy_mul(factors)),
        Expr::Add(terms) => {
            let mut flat = Vec::new();
            flatten_add(terms, &mut flat);
            (SUM, join_signed(flat.iter().map(|t| sympy(t, PRODUCT)).collect(), "0"))
        }
    };
    parenthesize_if(body, level, min)
}

fn sympy_mul(factors: &[Expr]) -> String {
    let mut flat = Vec::new();
    flatten_mul(factors, &mut flat);
    let (sign_negative, coeff, num, den) = split_mul(&flat, sympy, |b, e| format!("{b}**{e}"));
    assemble_mul(sign_negative, coeff.map(|c| c.render(Target::Sympy)), num, den, "*")
}

/// `Function('f')(r)` for an indeterminate function, never
/// differentiated -- deliberately NOT pre-bound to a Python variable
/// named `f` (e.g. no top-level `f = Function('f')` line anywhere this
/// crate emits): inlining the constructor at every use site is what
/// lets a chart coordinate's OWN name safely double as both a plain
/// `Symbol` (`r`, wherever it appears bare, e.g. inside a Christoffel
/// coefficient) and an indeterminate function of another coordinate
/// (`Function('r')(tau)`, `r`'s own trajectory along a geodesic) in the
/// very same equation -- both are real, simultaneous uses in this
/// project's own `geodesic`/`accel` output, and they are two genuinely
/// different SymPy objects that merely happen to share a display name;
/// binding either one to a bare Python variable called `r` would make
/// the second use silently shadow or collide with the first. Any
/// differentiated argument becomes `Derivative(f(...), var, order, ...)`
/// -- zero-order arguments omitted, SymPy's own flat multi-variable
/// `Derivative` form.
fn sympy_func(name: &str, args: &[Expr], order: &[u32]) -> String {
    let args_str = args.iter().map(|a| sympy(a, SUM)).collect::<Vec<_>>().join(", ");
    let call = format!("Function('{name}')({args_str})");
    let pieces: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|(i, _)| order[*i] > 0)
        .flat_map(|(i, a)| vec![sympy(a, SUM), order[i].to_string()])
        .collect();
    if pieces.is_empty() {
        call
    } else {
        format!("Derivative({call}, {})", pieces.join(", "))
    }
}

// ---------------------------------------------------------------------
// Shared `Mul` splitting: numerator/denominator factors plus a folded
// rational coefficient, target-agnostic (the caller renders each part).
// ---------------------------------------------------------------------

/// Splices nested `Mul` factors into one flat list -- `Expr`'s `*`
/// operator (used freely throughout this project, e.g. `a * b * c`)
/// nests as `Mul([Mul([a, b]), c])` rather than `Mul([a, b, c])`; only
/// `normalize` flattens it, and the renderer should not assume its
/// input has been normalized first.
fn flatten_mul<'a>(factors: &'a [Expr], out: &mut Vec<&'a Expr>) {
    for f in factors {
        match f {
            Expr::Mul(inner) => flatten_mul(inner, out),
            _ => out.push(f),
        }
    }
}

/// Same flattening, for nested `Add`.
fn flatten_add<'a>(terms: &'a [Expr], out: &mut Vec<&'a Expr>) {
    for t in terms {
        match t {
            Expr::Add(inner) => flatten_add(inner, out),
            _ => out.push(t),
        }
    }
}

#[allow(clippy::type_complexity)]
fn split_mul(
    factors: &[&Expr],
    render_factor: impl Fn(&Expr, u8) -> String,
    render_den_pow: impl Fn(&str, i32) -> String,
) -> (bool, Option<BigScalar>, Vec<String>, Vec<String>) {
    let mut sign_negative = false;
    let mut coeff: Option<BigScalar> = None;
    let mut num = Vec::new();
    let mut den = Vec::new();
    for &f in factors {
        if let Expr::Rational(s) = f {
            let mag = if s.is_negative() {
                sign_negative = !sign_negative;
                -s.clone()
            } else {
                s.clone()
            };
            coeff = Some(match coeff {
                Some(c) => c * mag,
                None => mag,
            });
            continue;
        }
        match f {
            Expr::Pow(base, exp) if *exp < 0 => {
                let base_str = render_factor(base, POWER);
                den.push(if *exp == -1 { base_str } else { render_den_pow(&base_str, -exp) });
            }
            _ => num.push(render_factor(f, POWER)),
        }
    }
    (sign_negative, coeff, num, den)
}

fn assemble_mul(
    sign_negative: bool,
    coeff: Option<String>,
    num: Vec<String>,
    den: Vec<String>,
    join: &str,
) -> String {
    let mut num_parts = Vec::new();
    if let Some(c) = coeff {
        if c != "1" || num.is_empty() {
            num_parts.push(c);
        }
    }
    num_parts.extend(num);
    let num_str = if num_parts.is_empty() { "1".to_string() } else { num_parts.join(join) };

    let body = if den.is_empty() {
        num_str
    } else if den.len() == 1 {
        format!("{num_str}/{}", den[0])
    } else {
        format!("{num_str}/({})", den.join(join))
    };
    if sign_negative {
        format!("-{body}")
    } else {
        body
    }
}

// ---------------------------------------------------------------------
// JSON: structural, tagged by variant, no precedence/parens needed.
// ---------------------------------------------------------------------

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json(e: &Expr) -> String {
    match e {
        Expr::Rational(s) => format!(r#"{{"type":"Rational","value":{}}}"#, s.render(Target::Json)),
        Expr::Var(name) => format!(r#"{{"type":"Var","name":{}}}"#, json_escape(name)),
        Expr::Add(terms) => {
            format!(r#"{{"type":"Add","terms":[{}]}}"#, terms.iter().map(json).collect::<Vec<_>>().join(","))
        }
        Expr::Mul(factors) => {
            format!(r#"{{"type":"Mul","factors":[{}]}}"#, factors.iter().map(json).collect::<Vec<_>>().join(","))
        }
        Expr::Pow(base, exp) => format!(r#"{{"type":"Pow","base":{},"exp":{exp}}}"#, json(base)),
        Expr::Sin(x) => format!(r#"{{"type":"Sin","arg":{}}}"#, json(x)),
        Expr::Cos(x) => format!(r#"{{"type":"Cos","arg":{}}}"#, json(x)),
        Expr::Exp(x) => format!(r#"{{"type":"Exp","arg":{}}}"#, json(x)),
        Expr::Sinh(x) => format!(r#"{{"type":"Sinh","arg":{}}}"#, json(x)),
        Expr::Cosh(x) => format!(r#"{{"type":"Cosh","arg":{}}}"#, json(x)),
        Expr::Func { name, args, order } => format!(
            r#"{{"type":"Func","name":{},"args":[{}],"order":[{}]}}"#,
            json_escape(name),
            args.iter().map(json).collect::<Vec<_>>().join(","),
            order.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize;

    /// Golden strings: these test the *renderer's output format*, never
    /// a mathematical claim -- correctness elsewhere in this project is
    /// always checked via `normalize`/structural `Expr` equality (see
    /// DESIGN-UI.md).
    #[test]
    fn unicode_infix_with_precedence() {
        let m = Expr::var("M");
        let r = Expr::var("r");
        let kretschmann = Expr::int(48) * m.clone().pow(2) * r.clone().pow(-6);
        assert_eq!(unicode(&kretschmann, SUM), "48*M^2/r^6");

        // The exact canonical shape here is normalize()'s call, not this
        // render test's: it always fully reduces to one num/den pair via
        // polynomial GCD now (rather than keeping an unreduced sum when
        // no cancellation is found the way the legacy engine did), so
        // `1 - 2M/r` becomes a single fraction with a parenthesized
        // numerator -- still exercises precedence (parens around a sum
        // numerator) just as well.
        let schwarzschild_gtt = Expr::int(1) - (Expr::int(2) * m) / r;
        assert_eq!(unicode(&normalize(&schwarzschild_gtt), SUM), "(-2*M + r)/r");

        let squared_sum = (Expr::var("a") + Expr::var("b")).pow(2);
        assert_eq!(unicode(&squared_sum, SUM), "(a + b)^2");
    }

    #[test]
    fn latex_uses_frac_and_greek_macros() {
        let m = Expr::var("M");
        let r = Expr::var("r");
        let kretschmann = Expr::int(48) * m * r.pow(-6);
        assert_eq!(latex(&kretschmann, SUM), "\\frac{48 M}{r^{6}}");

        let theta = Expr::var("theta");
        assert_eq!(latex(&theta.sin(), SUM), "\\sin\\left(\\theta\\right)");
    }

    #[test]
    fn json_is_a_tagged_tree() {
        let e = Expr::var("x").pow(2) + Expr::int(1);
        assert_eq!(
            json(&e),
            r#"{"type":"Add","terms":[{"type":"Pow","base":{"type":"Var","name":"x"},"exp":2},{"type":"Rational","value":{"num":1,"den":1}}]}"#
        );
    }

    #[test]
    fn display_matches_unicode_target() {
        let e = Expr::var("x") + Expr::int(1);
        assert_eq!(e.to_string(), e.render(Target::Unicode));
    }

    // -------------------------------------------------------------
    // Mathematica/SymPy (Rodada Exportação) -- golden strings, same
    // discipline as every other target's own tests above: these check
    // the renderer's output FORMAT, never a mathematical claim.
    // -------------------------------------------------------------

    #[test]
    fn mathematica_power_and_builtin_functions() {
        let m = Expr::var("M");
        let r = Expr::var("r");
        let kretschmann = Expr::int(48) * m.clone().pow(2) * r.clone().pow(-6);
        assert_eq!(mathematica(&kretschmann, SUM), "48*M^2/r^6");

        let theta = Expr::var("theta");
        assert_eq!(mathematica(&theta.clone().sin(), SUM), "Sin[theta]");
        assert_eq!(mathematica(&theta.clone().cos(), SUM), "Cos[theta]");
        assert_eq!(mathematica(&theta.exp(), SUM), "Exp[theta]");

        // A negative exponent stays parenthesized, never a bare `x^-6`.
        assert_eq!(mathematica(&r.pow(-6), SUM), "r^(-6)");
    }

    #[test]
    fn mathematica_indeterminate_function_and_derivative() {
        let r = Expr::var("r");
        assert_eq!(mathematica_func("f", &[r.clone()], &[0]), "f[r]");
        assert_eq!(mathematica_func("f", &[r.clone()], &[1]), "f'[r]");
        assert_eq!(mathematica_func("f", &[r.clone()], &[2]), "f''[r]");

        let t = Expr::var("t");
        // Multi-argument, mixed partial: {var,order} pairs, zero-order
        // arguments omitted.
        assert_eq!(mathematica_func("h", &[t.clone(), r.clone()], &[2, 1]), "D[h[t, r], {t, 2}, {r, 1}]");
        assert_eq!(mathematica_func("h", &[t, r], &[0, 0]), "h[t, r]");
    }

    #[test]
    fn mathematica_rational_literal_is_exact_division() {
        let e = Expr::rational(3, 4) * Expr::var("M");
        assert_eq!(mathematica(&e, SUM), "3/4*M");
    }

    #[test]
    fn sympy_power_and_builtin_functions_and_explicit_star() {
        let m = Expr::var("M");
        let r = Expr::var("r");
        let kretschmann = Expr::int(48) * m.clone().pow(2) * r.clone().pow(-6);
        assert_eq!(sympy(&kretschmann, SUM), "48*M**2/r**6");

        let theta = Expr::var("theta");
        assert_eq!(sympy(&theta.sin(), SUM), "sin(theta)");

        assert_eq!(sympy(&r.pow(-6), SUM), "r**(-6)");

        // A coefficient next to a variable must never be bare
        // juxtaposition -- SymPy has no such thing.
        let two_m = Expr::int(2) * m;
        assert_eq!(sympy(&two_m, SUM), "2*M");
    }

    #[test]
    fn sympy_indeterminate_function_and_derivative() {
        let r = Expr::var("r");
        assert_eq!(sympy_func("f", &[r.clone()], &[0]), "Function('f')(r)");
        assert_eq!(sympy_func("f", &[r.clone()], &[1]), "Derivative(Function('f')(r), r, 1)");

        let t = Expr::var("t");
        assert_eq!(sympy_func("h", &[t.clone(), r.clone()], &[2, 1]), "Derivative(Function('h')(t, r), t, 2, r, 1)");
        assert_eq!(sympy_func("h", &[t, r], &[0, 0]), "Function('h')(t, r)");
    }

    #[test]
    fn sympy_fraction_literal_is_wrapped_never_bare_python_division() {
        // The trap this renderer specifically exists to avoid: `3/4` as
        // bare Python source would evaluate to the float 0.75 before
        // SymPy ever saw it -- must come out as an explicit `Rational`.
        let e = Expr::rational(3, 4) * Expr::var("M");
        assert_eq!(sympy(&e, SUM), "Rational(3, 4)*M");

        let negative = Expr::rational(-3, 4);
        assert_eq!(sympy(&negative, SUM), "Rational(-3, 4)");

        // An integer coefficient is a plain, safe Python literal either way.
        let integer_coeff = Expr::int(48) * Expr::var("M");
        assert_eq!(sympy(&integer_coeff, SUM), "48*M");
    }

    #[test]
    fn mathematica_and_sympy_never_escape_greek_names_to_latex_macros() {
        // Unlike `latex_var`, neither new target should ever produce a
        // `\theta`-style macro -- both want the plain identifier.
        let theta = Expr::var("theta");
        assert_eq!(mathematica(&theta, SUM), "theta");
        assert_eq!(sympy(&theta, SUM), "theta");
    }
}
