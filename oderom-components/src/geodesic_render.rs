//! Rendering the geodesic equation (Marco 6 step 4, round B) --
//! `curvature::geodesic_equations`'s own output, one already-normalized
//! `Expr` per coordinate, into `"<lhs> = 0"` text. Reuses
//! `RenderedComponent`/`RenderedClasses` (`render.rs`) purely for their
//! generic shape (a list of formula+annotation pairs, plus a caller-facing
//! flat-text join) -- there is no tensor symmetry here at all (no orbit
//! grouping, no zero-suppression: all `chart.dim()` equations are always
//! shown), so this never touches `classify_tensor`/`classify_grid`.
//!
//! # The central design point: two encarnações of one coordinate name
//!
//! A chart coordinate (`t`, `r`, ...) means two different things in one
//! geodesic query's own output: `christoffel`'s own Gamma coefficients
//! are written in terms of the free coordinate (`Expr::Var("r")`,
//! exactly as `render_classes` already renders it, untouched here), and
//! the SAME name also appears as a function of the affine parameter
//! along the curve (`Expr::Func { name: "r", args: [Var(param)], order }`,
//! `curvature::geodesic_equations`'s own construction -- the identical
//! indeterminate-function machinery `f(r)`/`f'(r)` already use, round
//! A, applied here to a coordinate's own name). The two are never
//! confused at the REPRESENTATION level -- they are different `Expr`
//! shapes built by different code, never the same node reinterpreted.
//! What this module adds is choosing, per render target, how the
//! SECOND encarnação is typeset: LaTeX gets dot notation (`\dot{r}`,
//! `\ddot{r}`, the parameter left implicit -- "the form of a book"),
//! Unicode gets the explicit form (`r'(tau)`, `r''(tau)`) -- because a
//! dot glyph stacked over a letter does not survive plain text. Both
//! describe the identical equation; only the typography differs,
//! exactly the same split `theta`/`\theta` already has for an ordinary
//! coordinate name.
//!
//! Unicode needs no special code here at all: `Expr::Func`'s own
//! generic renderer (round A, `oderom_expr::render`) already produces
//! `r'(tau)`/`r''(tau)` for a single-argument function -- exactly the
//! form wanted -- so this module calls it completely unchanged. LaTeX
//! does need one: the generic renderer's own single-argument `Func`
//! form is prime notation too (`r'(tau)`, correct and wanted for a
//! *general* indeterminate function like `f(r)`, but not what a reader
//! expects for a coordinate along a worldline). Rather than duplicating
//! the whole precedence-aware LaTeX renderer just to intercept these
//! specific nodes, [`dot_notation_latex`] substitutes each node that is
//! EXACTLY "the affine parameter's own coordinate-derivative" (single
//! argument, and that argument is literally `Var(param)` -- checked,
//! never assumed, so an ordinary indeterminate function elsewhere in
//! the same equation, e.g. `f'(r)` from a metric that used `f(r)`,
//! is never mistaken for one just because it also happens to have one
//! argument at order 1 or 2) with a private-use-area-delimited
//! placeholder `Expr::Var` -- guaranteed never to collide with anything
//! a real `.od` file could ever lex, and rendered by the ordinary
//! machinery as an ordinary atom, the same precedence class a `Func`
//! call already has -- calls the SAME public `Expr::render(Target::Latex)`
//! unchanged, then substitutes the placeholder text back out for the
//! real dot notation. The algebra itself never sees any of this -- it
//! is a pure post-`normalize()` formatting step.

use crate::chart::Chart;
use crate::render::{RenderedClasses, RenderedComponent};
use oderom_core::{Render, Target};
use oderom_expr::{latex_var, Expr};
use std::collections::HashMap;

