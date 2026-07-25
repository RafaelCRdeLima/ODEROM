//! Symmetry-aware display of a tensor's components (DESIGN-UI.md, Camada
//! A, adjustment 2): showing a `Riemann` tensor is fundamentally an
//! *elision* problem, not a formatting one. A rank-4 head in dimension 4
//! has 256 raw index tuples; the point of this module is to collapse
//! that down to one line per independent component under the head's
//! declared symmetry group -- exactly the same [`oderom_core::Bsgs`]
//! orbit computation [`crate::tensor::ComponentTensor`] already uses to
//! decide what to *store* (`tensor::canonical_indices`) -- annotate each
//! line with how many raw components it stands for, suppress components
//! that are identically zero into a single count, and truncate the
//! output explicitly rather than ever printing all `dim^rank` lines.
//!
//! This lives here (next to `ComponentTensor`, which already depends on
//! `Bsgs` for the same reason) rather than in the CLI: which components
//! are independent is a property of the tensor's symmetry group, not of
//! how the result happens to get printed.

use crate::chart::Chart;
use crate::grid::Grid;
use crate::tensor::{canonical_indices, ComponentTensor, IndexTuple, Orbit};
use oderom_core::{Registry, Render, Target, Variance};
use oderom_expr::{latex_var, normalize, Expr};
use rustc_hash::FxHashMap;

/// The real variance of each of `christoffel`'s three slots (Gamma^a_bc)
/// -- always this, regardless of whether the connection came from a
/// metric or was declared directly: Christoffel symbols are never a
/// tensor, so there is no "which head declared this" to read a variance
/// off of the way [`classify_tensor`]'s results can (see that
/// function's own callers, which read real slot variances from the
/// `Registry`). One leading upper index, two lower -- the manual's own
/// grammar table (9.4) states this is the only variance Christoffel is
/// ever shown in.
pub const CHRISTOFFEL_VARIANCE: [Variance; 3] = [Variance::Contra, Variance::Co, Variance::Co];

/// Riemann's real variance when it comes from a bare `connection` (no
/// metric to lower the first index with) -- mixed, R^a_bcd, exactly the
/// form `riemann_mixed` computes and the manual's 9.5 table promises for
/// this source. Never used for the metric-sourced path, which lowers
/// the first index and so is fully covariant instead (that path reads
/// its variance from the internally declared head via the `Registry`,
/// same as any other [`classify_tensor`] result).
pub const RIEMANN_MIXED_VARIANCE: [Variance; 4] = [Variance::Contra, Variance::Co, Variance::Co, Variance::Co];

/// Riemann's real variance when it comes from a metric: fully covariant,
/// R_abcd -- the first index has already been lowered
/// (`lower_first_index`) by the time this is shown, so displaying
/// anything but four lower indices would misrepresent what was actually
/// computed. The internally declared head this path's components come
/// from (`declare_internal_head`, `oderom-cli/src/commands.rs`) always
/// uses `Variance::Co` for every slot for exactly this reason; this
/// constant just names that same fact for the render call site rather
/// than making `render_classes` reach into the `Registry` to rediscover
/// it.
pub const RIEMANN_COV_VARIANCE: [Variance; 4] = [Variance::Co, Variance::Co, Variance::Co, Variance::Co];

/// Ricci's real variance regardless of source: R_bd, always fully
/// covariant -- the contraction R^a_bad that produces it never leaves an
/// upper index (see the manual's 9.5 table, "Sim" for both metric and
/// connection sources with no variance distinction between them, unlike
/// `riemann`). Used for the `connection`-sourced path, where Ricci comes
/// back as a raw [`Grid`] with no head/`Registry` to read a variance
/// from; the metric-sourced path reads the same fact off its internally
/// declared head instead, for the same reason `riemann`'s metric path
/// does.
pub const RICCI_VARIANCE: [Variance; 2] = [Variance::Co, Variance::Co];

/// One independent component: `representative` is the lexicographically
/// minimal index tuple in its symmetry orbit, `orbit_size` is how many
/// raw index tuples share it (1 with no symmetry, as for [`Grid`]), and
/// `value` is that component's (already normalized) value.
#[derive(Clone)]
pub struct ComponentClass {
    pub representative: Vec<u8>,
    pub orbit_size: usize,
    pub value: Expr,
}

