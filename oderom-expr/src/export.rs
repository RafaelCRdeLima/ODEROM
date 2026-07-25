//! Support for `oderom-cli`'s `export` command (Rodada Exportação):
//! collecting every name (`Expr::Var`, `Expr::Func::name`) an expression
//! or a batch of them use, and renaming any that collide with a target
//! language's own reserved words -- Mathematica's `Protected` `System`
//! symbols (`E`, `Pi`, `I`, `D`, `N`, ...) or Python/SymPy's own keyword
//! and special-constant set. Pure tree substitution, never a
//! simplification: a name colliding with a reserved word gets renamed
//! and the rename is reported (the caller emits it as a visible comment
//! in the exported text -- see `oderom-cli::commands::export_cmd`), it
//! is never silently dropped or reinterpreted.

use crate::Expr;
use oderom_core::Target;
use std::collections::{BTreeMap, BTreeSet};

/// Mathematica `System`-context symbols with the `Protected` attribute
/// that a chart coordinate or free parameter name could plausibly
/// collide with in a real GR fixture -- assigning to any of these in
/// real Mathematica raises `Set::wrsym`. `E`/`Pi`/`I`/`D`/`N` are named
/// explicitly in this round's own spec; `Gamma` is added because this
/// project's own `christoffel` query always labels its result `Gamma`
/// -- a real, concrete, always-relevant collision for this domain
/// specifically, not a hypothetical one. `C`/`K`/`O` round out the
/// small set of single-letter `Protected` symbols a physics fixture
/// could plausibly reach for (integration constants, a coupling
/// constant, big-O).
pub const MATHEMATICA_RESERVED: &[&str] = &["E", "Pi", "I", "D", "N", "Gamma", "C", "K", "O"];

/// Python keywords (a real collision risk in this exact domain: the
/// cosmological constant is very often spelled `lambda`, a hard Python
/// keyword) plus SymPy's own special constants/classes that a plain
/// chart coordinate or parameter name could collide with.
pub const SYMPY_RESERVED: &[&str] = &[
    "lambda", "class", "def", "import", "from", "return", "if", "else", "elif", "for", "while", "in", "is", "not", "and", "or",
    "None", "True", "False", "global", "pass", "with", "as", "yield", "try", "except", "finally", "raise", "assert", "del",
    "I", "E", "pi", "oo", "zoo", "nan", "S", "N",
];

/// Every distinct name `expr` uses, split into plain variables
/// (`Expr::Var`) and indeterminate function names (`Expr::Func::name`)
/// -- kept separate because the two need different treatment on export
/// (a variable becomes a `symbols(...)` entry for SymPy; a function
/// name never does, see `oderom-expr::render::sympy_func`'s own doc
/// comment for why it is always inlined instead). Unlike
/// [`crate::free_vars`] (which deliberately never collects a `Func`'s
/// own name -- see that function's doc comment), this collects both:
/// export needs to detect and rename BOTH kinds of reserved-word
/// collision, not just plain variables.
pub fn collect_names(expr: &Expr, vars: &mut BTreeSet<String>, funcs: &mut BTreeSet<String>) {
    match expr {
        Expr::Rational(_) => {}
        Expr::Var(name) => {
            vars.insert(name.clone());
        }
        Expr::Add(terms) | Expr::Mul(terms) => terms.iter().for_each(|t| collect_names(t, vars, funcs)),
        Expr::Pow(base, _) => collect_names(base, vars, funcs),
        Expr::Sin(x) | Expr::Cos(x) | Expr::Exp(x) | Expr::Sinh(x) | Expr::Cosh(x) => collect_names(x, vars, funcs),
        Expr::Func { name, args, .. } => {
            funcs.insert(name.clone());
            args.iter().for_each(|a| collect_names(a, vars, funcs));
        }
    }
}

/// Builds a rename map for every name in `names` that collides with
/// `reserved`, by appending `suffix` repeatedly until the candidate is
/// neither reserved nor already in use by another name already present
/// in `names` (defensive: avoids a renamed name accidentally colliding
/// with an unrelated, genuinely-distinct name the expression already
/// uses). Conservative -- never renames a name that isn't actually
/// reserved -- and deterministic (`BTreeSet`/`BTreeMap` throughout, so
/// the same input always produces the same renames, never dependent on
/// hash-map iteration order, which matters here because the SAME map
/// must be reused consistently across every component of a multi-part
/// export, see `oderom-cli::commands::export_cmd`).
pub fn build_rename_map(names: &BTreeSet<String>, reserved: &[&str], suffix: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for name in names {
        if !reserved.contains(&name.as_str()) {
            continue;
        }
        let mut candidate = name.clone();
        while reserved.contains(&candidate.as_str()) || names.contains(&candidate) {
            candidate.push_str(suffix);
        }
        map.insert(name.clone(), candidate);
    }
    map
}