/// One geodesic equation's LHS, in `target`, as `"<lhs> = 0"` for every
/// target except the two symbolic export ones (Rodada Exportação),
/// where literal `=` is not book notation -- it's the target
/// language's own ASSIGNMENT operator, and `lhs` here is a whole
/// expression (a sum of acceleration/velocity/Christoffel terms), never
/// a bare identifier, so `<expr> = 0` is a `SyntaxError` in Python
/// ("cannot assign to expression here") and not meaningful in
/// Mathematica either. `Target::Sympy` uses `Eq(lhs, 0)` (a real,
/// evaluable SymPy object, e.g. for `solve`/`dsolve`); `Target::Mathematica`
/// uses `lhs == 0`, its own relational-equation operator. Caught by
/// actually running an export's geodesic output in real SymPy, not
/// merely inspecting the string -- see `oderom-cli/tests/export.rs`.
fn render_geodesic_component(lhs: &Expr, param: &str, target: Target) -> String {
    match target {
        Target::Latex => format!("{} = 0", dot_notation_latex(lhs, param)),
        Target::Sympy => format!("Eq({}, 0)", lhs.render(Target::Sympy)),
        Target::Mathematica => format!("{} == 0", lhs.render(Target::Mathematica)),
        _ => format!("{} = 0", lhs.render(target)),
    }
}

/// The flat, single-string form (CLI/REPL) -- see `render_classes`'s own
/// doc comment for why this and the structured form below share one
/// `to_text()` implementation rather than two independent joins that
/// could drift apart.
pub fn render_geodesic(label: &str, equations: &[Expr], param: &str, target: Target, max_lines: usize) -> String {
    match target {
        Target::Json => render_geodesic_json(label, param, equations),
        _ => render_geodesic_structured(label, equations, param, target, max_lines).to_text(),
    }
}

/// The structured form (`oderom-session`'s `EntryResult`, and so the
/// notebook/app's click-to-copy + per-equation KaTeX rendering) --
/// same split `render_classes_structured` already established for
/// tensor components, reused here for a list with no symmetry to elide.
pub fn render_geodesic_structured(label: &str, equations: &[Expr], param: &str, target: Target, max_lines: usize) -> RenderedClasses {
    let shown_count = equations.len().min(max_lines);
    let (shown, truncated) = (&equations[..shown_count], &equations[shown_count..]);
    let components =
        shown.iter().map(|eq| RenderedComponent { formula: render_geodesic_component(eq, param, target), orbit_note: None }).collect();
    let mut summary = Vec::new();
    if !truncated.is_empty() {
        summary.push(format!("... and {} more equation{} (truncated)", truncated.len(), if truncated.len() == 1 { "" } else { "s" }));
    }
    RenderedClasses { label: label.to_string(), components, summary }
}