/// Every `dim`-radix tuple of length `rank`, in increasing lexicographic
/// order -- `dim^rank` of them, small enough to enumerate directly for
/// any chart dimension this project deals with.
fn each_index_tuple(dim: usize, rank: usize) -> impl Iterator<Item = IndexTuple> {
    let total = dim.checked_pow(rank as u32).unwrap_or(0);
    (0..total).map(move |mut n| {
        let mut tuple: IndexTuple = smallvec::smallvec![0; rank];
        for slot in tuple.iter_mut().rev() {
            *slot = (n % dim) as u8;
            n /= dim;
        }
        tuple
    })
}

/// Independent components of `tensor`, one [`ComponentClass`] per
/// symmetry orbit that isn't forced to zero by the group itself
/// (antisymmetric heads with a repeated index, etc. -- those aren't
/// independent degrees of freedom at all, so they're excluded rather
/// than shown as zero).
pub fn classify_tensor(registry: &Registry, tensor: &ComponentTensor, dim: usize) -> Vec<ComponentClass> {
    let head = registry.head(tensor.head());
    let bsgs = &head.symmetry;
    let mut orbit_size: FxHashMap<IndexTuple, usize> = FxHashMap::default();
    let mut order: Vec<IndexTuple> = Vec::new();
    for tuple in each_index_tuple(dim, head.arity()) {
        if let Orbit::Representative(rep, _sign) = canonical_indices(bsgs, &tuple) {
            if let Some(count) = orbit_size.get_mut(&rep) {
                *count += 1;
            } else {
                orbit_size.insert(rep.clone(), 1);
                order.push(rep);
            }
        }
    }
    order
        .into_iter()
        .map(|rep| {
            let value = normalize(&tensor.get(registry, &rep).expect("representative has the head's own arity"));
            let size = orbit_size[&rep];
            ComponentClass { representative: rep.to_vec(), orbit_size: size, value }
        })
        .collect()
}

/// Same idea for a raw [`Grid`] (Christoffel symbols and the like): no
/// symmetry group to exploit, so every nonzero raw component is its own
/// class of orbit size 1 -- only the zero-suppression and truncation
/// parts of the display logic still apply.
pub fn classify_grid(grid: &Grid) -> Vec<ComponentClass> {
    each_index_tuple(grid.dim(), grid.rank())
        .map(|idx| ComponentClass {
            representative: idx.to_vec(),
            orbit_size: 1,
            value: normalize(&grid.get(&idx)),
        })
        .collect()
}

/// One independent component, ready to show: `formula` is the clean
/// "name = value" equation in the requested target (real coordinate
/// names, no raw integers, no comma-joined subscript -- see
/// [`format_indices`]) and nothing else -- no orbit-size annotation
/// glued onto it, so it is exactly what a click-to-copy feature should
/// place on the clipboard, and exactly what a KaTeX-backed frontend
/// should feed to KaTeX as one call. `orbit_note` is a separate,
/// deliberately plain-English fact ("N components by symmetry") a
/// caller can show subordinated to `formula` however it likes (smaller
/// text, a trailing parenthetical, its own line) -- never appended into
/// `formula` itself, which is what let the previous single-string
/// design's orbit note get passed straight into KaTeX along with the
/// real math and come out mangled (italic, no spaces).
#[derive(Clone, Debug, PartialEq)]
pub struct RenderedComponent {
    pub formula: String,
    pub orbit_note: Option<String>,
}

/// The full rendered picture of a `christoffel`/`riemann`/`ricci`
/// result: the independent nonzero components shown (each already split
/// into formula + orbit note, see [`RenderedComponent`]), and a
/// separate list of plain-English summary lines (truncation count, zero
/// count) that were never math to begin with and so were never glued to
/// any formula.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderedClasses {
    pub label: String,
    pub components: Vec<RenderedComponent>,
    pub summary: Vec<String>,
}

