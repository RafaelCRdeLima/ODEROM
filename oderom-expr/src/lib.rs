//! `oderom-expr` -- symbolic scalar expressions (Marco 2), plus variable
//! substitution ([`substitute`], Marco 3: expressing one chart's
//! components in another's coordinates via a transition map).
//!
//! A tensor's *component* in a chart (e.g. `g_tt = -(1 - 2M/r)`) is not a
//! rational number like `oderom_core::Scalar` -- it is an element of
//! `C^inf(U)`, a function of the chart's coordinates. [`Expr`] represents
//! such functions symbolically: rationals, coordinate variables, sums,
//! products, integer powers, `sin`, `cos`.
//!
//! This is deliberately *not* an e-graph (that is Marco 4's saturation
//! engine). [`normalize`] is a fixed point of ordinary bottom-up
//! rewriting: fold rational arithmetic and collect like terms/like bases
//! by structural equality after canonically ordering each `Add`/`Mul`'s
//! children -- see the `normalize` module docs for why it does *not*
//! distribute products over sums. That is enough to reduce the
//! closed-form rational functions that arise from Christoffel/Riemann/
//! Ricci computations to a single normal form -- which is all an
//! acceptance test that checks `Kretschmann == 48 M^2 / r^6` by
//! structural equality needs.

mod diff;
mod normalize;
mod render;
mod rationalize;
mod substitute;

pub use diff::diff;
pub use normalize::normalize;
pub use rationalize::rationalize;
pub use render::GREEK_LETTERS;
pub use substitute::substitute;

use oderom_core::Scalar;
use std::cmp::Ordering;

/// A symbolic scalar expression.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Expr {
    Rational(Scalar),
    /// A coordinate variable, named for readability (Marco 2 has one
    /// chart per manifold, so a bare name is unambiguous).
    Var(String),
    Add(Vec<Expr>),
    Mul(Vec<Expr>),
    /// Integer exponent: positive powers of a sum can be expanded into a
    /// finite polynomial; negative ones cannot (that would be an infinite
    /// series) and are left opaque, relying on matching-base cancellation
    /// in a surrounding `Mul` (see `normalize`).
    Pow(Box<Expr>, i32),
    Sin(Box<Expr>),
    Cos(Box<Expr>),
}

impl Expr {
    pub fn zero() -> Expr {
        Expr::Rational(Scalar::ZERO)
    }

    pub fn one() -> Expr {
        Expr::Rational(Scalar::ONE)
    }

    /// The integer `n` as an `Expr`.
    pub fn int(n: i64) -> Expr {
        Expr::Rational(Scalar::from_int(n))
    }

    /// The rational `num/den` as an `Expr`.
    pub fn rational(num: i64, den: i64) -> Expr {
        Expr::Rational(Scalar::new(num, den))
    }

    /// A coordinate variable named `name`.
    pub fn var(name: impl Into<String>) -> Expr {
        Expr::Var(name.into())
    }

    /// `self` raised to the integer power `exp`.
    pub fn pow(self, exp: i32) -> Expr {
        Expr::Pow(Box::new(self), exp)
    }

    pub fn sin(self) -> Expr {
        Expr::Sin(Box::new(self))
    }

    pub fn cos(self) -> Expr {
        Expr::Cos(Box::new(self))
    }

    /// Whether this is literally the rational zero (not whether it
    /// *simplifies* to zero -- call [`normalize`] first if it might not
    /// already be in normal form).
    pub fn is_zero(&self) -> bool {
        matches!(self, Expr::Rational(s) if s.is_zero())
    }

    /// Total number of nodes in the expression tree (every `Rational`,
    /// `Var`, and operator counts as one, plus its children). A cheap,
    /// purely structural size measure -- not a cost/complexity estimate,
    /// just "how big is this tree right now" -- used to diagnose and
    /// guard against expression blowup during symbolic computation (see
    /// DESIGN-M2.md's rational-normal-form note).
    pub fn node_count(&self) -> usize {
        1 + match self {
            Expr::Rational(_) | Expr::Var(_) => 0,
            Expr::Add(terms) | Expr::Mul(terms) => terms.iter().map(Expr::node_count).sum(),
            Expr::Pow(base, _) => base.node_count(),
            Expr::Sin(inner) | Expr::Cos(inner) => inner.node_count(),
        }
    }
}

/// Rank used only to order *different variants* against each other;
/// combined with per-variant field comparisons below into a full [`Ord`]
/// impl. This is not a mathematical ordering of the values `Expr`s
/// denote -- it exists solely so `normalize` can canonically sort
/// `Add`/`Mul` children, making two structurally-equal sums (e.g. `1 -
/// 2M/r` built in either term order) converge to the identical tree.
fn variant_rank(e: &Expr) -> u8 {
    match e {
        Expr::Rational(_) => 0,
        Expr::Var(_) => 1,
        Expr::Pow(_, _) => 2,
        Expr::Mul(_) => 3,
        Expr::Add(_) => 4,
        Expr::Sin(_) => 5,
        Expr::Cos(_) => 6,
    }
}

impl PartialOrd for Expr {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Expr {
    fn cmp(&self, other: &Self) -> Ordering {
        variant_rank(self).cmp(&variant_rank(other)).then_with(|| match (self, other) {
            (Expr::Rational(a), Expr::Rational(b)) => {
                (a.numerator(), a.denominator()).cmp(&(b.numerator(), b.denominator()))
            }
            (Expr::Var(a), Expr::Var(b)) => a.cmp(b),
            (Expr::Pow(a, ea), Expr::Pow(b, eb)) => a.cmp(b).then_with(|| ea.cmp(eb)),
            (Expr::Mul(a), Expr::Mul(b)) | (Expr::Add(a), Expr::Add(b)) => a.cmp(b),
            (Expr::Sin(a), Expr::Sin(b)) | (Expr::Cos(a), Expr::Cos(b)) => a.cmp(b),
            _ => Ordering::Equal,
        })
    }
}

impl std::ops::Add for Expr {
    type Output = Expr;
    fn add(self, rhs: Expr) -> Expr {
        Expr::Add(vec![self, rhs])
    }
}

impl std::ops::Sub for Expr {
    type Output = Expr;
    fn sub(self, rhs: Expr) -> Expr {
        Expr::Add(vec![self, Expr::Mul(vec![Expr::int(-1), rhs])])
    }
}

impl std::ops::Mul for Expr {
    type Output = Expr;
    fn mul(self, rhs: Expr) -> Expr {
        Expr::Mul(vec![self, rhs])
    }
}

impl std::ops::Neg for Expr {
    type Output = Expr;
    fn neg(self) -> Expr {
        Expr::Mul(vec![Expr::int(-1), self])
    }
}

impl std::ops::Div for Expr {
    type Output = Expr;
    fn div(self, rhs: Expr) -> Expr {
        Expr::Mul(vec![self, Expr::Pow(Box::new(rhs), -1)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_count_counts_every_node() {
        // (x + 1)^2: Pow -> Add -> [Var, Rational] = 4 nodes.
        let e = (Expr::var("x") + Expr::one()).pow(2);
        assert_eq!(e.node_count(), 4);
    }

    #[test]
    fn ord_makes_addition_order_irrelevant_to_sorted_form() {
        let m = Expr::var("M");
        let r = Expr::var("r");
        let mut a = vec![Expr::one(), (Expr::int(-2) * m.clone()) / r.clone()];
        let mut b = vec![(Expr::int(-2) * m) / r, Expr::one()];
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }
}
