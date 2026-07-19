//! Multiple charts on a manifold, related by transition maps, and the
//! check this marco's acceptance criterion actually asks for: that a
//! tensor declared independently in two overlapping charts agrees once
//! pulled back through the transition between them.

use crate::chart::Chart;
use crate::error::ComponentError;
use crate::tensor::ComponentTensor;
use oderom_core::Registry;
use oderom_expr::{diff, normalize, rationalize, substitute, Expr};

/// An index into an [`Atlas`]'s charts. Meaningless outside the `Atlas`
/// that produced it (same convention as `oderom_core::Registry`'s ids).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChartId(u32);

/// A manifold's collection of charts.
#[derive(Clone, Debug, Default)]
pub struct Atlas {
    charts: Vec<Chart>,
}

impl Atlas {
    pub fn new() -> Self {
        Atlas::default()
    }

    /// Adds `chart` to the atlas, returning its id.
    pub fn add_chart(&mut self, chart: Chart) -> ChartId {
        let id = ChartId(self.charts.len() as u32);
        self.charts.push(chart);
        id
    }

    pub fn chart(&self, id: ChartId) -> &Chart {
        &self.charts[id.0 as usize]
    }
}

/// A coordinate change from `from` to `to`: `forward[i]` is `to`'s `i`-th
/// coordinate, expressed as a function of `from`'s coordinates. Valid
/// (only) on the overlap of the two charts' domains -- Marco 3 does not
/// check that overlap automatically (see DESIGN-M3.md, D3.1); it is the
/// caller's declared claim.
pub struct TransitionMap {
    pub from: ChartId,
    pub to: ChartId,
    pub forward: Vec<Expr>,
}

/// Checks that `g_from` (declared directly in `transition.from`'s
/// coordinates) equals the pullback of `g_to` (declared directly in
/// `transition.to`'s coordinates) through `transition`:
///
/// ```text
/// g_from[i,j](x) = sum_{k,l} g_to[k,l](phi(x)) * d(phi_k)/d(x_i) * d(phi_l)/d(x_j)
/// ```
///
/// where `phi = transition.forward`. Both tensors must already be
/// expressed in reduced (`normalize`d) form for their own chart; this
/// does not re-derive one from the other, only checks the two agree.
///
/// Compares `pulled` and `direct` by cross-multiplying their
/// [`rationalize`]d numerator/denominator pairs (`pulled == direct` iff
/// `pulled_num*direct_den == direct_num*pulled_den`) rather than
/// `normalize`-ing each on its own and comparing directly: a pullback
/// through a transition routinely multiplies together several
/// *independent* sums (the pulled-back metric's own conformal factor,
/// the transition's own Jacobian), which `normalize`'s local rewriting
/// cannot always reduce to a single canonical form on its own -- see
/// `oderom_expr::rationalize`'s module docs for why.
pub fn metric_agrees_across_transition(
    registry: &Registry,
    atlas: &Atlas,
    g_from: &ComponentTensor,
    g_to: &ComponentTensor,
    transition: &TransitionMap,
) -> Result<bool, ComponentError> {
    let from_chart = atlas.chart(transition.from);
    let to_chart = atlas.chart(transition.to);
    let n = from_chart.dim();

    for i in 0..n as u8 {
        for j in 0..n as u8 {
            let mut pulled = Expr::zero();
            for k in 0..n as u8 {
                for l in 0..n as u8 {
                    let mut g_to_kl = g_to.get(registry, &[k, l])?;
                    for m in 0..n as u8 {
                        g_to_kl = substitute(&g_to_kl, to_chart.coord(m), &transition.forward[m as usize]);
                    }
                    let d_k = diff(&transition.forward[k as usize], from_chart.coord(i));
                    let d_l = diff(&transition.forward[l as usize], from_chart.coord(j));
                    pulled = pulled + g_to_kl * d_k * d_l;
                }
            }
            let direct = g_from.get(registry, &[i, j])?;
            let (pulled_num, pulled_den) = rationalize(&pulled);
            let (direct_num, direct_den) = rationalize(&direct);
            let lhs = normalize(&(pulled_num * direct_den));
            let rhs = normalize(&(direct_num * pulled_den));
            if lhs != rhs {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