impl RenderedClasses {
    /// The single joined string this module produced before components
    /// and summary were split apart -- kept as the CLI/REPL's own
    /// output shape (`oderom-cli::commands`, `oderom-repl`'s
    /// `println!("{}", result.unicode)`), which is plain text a
    /// terminal prints as-is, never fed through KaTeX, so gluing was
    /// never the bug there. The orbit note now gets its own indented
    /// line rather than sharing the formula's line, which is a real
    /// (if purely cosmetic) improvement for those targets too -- the
    /// manual's own `--target latex` example glued a plain-English
    /// parenthetical onto real LaTeX, exactly the mixing problem 1 is
    /// about, just without KaTeX around to visibly mangle it.
    pub fn to_text(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for c in &self.components {
            lines.push(c.formula.clone());
            if let Some(note) = &c.orbit_note {
                lines.push(format!("  ({note})"));
            }
        }
        lines.extend(self.summary.iter().cloned());
        if lines.is_empty() {
            lines.push(format!("{}: no independent components (identically zero)", self.label));
        }
        lines.join("\n")
    }
}

/// Renders `classes` under `label` (e.g. a tensor head's name, or
/// `"Gamma"` for Christoffel symbols): independent nonzero components
/// first (at most `max_lines` of them, the rest folded into an explicit
/// "... and N more" line), then a single line counting the
/// identically-zero independent components. `Target::Json` instead
/// produces one JSON object describing the same summary, since a
/// per-line text format isn't a machine-readable target -- `chart` and
/// `variance` (unused on that path) are accepted uniformly anyway so
/// callers don't need a special case for it.
///
/// `chart` names each raw integer index (`Chart::coord`); `variance` is
/// this result's real per-slot variance (contravariant slots shown as a
/// leading superscript group, covariant as a trailing subscript group,
/// see [`format_indices`]) -- always the variance of what was actually
/// computed, never adjusted for display (see this crate's
/// [`CHRISTOFFEL_VARIANCE`]/[`RIEMANN_MIXED_VARIANCE`]/[`RIEMANN_COV_VARIANCE`]/[`RICCI_VARIANCE`]
/// and `classify_tensor` callers that read a real declared head's
/// slots instead).
pub fn render_classes(label: &str, classes: Vec<ComponentClass>, chart: &Chart, variance: &[Variance], target: Target, max_lines: usize) -> String {
    match target {
        Target::Json => {
            let mut classes = classes;
            classes.sort_by(|a, b| a.representative.cmp(&b.representative));
            let (zero, nonzero): (Vec<_>, Vec<_>) = classes.into_iter().partition(|c| c.value.is_zero());
            let shown_count = nonzero.len().min(max_lines);
            let (shown, truncated) = (&nonzero[..shown_count], &nonzero[shown_count..]);
            render_json(label, shown, truncated.len(), zero.len())
        }
        _ => render_classes_structured(label, classes, chart, variance, target, max_lines).to_text(),
    }
}

/// Same computation as [`render_classes`], but stops short of joining
/// everything into one string -- the shape a KaTeX-backed frontend
/// needs (`oderom-session::run_query`'s `EntryResult`), since that
/// caller has to feed `RenderedComponent::formula` to KaTeX one call at
/// a time and show `orbit_note`/`summary` as plain text beside or below
/// it, never inside the same KaTeX call. Not meaningful for
/// `Target::Json` (that target already produces one self-describing
/// object via [`render_classes`] directly); callers that want JSON
/// should call that instead.
pub fn render_classes_structured(
    label: &str,
    classes: Vec<ComponentClass>,
    chart: &Chart,
    variance: &[Variance],
    target: Target,
    max_lines: usize,
) -> RenderedClasses {
    let mut classes = classes;
    classes.sort_by(|a, b| a.representative.cmp(&b.representative));
    let (zero, nonzero): (Vec<_>, Vec<_>) = classes.into_iter().partition(|c| c.value.is_zero());
    let shown_count = nonzero.len().min(max_lines);
    let (shown, truncated) = (&nonzero[..shown_count], &nonzero[shown_count..]);

    let components = shown.iter().map(|c| render_component(label, c, chart, variance, target)).collect();
    let mut summary = Vec::new();
    if !truncated.is_empty() {
        summary.push(format!("... and {} more independent component{} (truncated)", truncated.len(), plural(truncated.len())));
    }
    if !zero.is_empty() {
        summary.push(format!("{} independent component{} identically zero", zero.len(), plural(zero.len())));
    }
    RenderedClasses { label: label.to_string(), components, summary }
}

