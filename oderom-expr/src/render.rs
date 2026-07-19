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

use crate::Expr;
use oderom_core::{Render, Scalar, Target};
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
                if *c != Scalar::ONE || num.is_empty() {
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

/// LaTeX macros for the Greek letters that show up constantly as
/// coordinate/index names in differential geometry (`theta`, `phi`,
/// ...); anything else passes through unchanged.
fn latex_var(name: &str) -> String {
    const GREEK: &[&str] = &[
        "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
        "lambda", "mu", "nu", "xi", "omicron", "pi", "rho", "sigma", "tau", "upsilon", "phi",
        "chi", "psi", "omega",
    ];
    let lower = name.to_ascii_lowercase();
    if GREEK.contains(&lower.as_str()) {
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
) -> (bool, Option<Scalar>, Vec<String>, Vec<String>) {
    let mut sign_negative = false;
    let mut coeff: Option<Scalar> = None;
    let mut num = Vec::new();
    let mut den = Vec::new();
    for &f in factors {
        if let Expr::Rational(s) = f {
            let mag = if s.numerator() < 0 {
                sign_negative = !sign_negative;
                Scalar::new(-s.numerator(), s.denominator())
            } else {
                *s
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

        let schwarzschild_gtt = Expr::int(1) - (Expr::int(2) * m) / r;
        assert_eq!(unicode(&normalize(&schwarzschild_gtt), SUM), "1 - 2*M/r");

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
}