/// Applies `map` to every `Expr::Var`/`Expr::Func::name` in `expr`,
/// leaving anything not in `map` untouched. Pure renaming -- no
/// arithmetic, no simplification; the tree's shape and every numeric
/// value it carries are otherwise identical to `expr`.
pub fn apply_renames(expr: &Expr, map: &BTreeMap<String, String>) -> Expr {
    let rename = |n: &str| map.get(n).cloned().unwrap_or_else(|| n.to_string());
    match expr {
        Expr::Rational(_) => expr.clone(),
        Expr::Var(name) => Expr::Var(rename(name)),
        Expr::Add(terms) => Expr::Add(terms.iter().map(|t| apply_renames(t, map)).collect()),
        Expr::Mul(factors) => Expr::Mul(factors.iter().map(|f| apply_renames(f, map)).collect()),
        Expr::Pow(base, exp) => Expr::Pow(Box::new(apply_renames(base, map)), *exp),
        Expr::Sin(x) => Expr::Sin(Box::new(apply_renames(x, map))),
        Expr::Cos(x) => Expr::Cos(Box::new(apply_renames(x, map))),
        Expr::Exp(x) => Expr::Exp(Box::new(apply_renames(x, map))),
        Expr::Sinh(x) => Expr::Sinh(Box::new(apply_renames(x, map))),
        Expr::Cosh(x) => Expr::Cosh(Box::new(apply_renames(x, map))),
        Expr::Func { name, args, order } => {
            Expr::Func { name: rename(name), args: args.iter().map(|a| apply_renames(a, map)).collect(), order: order.clone() }
        }
    }
}

/// Wraps `text` (an orbit-size annotation, a zero/truncation count, a
/// reserved-word rename note -- anything ordinarily shown as a bare
/// plain-English line beside a formula) as a real comment in `target`'s
/// own syntax. Needed because export's flat text output mixes formula
/// lines with exactly these kinds of annotations in the same block
/// (`RenderedClasses::to_text()`'s own join) -- for `Target::Unicode`/
/// `Target::Latex` a bare prose line is harmless (nobody re-executes
/// that text), but for `Target::Mathematica`/`Target::Sympy` a bare,
/// non-code line breaks `exec`/evaluation the moment it's pasted,
/// exactly the "cole na ferramenta e funcione" bar export exists to
/// clear. Every other `Target` returns `text` unchanged: nothing about
/// this function's callers needs a comment marker there.
pub fn comment_line(text: &str, target: Target) -> String {
    match target {
        Target::Mathematica => format!("(* {text} *)"),
        Target::Sympy => format!("# {text}"),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Expr;

    #[test]
    fn collect_names_separates_variables_from_function_names() {
        let e = Expr::var("M") * Expr::func("f", vec![Expr::var("r")]);
        let mut vars = BTreeSet::new();
        let mut funcs = BTreeSet::new();
        collect_names(&e, &mut vars, &mut funcs);
        assert_eq!(vars, BTreeSet::from(["M".to_string(), "r".to_string()]));
        assert_eq!(funcs, BTreeSet::from(["f".to_string()]));
    }

    #[test]
    fn build_rename_map_only_touches_reserved_names() {
        let names = BTreeSet::from(["M".to_string(), "E".to_string(), "r".to_string()]);
        let map = build_rename_map(&names, MATHEMATICA_RESERVED, "$");
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("E"), Some(&"E$".to_string()));
        assert!(!map.contains_key("M"));
        assert!(!map.contains_key("r"));
    }

    #[test]
    fn build_rename_map_avoids_colliding_with_an_existing_unrelated_name() {
        // A pathological but real-to-guard-against case: both "E" and
        // "E$" already appear as genuinely distinct names -- the rename
        // must skip past "E$" to "E$$" rather than merge the two.
        let names = BTreeSet::from(["E".to_string(), "E$".to_string()]);
        let map = build_rename_map(&names, MATHEMATICA_RESERVED, "$");
        assert_eq!(map.get("E"), Some(&"E$$".to_string()));
    }

    #[test]
    fn apply_renames_rewrites_vars_and_func_names_but_nothing_else() {
        let e = Expr::var("E") * Expr::func("D", vec![Expr::var("r")]);
        let map = BTreeMap::from([("E".to_string(), "E$".to_string()), ("D".to_string(), "D$".to_string())]);
        let renamed = apply_renames(&e, &map);
        let mut vars = BTreeSet::new();
        let mut funcs = BTreeSet::new();
        collect_names(&renamed, &mut vars, &mut funcs);
        assert_eq!(vars, BTreeSet::from(["E$".to_string(), "r".to_string()]));
        assert_eq!(funcs, BTreeSet::from(["D$".to_string()]));
    }

    #[test]
    fn apply_renames_leaves_a_non_colliding_expression_byte_for_byte_equal() {
        let e = Expr::var("M") * Expr::var("r").pow(-1);
        let map = BTreeMap::new();
        assert_eq!(apply_renames(&e, &map), e);
    }

    #[test]
    fn comment_line_uses_each_symbolic_targets_own_comment_syntax() {
        assert_eq!(comment_line("4 components by symmetry", Target::Sympy), "# 4 components by symmetry");
        assert_eq!(comment_line("4 components by symmetry", Target::Mathematica), "(* 4 components by symmetry *)");
    }

    #[test]
    fn comment_line_leaves_non_symbolic_targets_unchanged() {
        assert_eq!(comment_line("4 components by symmetry", Target::Unicode), "4 components by symmetry");
        assert_eq!(comment_line("4 components by symmetry", Target::Latex), "4 components by symmetry");
        assert_eq!(comment_line("4 components by symmetry", Target::Json), "4 components by symmetry");
    }
}
