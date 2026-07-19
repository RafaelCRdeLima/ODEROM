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

use crate::grid::Grid;
use crate::tensor::{canonical_indices, ComponentTensor, IndexTuple, Orbit};
use oderom_core::{Registry, Render, Target};
use oderom_expr::{normalize, Expr};
use rustc_hash::FxHashMap;

/// One independent component: `representative` is the lexicographically
/// minimal index tuple in its symmetry orbit, `orbit_size` is how many
/// raw index tuples share it (1 with no symmetry, as for [`Grid`]), and
/// `value` is that component's (already normalized) value.
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

/// Renders `classes` under `label` (e.g. a tensor head's name, or
/// `"Gamma"` for Christoffel symbols): independent nonzero components
/// first (at most `max_lines` of them, the rest folded into an explicit
/// "... and N more" line), then a single line counting the
/// identically-zero independent components. `Target::Json` instead
/// produces one JSON object describing the same summary, since a
/// per-line text format isn't a machine-readable target.
pub fn render_classes(label: &str, classes: Vec<ComponentClass>, target: Target, max_lines: usize) -> String {
    let mut classes = classes;
    classes.sort_by(|a, b| a.representative.cmp(&b.representative));
    let (zero, nonzero): (Vec<_>, Vec<_>) = classes.into_iter().partition(|c| c.value.is_zero());
    let shown_count = nonzero.len().min(max_lines);
    let (shown, truncated) = (&nonzero[..shown_count], &nonzero[shown_count..]);

    match target {
        Target::Json => render_json(label, shown, truncated.len(), zero.len()),
        _ => render_text(label, shown, truncated.len(), zero.len(), target),
    }
}

fn render_text(label: &str, shown: &[ComponentClass], truncated: usize, zero_count: usize, target: Target) -> String {
    let mut lines: Vec<String> = shown.iter().map(|c| render_class_line(label, c, target)).collect();
    if truncated > 0 {
        lines.push(format!("... and {truncated} more independent component{} (truncated)", plural(truncated)));
    }
    if zero_count > 0 {
        lines.push(format!("{zero_count} independent component{} identically zero", plural(zero_count)));
    }
    if lines.is_empty() {
        lines.push(format!("{label}: no independent components (identically zero)"));
    }
    lines.join("\n")
}

fn render_class_line(label: &str, class: &ComponentClass, target: Target) -> String {
    let idx = class.representative.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
    let value = class.value.render(target);
    let orbit_note =
        if class.orbit_size > 1 { format!("  ({} components by symmetry)", class.orbit_size) } else { String::new() };
    match target {
        Target::Latex => format!("{label}_{{{idx}}} = {value}{orbit_note}"),
        _ => format!("{label}[{idx}] = {value}{orbit_note}"),
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
    fn schwarzschild_riemann() -> (Registry, ComponentTensor, usize) {
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
        (registry, riemann, 4)
    }

    #[test]
    fn classify_tensor_groups_by_orbit_and_counts_size() {
        let (registry, riemann, dim) = schwarzschild_riemann();
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
        let (registry, riemann, dim) = schwarzschild_riemann();
        let classes = classify_tensor(&registry, &riemann, dim);
        let zero_count = classes.iter().filter(|c| c.value.is_zero()).count();
        let text = render_classes("R", classes, Target::Unicode, 100);
        assert!(text.contains(&format!("{zero_count} independent components identically zero")));
        assert!(text.contains("R[0,1,0,1] ="));
    }

    #[test]
    fn render_classes_truncates_explicitly() {
        let (registry, riemann, dim) = schwarzschild_riemann();
        let classes = classify_tensor(&registry, &riemann, dim);
        let nonzero_total = classes.iter().filter(|c| !c.value.is_zero()).count();
        assert!(nonzero_total >= 1);
        let text = render_classes("R", classes, Target::Unicode, 0);
        assert!(text.contains(&format!("... and {nonzero_total} more independent component")));
    }

    /// Golden string: tests the JSON renderer's shape, not any
    /// mathematical claim (see DESIGN-UI.md, adjustment 3).
    #[test]
    fn render_classes_json_is_one_object() {
        let (registry, riemann, dim) = schwarzschild_riemann();
        let classes = classify_tensor(&registry, &riemann, dim);
        let json = render_classes("R", classes, Target::Json, 100);
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