fn render_geodesic_json(label: &str, param: &str, equations: &[Expr]) -> String {
    let items = equations.iter().map(|eq| format!(r#"{{"lhs":{}}}"#, eq.render(Target::Json))).collect::<Vec<_>>().join(",");
    format!(r#"{{"label":{},"param":{},"equations":[{items}]}}"#, json_escape(label), json_escape(param))
}

fn json_escape(s: &str) -> String {
    format!("{:?}", s)
}

/// One `accel` equation, `"<lhs> = <rhs>"` -- `lhs` is always literally
/// the acceleration `Func` node for `coord` (never present in `rhs`
/// itself, since `oderom-components::curvature::accel_equations`
/// already isolated it out), so both sides go through the exact same
/// dot-notation/generic-render treatment `render_geodesic_component`
/// already uses for a whole canonical equation -- this module's own
/// "which encarnação is on screen" design point applies identically
/// here, just split across an `=` instead of embedded in one sum.
///
/// Same `Eq(...)`/`==` substitution as `render_geodesic_component` for
/// the two symbolic export targets, and for the identical reason: `lhs`
/// here is `Derivative(Function('t')(tau), tau, 2)` (SymPy) or
/// `t''[tau]` (Mathematica), never a bare identifier, so literal `=`
/// would try to ASSIGN to that expression -- a `SyntaxError` in Python.
fn render_geodesic_solved_component(coord: &str, rhs: &Expr, param: &str, target: Target) -> String {
    let lhs = Expr::Func { name: coord.to_string(), args: vec![Expr::var(param)], order: vec![2] };
    match target {
        Target::Latex => format!("{} = {}", dot_notation_latex(&lhs, param), dot_notation_latex(rhs, param)),
        Target::Sympy => format!("Eq({}, {})", lhs.render(Target::Sympy), rhs.render(Target::Sympy)),
        Target::Mathematica => format!("{} == {}", lhs.render(Target::Mathematica), rhs.render(Target::Mathematica)),
        _ => format!("{} = {}", lhs.render(target), rhs.render(target)),
    }
}

/// The flat, single-string form of `accel`'s output (Marco 6 step 4,
/// round C) -- `rhs` is `curvature::accel_equations`' own output, one
/// already-solved-and-normalized right-hand side per coordinate, in the
/// same order as `chart.coords`; `chart` is only needed here (unlike
/// `render_geodesic` above) to name each equation's own left-hand side,
/// since a solved right-hand side no longer contains that coordinate's
/// own acceleration node to read the name back off of.
pub fn render_geodesic_solved(label: &str, chart: &Chart, rhs: &[Expr], param: &str, target: Target, max_lines: usize) -> String {
    match target {
        Target::Json => render_geodesic_solved_json(label, param, chart, rhs),
        _ => render_geodesic_solved_structured(label, chart, rhs, param, target, max_lines).to_text(),
    }
}

/// The structured form -- same split `render_geodesic_structured`
/// already established, reused here for the solved form's own list.
pub fn render_geodesic_solved_structured(label: &str, chart: &Chart, rhs: &[Expr], param: &str, target: Target, max_lines: usize) -> RenderedClasses {
    let shown_count = rhs.len().min(max_lines);
    let (shown, truncated) = (&rhs[..shown_count], &rhs[shown_count..]);
    let components = shown
        .iter()
        .enumerate()
        .map(|(a, e)| RenderedComponent { formula: render_geodesic_solved_component(chart.coord(a as u8), e, param, target), orbit_note: None })
        .collect();
    let mut summary = Vec::new();
    if !truncated.is_empty() {
        summary.push(format!("... and {} more equation{} (truncated)", truncated.len(), if truncated.len() == 1 { "" } else { "s" }));
    }
    RenderedClasses { label: label.to_string(), components, summary }
}

fn render_geodesic_solved_json(label: &str, param: &str, chart: &Chart, rhs: &[Expr]) -> String {
    let items = rhs
        .iter()
        .enumerate()
        .map(|(a, e)| format!(r#"{{"coord":{},"rhs":{}}}"#, json_escape(chart.coord(a as u8)), e.render(Target::Json)))
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"label":{},"param":{},"equations":[{items}]}}"#, json_escape(label), json_escape(param))
}

/// LaTeX dot notation for the parameter-derivative `Func` nodes in
/// `lhs`, everything else rendered exactly as `Expr::render(Target::Latex)`
/// already would -- see this module's own header doc for why a
/// substitute-then-render-then-substitute-back pass, not a parallel
/// renderer.
fn dot_notation_latex(lhs: &Expr, param: &str) -> String {
    let mut placeholder_of: HashMap<(String, u32), String> = HashMap::new();
    let substituted = substitute_dotted(lhs, param, &mut placeholder_of);
    let mut rendered = substituted.render(Target::Latex);
    for ((name, order), placeholder) in &placeholder_of {
        let command = match order {
            1 => "dot",
            2 => "ddot",
            _ => unreachable!("substitute_dotted only ever creates a placeholder for order 1 or 2"),
        };
        let final_latex = format!("\\{command}{{{}}}", latex_var(name));
        rendered = rendered.replace(placeholder.as_str(), &final_latex);
    }
    rendered
}

/// Walks `e`, replacing every `Func` node that is EXACTLY "a coordinate
/// differentiated once or twice with respect to `param`" -- one
/// argument, and that argument is literally `Var(param)`, checked, not
/// inferred from shape alone -- with a private-use-area placeholder
/// `Var`. Everything else (including a `Func` whose argument is NOT
/// `param`, e.g. an ordinary `f(r)` surviving in a Christoffel
/// coefficient from an indeterminate-function metric) is left exactly
/// as-is, recursing into its own arguments in case one of THOSE somehow
/// nested a parameter-derivative (never happens from
/// `geodesic_equations`'s own construction today, but this walks
/// generically rather than assuming a fixed shallow shape).
fn substitute_dotted(e: &Expr, param: &str, placeholder_of: &mut HashMap<(String, u32), String>) -> Expr {
    match e {
        Expr::Rational(_) | Expr::Var(_) => e.clone(),
        Expr::Add(terms) => Expr::Add(terms.iter().map(|t| substitute_dotted(t, param, placeholder_of)).collect()),
        Expr::Mul(factors) => Expr::Mul(factors.iter().map(|f| substitute_dotted(f, param, placeholder_of)).collect()),
        Expr::Pow(base, n) => Expr::Pow(Box::new(substitute_dotted(base, param, placeholder_of)), *n),
        Expr::Sin(x) => Expr::Sin(Box::new(substitute_dotted(x, param, placeholder_of))),
        Expr::Cos(x) => Expr::Cos(Box::new(substitute_dotted(x, param, placeholder_of))),
        Expr::Exp(x) => Expr::Exp(Box::new(substitute_dotted(x, param, placeholder_of))),
        Expr::Sinh(x) => Expr::Sinh(Box::new(substitute_dotted(x, param, placeholder_of))),
        Expr::Cosh(x) => Expr::Cosh(Box::new(substitute_dotted(x, param, placeholder_of))),
        Expr::Func { name, args, order } => {
            let is_param_derivative =
                args.len() == 1 && matches!(&args[0], Expr::Var(v) if v == param) && (order[0] == 1 || order[0] == 2);
            if is_param_derivative {
                let key = (name.clone(), order[0]);
                let next_id = placeholder_of.len();
                let placeholder = placeholder_of.entry(key).or_insert_with(|| format!("\u{E000}GEODOT{next_id}\u{E000}"));
                Expr::var(placeholder.clone())
            } else {
                Expr::Func {
                    name: name.clone(),
                    args: args.iter().map(|a| substitute_dotted(a, param, placeholder_of)).collect(),
                    order: order.clone(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oderom_core::Target;

    #[test]
    fn unicode_needs_no_special_casing_and_matches_the_generic_func_renderer() {
        let acc = Expr::Func { name: "t".to_string(), args: vec![Expr::var("tau")], order: vec![2] };
        let vel = Expr::Func { name: "r".to_string(), args: vec![Expr::var("tau")], order: vec![1] };
        let lhs = acc + vel;
        let text = render_geodesic_component(&lhs, "tau", Target::Unicode);
        assert!(text.contains("t''(tau)"), "{text}");
        assert!(text.contains("r'(tau)"), "{text}");
        assert!(text.ends_with(" = 0"), "{text}");
    }

    #[test]
    fn latex_uses_dot_notation_for_velocity_and_acceleration() {
        let acc = Expr::Func { name: "t".to_string(), args: vec![Expr::var("tau")], order: vec![2] };
        let vel = Expr::Func { name: "r".to_string(), args: vec![Expr::var("tau")], order: vec![1] };
        let lhs = acc + vel;
        let text = render_geodesic_component(&lhs, "tau", Target::Latex);
        assert!(text.contains("\\ddot{t}"), "{text}");
        assert!(text.contains("\\dot{r}"), "{text}");
        assert!(!text.contains("tau"), "the parameter must be implicit in the dot form: {text}");
        assert!(!text.contains('\''), "no leftover prime marks: {text}");
    }

    #[test]
    fn latex_dot_notation_uses_the_greek_macro_for_a_greek_coordinate_name() {
        let vel = Expr::Func { name: "theta".to_string(), args: vec![Expr::var("tau")], order: vec![1] };
        let text = render_geodesic_component(&vel, "tau", Target::Latex);
        assert!(text.contains("\\dot{\\theta}"), "{text}");
    }

    #[test]
    fn an_ordinary_indeterminate_function_of_a_chart_coordinate_is_never_mistaken_for_a_parameter_derivative() {
        // f'(r) -- single argument, order 1, EXACTLY the shape a
        // parameter-derivative has -- but its argument is the chart
        // coordinate `r`, never the affine parameter `tau`. Must stay
        // ordinary prime notation in LaTeX too, never `\dot{f}`.
        let f_prime_of_r = Expr::Func { name: "f".to_string(), args: vec![Expr::var("r")], order: vec![1] };
        let text = render_geodesic_component(&f_prime_of_r, "tau", Target::Latex);
        assert!(text.contains("f'(r)"), "{text}");
        assert!(!text.contains("\\dot"), "{text}");
    }

    #[test]
    fn repeated_occurrences_of_the_same_velocity_share_one_placeholder() {
        // r'(tau) * r'(tau), i.e. r'(tau)^2 written as a product of two
        // occurrences -- must render as \dot{r}\dot{r}, not leak any
        // placeholder text if the two occurrences aren't deduplicated
        // correctly.
        let vel = Expr::Func { name: "r".to_string(), args: vec![Expr::var("tau")], order: vec![1] };
        let lhs = vel.clone() * vel;
        let text = render_geodesic_component(&lhs, "tau", Target::Latex);
        assert_eq!(text.matches("\\dot{r}").count(), 2, "{text}");
        assert!(!text.contains('\u{E000}'), "no placeholder text must leak into the final output: {text}");
    }

    #[test]
    fn solved_component_unicode_shows_the_acceleration_alone_on_the_left() {
        let vel = Expr::Func { name: "r".to_string(), args: vec![Expr::var("tau")], order: vec![1] };
        let rhs = Expr::var("M") * vel;
        let text = render_geodesic_solved_component("t", &rhs, "tau", Target::Unicode);
        assert!(text.starts_with("t''(tau) = "), "{text}");
        assert!(text.contains("r'(tau)"), "{text}");
    }

    #[test]
    fn solved_component_latex_uses_dot_notation_on_both_sides() {
        let vel = Expr::Func { name: "r".to_string(), args: vec![Expr::var("tau")], order: vec![1] };
        let rhs = Expr::var("M") * vel;
        let text = render_geodesic_solved_component("t", &rhs, "tau", Target::Latex);
        assert!(text.starts_with("\\ddot{t} = "), "{text}");
        assert!(text.contains("\\dot{r}"), "{text}");
        assert!(!text.contains("tau"), "the parameter must be implicit in the dot form: {text}");
    }

    #[test]
    fn mathematica_and_sympy_reuse_the_generic_func_renderer_but_use_a_real_equation_not_assignment() {
        // Same point as `unicode_needs_no_special_casing_...` above for
        // the INNER `Expr::Func` node rendering (`mathematica_func`/
        // `sympy_func` produce correct prime/`Derivative` notation with
        // zero code added here) -- but unlike Unicode/LaTeX, the OUTER
        // `<lhs> = 0` shape itself needs a real substitution for these
        // two targets: `lhs` is a whole expression, never a bare
        // identifier, so literal `=` would be an invalid ASSIGNMENT in
        // the target language, not book notation. `Eq(...)`/`==` are
        // that substitution -- see `render_geodesic_component`'s own
        // doc comment for why (a bug caught by actually running export
        // output in real SymPy, `oderom-cli/tests/export.rs`).
        let acc = Expr::Func { name: "t".to_string(), args: vec![Expr::var("tau")], order: vec![2] };
        let vel = Expr::Func { name: "r".to_string(), args: vec![Expr::var("tau")], order: vec![1] };
        let lhs = acc.clone() + vel.clone();

        let math_text = render_geodesic_component(&lhs, "tau", Target::Mathematica);
        assert!(math_text.contains("t''[tau]"), "{math_text}");
        assert!(math_text.contains("r'[tau]"), "{math_text}");
        assert!(math_text.ends_with(" == 0"), "{math_text}");

        let sympy_text = render_geodesic_component(&lhs, "tau", Target::Sympy);
        assert!(sympy_text.contains("Derivative(Function('t')(tau), tau, 2)"), "{sympy_text}");
        assert!(sympy_text.contains("Derivative(Function('r')(tau), tau, 1)"), "{sympy_text}");
        assert!(sympy_text.starts_with("Eq("), "{sympy_text}");
        assert!(sympy_text.ends_with(", 0)"), "{sympy_text}");
    }

    #[test]
    fn solved_structured_names_each_equations_own_lhs_by_chart_position() {
        let chart = Chart::new(["t", "r"]);
        let rhs = vec![Expr::var("M"), Expr::var("M") * Expr::int(2)];
        let rendered = render_geodesic_solved_structured("accel", &chart, &rhs, "tau", Target::Unicode, 20);
        assert_eq!(rendered.components.len(), 2);
        assert!(rendered.components[0].formula.starts_with("t''(tau) = "), "{}", rendered.components[0].formula);
        assert!(rendered.components[1].formula.starts_with("r''(tau) = "), "{}", rendered.components[1].formula);
    }
}