fn render_component(label: &str, class: &ComponentClass, chart: &Chart, variance: &[Variance], target: Target) -> RenderedComponent {
    let idx = format_indices(&class.representative, chart, variance, target);
    let value = class.value.render(target);
    // `label` ("Gamma", "R", "Ricci") is itself a name, same as a
    // coordinate index or an expression variable -- `latex_var` maps
    // `Gamma` to `\Gamma` under `Target::Latex` for exactly the same
    // reason `theta` maps to `\theta`. Without this, Christoffel's
    // label rendered as five separate italic letters (KaTeX's default
    // for any bare, backslash-less run of letters in math mode), not
    // the capital Greek letter it names.
    //
    // Mathematica/Sympy (Rodada Exportação) use the raw label
    // unchanged, same as every non-Latex target already did -- a
    // reserved-word collision on the label itself (e.g. `Gamma`, a real
    // Mathematica built-in) is the exporting CALLER's job to detect and
    // rename before this function is ever reached (`oderom_expr::export`),
    // not something this low-level renderer decides on its own; by the
    // time `label` gets here it is assumed already safe for `target`.
    let shown_label = if target == Target::Latex { latex_var(label) } else { label.to_string() };
    // A single flat, valid identifier per component under Mathematica/
    // Sympy (`R_t_r_t_r`/`R$t$r$t$r`, see `format_indices`' own doc
    // comment for the separator and variance-marking convention)
    // assigned via `=` -- "cole na ferramenta e funcione", not book
    // notation; every other target keeps the existing book-style form.
    let formula = format!("{shown_label}{idx} = {value}");
    let orbit_note = if class.orbit_size > 1 { Some(format!("{} components by symmetry", class.orbit_size)) } else { None };
    RenderedComponent { formula, orbit_note }
}

/// Book-style index notation: real coordinate names (via `Chart::coord`,
/// Greek ones spelled as their LaTeX macro under `Target::Latex` --
/// `oderom_expr::latex_var`, the same mapping expression variables
/// already use, so `theta` reads as `\theta` here exactly as it does
/// anywhere else in a rendered expression) grouped by `variance`:
/// contravariant slots first as a leading superscript group, covariant
/// slots as a trailing subscript group -- `R^{a}_{bcd}` for a (1,3)
/// result, `R_{trtr}` (no upper group at all) for a fully covariant one.
/// Grouping by variance rather than by original slot position is
/// standard tensor notation (a mixed tensor's upper indices are shown
/// together, then its lower indices together, regardless of which
/// literal argument position each came from) and, for every variance
/// pattern this project actually produces (a single leading
/// contravariant slot, or none at all), coincides exactly with
/// preserving original order anyway.
///
/// `Target::Latex` joins names with a plain space, not the empty
/// string -- `\theta` is a *control word*, which TeX/KaTeX parses as
/// "backslash, then every following letter", so `\theta` directly
/// followed by the letter `r` (no separator at all) parses as the
/// single, undefined macro `\thetar`, not `\theta` then `r` (a real bug
/// this project shipped once: `R^{\theta}_{\thetar}` came out as
/// literal red KaTeX error text). A space is the standard, always-safe
/// fix: math mode ignores whitespace for layout (an ordinary letter
/// followed by a space followed by another ordinary letter renders
/// with no visible gap, exactly as `tr` would), but a space still ends
/// a control word, so `\theta r` parses as `\theta` then `r` as two
/// separate tokens. Applied uniformly to every pair, not just
/// macro-then-letter ones, since it costs nothing for the ordinary
/// letter-then-letter case and does not need to special-case which
/// pairs are "safe" already.
///
/// `Target::Mathematica`/`Target::Sympy` (Rodada Exportação) instead
/// produce a flat suffix meant to be concatenated directly onto `label`
/// to form ONE valid identifier in the target language -- there is no
/// book notation there, only source code, so `R^{t}_{trr}` becomes
/// something like `R_up_t_t_r_r` (Sympy) or `R$up$t$t$r$r`
/// (Mathematica) instead. Sympy uses `_` as the separator (Python
/// identifiers already allow it); Mathematica uses `$` (Mathematica
/// identifiers allow `$` but reserve bare `_` for its own
/// pattern-`Blank` construct -- using `_` there would produce a
/// pattern, not a plain symbol). Each contravariant coordinate name is
/// prefixed with an `up` marker segment (covariant ones are left
/// unmarked, matching the "fully covariant is the unmarked default"
/// convention used throughout this module) so a mixed-variance
/// result's exported name still records which slots were upper --
/// lossy only in that it can't be mechanically parsed back apart from
/// a literal coordinate that happened to be named `up`, an acceptable
/// cost given the goal is "cole e funcione", not round-tripping.
///
/// Every remaining target (`Target::Unicode`, `Target::Json`) keeps
/// the pre-existing bracketed, comma-separated form (`[t,r,theta,phi]`)
/// with a leading `^` on any contravariant name (`[^a,b,c]`) -- plain
/// text has no control-word concept at all, so the comma is what
/// delimits there.
fn format_indices(representative: &[u8], chart: &Chart, variance: &[Variance], target: Target) -> String {
    let name = |i: usize| -> String {
        let raw = chart.coord(representative[i]);
        if target == Target::Latex {
            latex_var(raw)
        } else {
            raw.to_string()
        }
    };
    let (up, down): (Vec<usize>, Vec<usize>) = (0..representative.len()).partition(|&i| variance.get(i) == Some(&Variance::Contra));

    match target {
        Target::Latex => {
            let up_part = if up.is_empty() { String::new() } else { format!("^{{{}}}", up.iter().map(|&i| name(i)).collect::<Vec<_>>().join(" ")) };
            let down_part = if down.is_empty() { String::new() } else { format!("_{{{}}}", down.iter().map(|&i| name(i)).collect::<Vec<_>>().join(" ")) };
            format!("{up_part}{down_part}")
        }
        Target::Mathematica | Target::Sympy => {
            let sep = if target == Target::Sympy { "_" } else { "$" };
            let mut pieces: Vec<String> = up.iter().map(|&i| format!("up{sep}{}", name(i))).collect();
            pieces.extend(down.iter().map(|&i| name(i)));
            pieces.iter().map(|p| format!("{sep}{p}")).collect::<Vec<_>>().join("")
        }
        Target::Unicode | Target::Json => {
            let up_names: Vec<String> = up.iter().map(|&i| format!("^{}", name(i))).collect();
            let down_names: Vec<String> = down.iter().map(|&i| name(i)).collect();
            format!("[{}]", up_names.iter().cloned().chain(down_names).collect::<Vec<_>>().join(","))
        }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn render_json(label: &str, shown: &[ComponentClass], truncated: usize, zero_count: usize) -> String {
    let classes = shown
        .iter()
        .map(|c| {
            let idx = c.representative.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
            format!(
                r#"{{"indices":[{idx}],"orbit_size":{},"value":{}}}"#,
                c.orbit_size,
                c.value.render(Target::Json)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"label":{},"classes":[{classes}],"truncated":{truncated},"zero_count":{zero_count}}}"#,
        json_escape(label)
    )
}

fn json_escape(s: &str) -> String {
    format!("{:?}", s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::Chart;
    use crate::curvature::{christoffel, metric_inverse_diagonal};
    use oderom_core::{Perm, SignedPerm, SlotSig, Variance};

    /// Rebuilds the Schwarzschild fixture used by the Marco 2 acceptance
    /// test (see `oderom-components/tests/schwarzschild.rs`), just
    /// enough of it to exercise the renderer against a real, nontrivial
    /// symmetry group (Riemann's order-8 slot symmetry).
    fn schwarzschild_riemann() -> (Registry, ComponentTensor, usize, Chart) {
        let mut registry = Registry::new();
        let manifold = registry.declare_manifold("M", 4).unwrap();
        let tm = registry.declare_bundle("TM", manifold, 4).unwrap();
        let co = SlotSig { bundle: tm, variance: Variance::Co, dim: 4 };
        let metric_slots: smallvec::SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
        let metric_head =
            registry.declare_head("g", metric_slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();

        let chart = Chart::new(["t", "r", "theta", "phi"]);
        let m = Expr::var("M");
        let r = Expr::var("r");
        let f = Expr::one() - Expr::int(2) * m / r.clone();

        let mut g = ComponentTensor::new(metric_head);
        g.set(&registry, &[0, 0], normalize(&-f.clone())).unwrap();
        g.set(&registry, &[1, 1], normalize(&(Expr::one() / f))).unwrap();
        g.set(&registry, &[2, 2], normalize(&r.clone().pow(2))).unwrap();
        g.set(&registry, &[3, 3], normalize(&(r.pow(2) * Expr::var("theta").sin().pow(2)))).unwrap();

        let ginv = metric_inverse_diagonal(&registry, &chart, &g).unwrap();
        let gamma = christoffel(&registry, &chart, &g, &ginv).unwrap();

        // A minimal, deliberately sparse rank-4 head with Riemann's slot
        // symmetry, just to exercise `classify_tensor`/`render_classes`
        // -- not a claim about the actual Riemann tensor's components,
        // which the Marco 2 acceptance test already checks by structural
        // equality (Kretschmann), not by rendered strings.
        let riemann_slots: smallvec::SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co, co, co];
        let riemann_head = registry
            .declare_head(
                "R",
                riemann_slots,
                vec![
                    SignedPerm::new(Perm::transposition(4, 0, 1), -1),
                    SignedPerm::new(Perm::transposition(4, 2, 3), -1),
                    SignedPerm::new(Perm::try_from_images(&[2, 3, 0, 1]).unwrap(), 1),
                ],
            )
            .unwrap();
        let mut riemann = ComponentTensor::new(riemann_head);
        riemann.set(&registry, &[0, 1, 0, 1], gamma.get(&[1, 0, 0])).unwrap();
        (registry, riemann, 4, chart)
    }

    #[test]
    fn classify_tensor_groups_by_orbit_and_counts_size() {
        let (registry, riemann, dim, _chart) = schwarzschild_riemann();
        let classes = classify_tensor(&registry, &riemann, dim);
        // R[0,1,0,1] is nonzero and its orbit (under Riemann's order-8
        // symmetry, with indices 0 and 1 distinct) has more than one raw
        // member -- this is the actual content being tested: that
        // classification is orbit-based, not per-raw-tuple.
        let class = classes.iter().find(|c| !c.value.is_zero()).expect("one nonzero class");
        assert_eq!(class.representative, vec![0, 1, 0, 1]);
        assert!(class.orbit_size > 1, "orbit size should count more than the one raw tuple that was set");
    }

    #[test]
    fn render_classes_suppresses_zero_and_counts_them() {
        let (registry, riemann, dim, chart) = schwarzschild_riemann();
        let classes = classify_tensor(&registry, &riemann, dim);
        let zero_count = classes.iter().filter(|c| c.value.is_zero()).count();
        let text = render_classes("R", classes, &chart, &RIEMANN_COV_VARIANCE, Target::Unicode, 100);
        assert!(text.contains(&format!("{zero_count} independent components identically zero")));
        // Representative [0,1,0,1] in chart (t,r,theta,phi) -> t,r,t,r --
        // named, not raw integers (Problem 1's whole point).
        assert!(text.contains("R[t,r,t,r] ="));
    }

    #[test]
    fn render_classes_names_indices_and_keeps_the_orbit_note_off_the_formula_line() {
        let (registry, riemann, dim, chart) = schwarzschild_riemann();
        let classes = classify_tensor(&registry, &riemann, dim);
        let structured = render_classes_structured("R", classes, &chart, &RIEMANN_COV_VARIANCE, Target::Latex, 100);
        // Space-separated, not bare-concatenated -- see format_indices's
        // own doc comment: KaTeX/LaTeX ignore the space for layout, but
        // it is what keeps a Greek macro from swallowing the next plain
        // letter into its own control-word name (the real bug a user
        // caught: `\thetar` instead of `\theta` then `r`).
        let comp = structured.components.iter().find(|c| c.formula.starts_with("R_{t r t r} =")).expect("named, space-separated LaTeX subscript R_{t r t r}");
        assert!(!comp.formula.contains("components by symmetry"), "the orbit note must never be part of the formula string: {}", comp.formula);
        assert_eq!(comp.orbit_note.as_deref(), Some("4 components by symmetry"));
    }

    #[test]
    fn render_classes_latex_label_uses_the_greek_macro_not_five_bare_italic_letters() {
        let (registry, riemann, dim, chart) = schwarzschild_riemann();
        let classes = classify_tensor(&registry, &riemann, dim);
        // "Gamma" is Christoffel's real label (see CHRISTOFFEL_VARIANCE's
        // own doc comment) -- reused here against this fixture's data
        // purely to exercise the label-conversion mechanism, which does
        // not care what tensor the components came from.
        let structured = render_classes_structured("Gamma", classes, &chart, &RIEMANN_COV_VARIANCE, Target::Latex, 100);
        assert!(
            structured.components.iter().any(|c| c.formula.starts_with("\\Gamma_")),
            "the label itself must go through the same Greek-macro mapping as any other name -- got: {:?}",
            structured.components.iter().map(|c| &c.formula).collect::<Vec<_>>()
        );
    }

    #[test]
    fn render_classes_latex_separates_a_greek_index_from_a_directly_following_plain_letter_index() {
        let (_registry, _riemann, _dim, chart) = schwarzschild_riemann();
        // A hand-built component whose representative is exactly the
        // failure case: `theta` (chart index 2) immediately followed by
        // `r` (chart index 1), both covariant, both shown in the same
        // subscript group -- `\theta` then `r` with no separator
        // between them parses as the single, undefined macro `\thetar`.
        let class = ComponentClass { representative: vec![2, 1], orbit_size: 1, value: normalize(&Expr::one()) };
        let structured = render_classes_structured("R", vec![class], &chart, &[Variance::Co, Variance::Co], Target::Latex, 100);
        let formula = &structured.components[0].formula;
        assert!(formula.starts_with("R_{\\theta r} ="), "expected a space between \\theta and r, got: {formula}");
        assert!(!formula.contains("\\thetar"), "the undefined macro \\thetar must never appear: {formula}");
    }

    /// The residual case the previous test alone doesn't rule out: two
    /// Greek macros directly adjacent (`\theta\phi`, no plain letter
    /// between them at all). A backslash is not a letter, so TeX's
    /// control-word reader for `\theta` already stops at the `\` that
    /// starts `\phi` -- this pairing was never actually ambiguous, but
    /// asserted here explicitly rather than left as an inference from
    /// the letter-adjacency fix, per the standing request that this
    /// exact family never again depend on a human eye to catch a
    /// regression.
    #[test]
    fn render_classes_latex_separates_two_directly_adjacent_greek_macro_indices() {
        // A chart matching hyperbolic_plane.od's real fixture (chi, phi
        // -- both recognized Greek letters), so this isn't a synthetic
        // name pairing that could never occur in real output.
        let chart = Chart::new(["chi", "phi"]);
        let class = ComponentClass { representative: vec![0, 1], orbit_size: 1, value: normalize(&Expr::one()) };
        let structured = render_classes_structured("R", vec![class], &chart, &[Variance::Co, Variance::Co], Target::Latex, 100);
        let formula = &structured.components[0].formula;
        assert!(formula.starts_with("R_{\\chi \\phi} ="), "expected \\chi and \\phi as two separate macros, got: {formula}");
        assert!(!formula.contains("\\chiphi") && !formula.contains("\\chi\\phi"), "each macro must remain independently parseable: {formula}");
    }

    /// The other residual case: a Greek macro directly followed by a
    /// name that itself ends in a digit (`x1`) -- chart coordinate
    /// names can never be purely numeric (the lexer only starts an
    /// identifier on an alphabetic character, `oderom-cli/src/parser.rs`),
    /// so a bare digit can never immediately follow a macro from index
    /// concatenation alone; a letter-then-digit name like `x1` is the
    /// real, reachable shape this could take. TeX control words never
    /// absorb digits either way (`\theta` followed directly by `1`
    /// would already stop cleanly at the `1`), but this project spaces
    /// every pair unconditionally regardless -- asserted here rather
    /// than left as an inference from that rule.
    #[test]
    fn render_classes_latex_separates_a_greek_index_from_a_directly_following_digit_suffixed_name() {
        let chart = Chart::new(["theta", "x1"]);
        let class = ComponentClass { representative: vec![0, 1], orbit_size: 1, value: normalize(&Expr::one()) };
        let structured = render_classes_structured("R", vec![class], &chart, &[Variance::Co, Variance::Co], Target::Latex, 100);
        let formula = &structured.components[0].formula;
        assert!(formula.starts_with("R_{\\theta x1} ="), "expected a space between \\theta and x1, got: {formula}");
        assert!(!formula.contains("\\thetax1"), "the undefined macro \\thetax1 must never appear: {formula}");
    }

    #[test]
    fn render_classes_sympy_produces_one_flat_underscore_identifier_per_component() {
        let (_registry, _riemann, _dim, chart) = schwarzschild_riemann();
        let class = ComponentClass { representative: vec![0, 1, 0, 1], orbit_size: 1, value: normalize(&Expr::one()) };
        let structured = render_classes_structured("R", vec![class], &chart, &RIEMANN_COV_VARIANCE, Target::Sympy, 100);
        let formula = &structured.components[0].formula;
        assert!(formula.starts_with("R_t_r_t_r = "), "expected a flat, valid Python identifier, got: {formula}");
    }

    #[test]
    fn render_classes_mathematica_produces_one_flat_dollar_identifier_per_component() {
        let (_registry, _riemann, _dim, chart) = schwarzschild_riemann();
        let class = ComponentClass { representative: vec![0, 1, 0, 1], orbit_size: 1, value: normalize(&Expr::one()) };
        let structured = render_classes_structured("R", vec![class], &chart, &RIEMANN_COV_VARIANCE, Target::Mathematica, 100);
        let formula = &structured.components[0].formula;
        // `_` is Mathematica's own pattern-Blank operator, unsafe as a
        // plain separator -- `$` is the one this target uses instead
        // (see format_indices's own doc comment).
        assert!(formula.starts_with("R$t$r$t$r = "), "expected a flat, valid Mathematica symbol, got: {formula}");
        assert!(!formula.contains('_'), "must never use Mathematica's reserved pattern-Blank character: {formula}");
    }

    #[test]
    fn render_classes_sympy_and_mathematica_mark_contravariant_slots_with_an_up_segment() {
        let chart = Chart::new(["t", "r"]);
        let class = ComponentClass { representative: vec![0, 1], orbit_size: 1, value: normalize(&Expr::one()) };
        let variance = [Variance::Contra, Variance::Co];

        let sympy = render_classes_structured("R", vec![class.clone()], &chart, &variance, Target::Sympy, 100);
        assert!(sympy.components[0].formula.starts_with("R_up_t_r = "), "{}", sympy.components[0].formula);

        let math = render_classes_structured("R", vec![class], &chart, &variance, Target::Mathematica, 100);
        assert!(math.components[0].formula.starts_with("R$up$t$r = "), "{}", math.components[0].formula);
    }

    #[test]
    fn render_classes_truncates_explicitly() {
        let (registry, riemann, dim, chart) = schwarzschild_riemann();
        let classes = classify_tensor(&registry, &riemann, dim);
        let nonzero_total = classes.iter().filter(|c| !c.value.is_zero()).count();
        assert!(nonzero_total >= 1);
        let text = render_classes("R", classes, &chart, &RIEMANN_COV_VARIANCE, Target::Unicode, 0);
        assert!(text.contains(&format!("... and {nonzero_total} more independent component")));
    }

    /// Golden string: tests the JSON renderer's shape, not any
    /// mathematical claim (see DESIGN-UI.md, adjustment 3). JSON keeps
    /// raw integer indices unaffected by this round's typography work
    /// -- it is a machine-consumption format, not something a person
    /// reads or pastes into a paper, and nothing asked for it to change.
    #[test]
    fn render_classes_json_is_one_object() {
        let (registry, riemann, dim, chart) = schwarzschild_riemann();
        let classes = classify_tensor(&registry, &riemann, dim);
        let json = render_classes("R", classes, &chart, &RIEMANN_COV_VARIANCE, Target::Json, 100);
        assert!(json.starts_with(r#"{"label":"R","classes":["#));
        assert!(json.contains(r#""indices":[0,1,0,1]"#));
        assert!(json.contains(r#""truncated":0"#));
    }

    #[test]
    fn classify_grid_has_orbit_size_one() {
        let mut grid = Grid::new(2, 2);
        grid.set(&[0, 0], Expr::int(1));
        let classes = classify_grid(&grid);
        assert_eq!(classes.len(), 4); // dim^rank = 2^2, no symmetry to collapse
        assert!(classes.iter().all(|c| c.orbit_size == 1));
        assert_eq!(classes.iter().filter(|c| !c.value.is_zero()).count(), 1);
    }
}
